//! A device identity: one P-256 key, a self-signed certificate made from it, the SPKI SHA-256
//! fingerprint every pin compares, and the Mac ID.
//!
//! Ports `Base64URL`, `SPKI`, `MacID`, `SelfSignedCertificate`, `RemoteIdentity` and `RemoteKey`
//! (Sources/StreamProtocol/RemoteIdentity.swift) and the device's key rules
//! (iOSClient/DeviceIdentity.swift, `DeviceKeyPolicy` in SavedMacs.swift). Trust is the key itself:
//! the Mac pins the SHA-256 of the SubjectPublicKeyInfo, so the key (not the certificate) is the
//! identity; the certificate is rebuilt at every launch (RemoteIdentity.swift:155-160).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use p256::elliptic_curve::rand_core::{OsRng, RngCore};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use p256::SecretKey;
use sha2::{Digest, Sha256};
use std::sync::Mutex;

// ---------------------------------------------------------------------------------------------
// Base64URL (RemoteIdentity.swift:11-33)

/// base64url without padding (RFC 4648 section 5): every byte string in the pairing link, kind 19
/// and kind 20 (RemoteIdentity.swift:12-17).
pub fn base64url_encode(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

/// None for anything that is not canonical unpadded base64url: padding, the standard alphabet's
/// `+` and `/`, whitespace, a length no encoding produces, non-zero trailing bits
/// (RemoteIdentity.swift:19-33; Swift re-encodes and compares, so does this).
pub fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    if !text
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || text.len() % 4 == 1
    {
        return None;
    }
    let d = URL_SAFE_NO_PAD.decode(text).ok()?;
    (base64url_encode(&d) == text).then_some(d)
}

// ---------------------------------------------------------------------------------------------
// SPKI (RemoteIdentity.swift:35-71)

/// DER prefix of a P-256 SubjectPublicKeyInfo up to the 65-byte point (Scripts/sillclient.py
/// `SPKI_PREFIX`): `SEQ{ SEQ{ OID ecPublicKey, OID prime256v1 }, BIT STRING 0x00 || point }`.
pub const SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// The SPKI DER (91 bytes) for a 65-byte uncompressed X9.63 point `04 || X || Y`
/// (RemoteIdentity.swift:39-41). None unless `point` is 65 bytes starting with 0x04.
pub fn spki_der(point: &[u8]) -> Option<Vec<u8>> {
    if point.len() != 65 || point[0] != 0x04 {
        return None;
    }
    let mut v = SPKI_PREFIX.to_vec();
    v.extend_from_slice(point);
    Some(v)
}

/// SHA-256 of the SPKI DER: the 32-byte fingerprint every pin compares; 43 characters as base64url
/// (RemoteIdentity.swift:44-46). None for a malformed point.
pub fn spki_fingerprint(point: &[u8]) -> Option<[u8; 32]> {
    spki_der(point).map(|d| Sha256::digest(d).into())
}

/// The fingerprint of an X.509 certificate's key when it is P-256, else None. Nothing else about
/// the certificate is looked at: names, dates and chains mean nothing to a pin
/// (RemoteIdentity.swift:62-64; the walk is Scripts/sillclient.py `spki`).
pub fn certificate_fingerprint(cert_der: &[u8]) -> Option<[u8; 32]> {
    let spki = certificate_spki(cert_der)?;
    // Only a P-256 key counts: exactly the 91-byte form with the prefix above.
    if spki.len() == 91 && spki[..26] == SPKI_PREFIX && spki[26] == 0x04 {
        Some(Sha256::digest(spki).into())
    } else {
        None
    }
}

/// The SubjectPublicKeyInfo TLV of a certificate: the 6th field of TBSCertificate after the optional
/// `[0]` version (sillclient.py `spki`). Hand-walked DER; no X.509 parser needed.
pub fn certificate_spki(der: &[u8]) -> Option<&[u8]> {
    let (_, cs, ce) = tlv(der, 0)?;
    let (_, ts, te) = tlv(der, cs)?;
    if te > ce {
        return None;
    }
    let mut fields = Vec::new();
    let mut i = ts;
    while i < te {
        let (tag, _, end) = tlv(der, i)?;
        fields.push((tag, i, end));
        i = end;
    }
    if fields.first()?.0 == 0xA0 {
        fields.remove(0);
    }
    let (_, start, end) = *fields.get(5)?;
    der.get(start..end)
}

/// (tag, content start, content end) of the TLV at `i`.
fn tlv(b: &[u8], i: usize) -> Option<(u8, usize, usize)> {
    let tag = *b.get(i)?;
    let n = *b.get(i + 1)?;
    let mut i = i + 2;
    let len = if n & 0x80 != 0 {
        let k = (n & 0x7f) as usize;
        if k == 0 || k > 4 {
            return None;
        }
        let mut v = 0usize;
        for x in b.get(i..i + k)? {
            v = (v << 8) | *x as usize;
        }
        i += k;
        v
    } else {
        n as usize
    };
    let end = i.checked_add(len)?;
    (end <= b.len()).then_some((tag, i, end))
}

// ---------------------------------------------------------------------------------------------
// Mac ID (RemoteIdentity.swift:73-100)

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Crockford base32 of the fingerprint's first 10 bytes (80 bits, most significant first):
/// 16 characters (RemoteIdentity.swift:81-97). Empty for fewer than 10 bytes, as Swift.
pub fn mac_id(fingerprint: &[u8]) -> String {
    if fingerprint.len() < 10 {
        return String::new();
    }
    let mut out = String::with_capacity(16);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &b in &fingerprint[..10] {
        buffer = (buffer << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(CROCKFORD[((buffer >> bits) & 31) as usize] as char);
        }
        buffer &= (1 << bits) - 1;
    }
    out
}

/// Whether `text` is 16 characters of the Crockford alphabet, upper case (RemoteIdentity.swift:100).
pub fn mac_id_is_well_formed(text: &str) -> bool {
    text.len() == 16 && text.bytes().all(|b| CROCKFORD.contains(&b))
}

/// "5KD2Q7": the first six characters of the Mac ID form of a fingerprint, for log lines
/// (RemoteIdentity.swift:152).
pub fn short_name(fingerprint: &[u8]) -> String {
    mac_id(fingerprint).chars().take(6).collect()
}

// ---------------------------------------------------------------------------------------------
// The identity

/// Why an identity could not be made or restored.
#[derive(Debug)]
pub enum IdentityError {
    /// The bytes are not a P-256 PKCS#8 private key.
    BadKey(String),
    /// The certificate could not be built.
    Certificate(String),
    /// The key store failed.
    Store(String),
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::BadKey(s) => write!(f, "not a P-256 key: {s}"),
            IdentityError::Certificate(s) => write!(f, "certificate: {s}"),
            IdentityError::Store(s) => write!(f, "key store: {s}"),
        }
    }
}
impl std::error::Error for IdentityError {}

/// This device's identity: a P-256 key. The pin the Mac stores is `fingerprint()`.
#[derive(Clone)]
pub struct DeviceIdentity {
    key: SecretKey,
}

impl std::fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never prints the key.
        write!(f, "DeviceIdentity({})", short_name(&self.fingerprint()))
    }
}

impl DeviceIdentity {
    /// A fresh key from the OS random source (RemoteKey.generate, RemoteIdentity.swift:166-170).
    pub fn generate() -> DeviceIdentity {
        DeviceIdentity {
            key: SecretKey::random(&mut OsRng),
        }
    }

    /// Restores a key from PKCS#8 DER (the form `to_pkcs8_der` writes; also what OpenSSL's
    /// `genpkey`/`pkcs8` produce for P-256).
    pub fn from_pkcs8_der(der: &[u8]) -> Result<DeviceIdentity, IdentityError> {
        SecretKey::from_pkcs8_der(der)
            .map(|key| DeviceIdentity { key })
            .map_err(|e| IdentityError::BadKey(e.to_string()))
    }

    /// The private key as PKCS#8 DER: what a `KeyStore` keeps (the Windows build wraps it with DPAPI).
    pub fn to_pkcs8_der(&self) -> Vec<u8> {
        self.key
            .to_pkcs8_der()
            .expect("a P-256 secret key always encodes")
            .as_bytes()
            .to_vec()
    }

    /// The private key as X9.63 `04 || X || Y || D`, 97 bytes (RemoteKey.export, the Swift TEST ONLY
    /// file store, RemoteIdentity.swift:173-177).
    pub fn export_x963(&self) -> Vec<u8> {
        let mut v = self.public_point().to_vec();
        v.extend_from_slice(&self.key.to_bytes());
        v
    }

    /// A key back from `export_x963` (RemoteKey.importPrivate, RemoteIdentity.swift:180-188). Checks
    /// that the point half matches the scalar.
    pub fn import_x963(data: &[u8]) -> Result<DeviceIdentity, IdentityError> {
        if data.len() != 97 {
            return Err(IdentityError::BadKey("97 bytes expected".into()));
        }
        let key =
            SecretKey::from_slice(&data[65..]).map_err(|e| IdentityError::BadKey(e.to_string()))?;
        let id = DeviceIdentity { key };
        if id.public_point()[..] != data[..65] {
            return Err(IdentityError::BadKey(
                "public point does not match the scalar".into(),
            ));
        }
        Ok(id)
    }

    /// The public key's 65-byte X9.63 point `04 || X || Y` (RemoteKey.publicPoint,
    /// RemoteIdentity.swift:191-195).
    pub fn public_point(&self) -> [u8; 65] {
        let ep = self.key.public_key().to_encoded_point(false);
        let mut out = [0u8; 65];
        out.copy_from_slice(ep.as_bytes());
        out
    }

    /// SHA-256 of the SPKI: this identity's pin (RemoteIdentity.swift:44-46).
    pub fn fingerprint(&self) -> [u8; 32] {
        spki_fingerprint(&self.public_point()).expect("a P-256 point is 65 bytes")
    }

    /// The Mac ID form of the fingerprint (RemoteIdentity.swift:81).
    pub fn mac_id(&self) -> String {
        mac_id(&self.fingerprint())
    }

    /// A self-signed P-256/ECDSA-SHA256 X.509 certificate for this key, DER.
    ///
    /// The Swift side hand-builds a no-extension v3 certificate (RemoteIdentity.swift:106-141); the
    /// Mac checks nothing of it but the key (docs/windows-client/protocol-spec.md section 3.4, and
    /// sillclient.py's plain `openssl req -x509` cert is accepted by the real host), so this uses
    /// rcgen's: issuer = subject = CN of 16 random hex characters (names nothing, as Swift), a random
    /// positive 16-byte serial, valid 2026-01-01 to 9999-12-31. Rebuilt at every launch; the pin is the
    /// key. Fails only if rcgen cannot take the key.
    pub fn certificate_der(&self) -> Result<Vec<u8>, IdentityError> {
        use rcgen::{
            date_time_ymd, CertificateParams, DistinguishedName, DnType, KeyPair, SerialNumber,
        };
        let pkcs8 = self.to_pkcs8_der();
        let key_pair = KeyPair::try_from(pkcs8.as_slice())
            .map_err(|e| IdentityError::Certificate(e.to_string()))?;
        let mut cn = [0u8; 8];
        let mut serial = [0u8; 16];
        OsRng.fill_bytes(&mut cn);
        OsRng.fill_bytes(&mut serial);
        serial[0] = (serial[0] & 0x7f) | 0x01; // positive, no leading zero byte to trim (Swift :113)
        let hex: String = cn.iter().map(|b| format!("{b:02x}")).collect();
        let mut params = CertificateParams::new(Vec::<String>::new())
            .map_err(|e| IdentityError::Certificate(e.to_string()))?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, hex);
        params.distinguished_name = dn;
        params.serial_number = Some(SerialNumber::from_slice(&serial));
        params.not_before = date_time_ymd(2026, 1, 1);
        params.not_after = date_time_ymd(9999, 12, 31);
        let cert = params
            .self_signed(&key_pair)
            .map_err(|e| IdentityError::Certificate(e.to_string()))?;
        Ok(cert.der().to_vec())
    }

    /// Signs `message` with ECDSA P-256/SHA-256, DER signature (the host's kind-18 signature form,
    /// RemoteIdentity.swift's counterpart in Remote.swift:84-91). Only tests and host-side tools need
    /// this; the client never signs a message with its identity key except inside TLS.
    pub fn sign_der(&self, message: &[u8]) -> Vec<u8> {
        use p256::ecdsa::{signature::Signer, Signature, SigningKey};
        let sk = SigningKey::from(&self.key);
        let sig: Signature = sk.sign(message);
        sig.to_der().as_bytes().to_vec()
    }
}

// ---------------------------------------------------------------------------------------------
// Storage

/// Where the device key lives between runs. The Windows build wraps the PKCS#8 bytes with DPAPI
/// (later, Windows-only); tests and the CLI use `MemoryKeyStore`. The device key rules of
/// DeviceKeyPolicy (SavedMacs.swift): a key is made only when the store says there is none, and
/// only a key the store gives back is used (`load_or_create`).
pub trait KeyStore {
    /// The stored PKCS#8 bytes, or None when nothing is stored. An Err is "could not read" and is
    /// never taken for "none": a failed read must not make a second key.
    fn load(&self) -> Result<Option<Vec<u8>>, IdentityError>;
    /// Stores `pkcs8` (replacing nothing: callers only save when `load` said None).
    fn save(&self, pkcs8: &[u8]) -> Result<(), IdentityError>;
    /// Removes the key (forget / start over).
    fn delete(&self) -> Result<(), IdentityError>;
}

/// An in-memory store (the CLI's throwaway identity, tests).
#[derive(Default)]
pub struct MemoryKeyStore {
    slot: Mutex<Option<Vec<u8>>>,
}

impl MemoryKeyStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl KeyStore for MemoryKeyStore {
    fn load(&self) -> Result<Option<Vec<u8>>, IdentityError> {
        Ok(self
            .slot
            .lock()
            .map_err(|e| IdentityError::Store(e.to_string()))?
            .clone())
    }
    fn save(&self, pkcs8: &[u8]) -> Result<(), IdentityError> {
        *self
            .slot
            .lock()
            .map_err(|e| IdentityError::Store(e.to_string()))? = Some(pkcs8.to_vec());
        Ok(())
    }
    fn delete(&self) -> Result<(), IdentityError> {
        *self
            .slot
            .lock()
            .map_err(|e| IdentityError::Store(e.to_string()))? = None;
        Ok(())
    }
}

/// The stored identity, or a new one saved and read back. A stored value that is not a key is an
/// error (never silently replaced: that would pair this device afresh as a new device on the Mac);
/// a failed read is an error and makes no key. A made key is used only once the store gives back the
/// very same bytes (DeviceKeyPolicy, SavedMacs.swift: "a key made used only once the Keychain gives
/// back this very key").
pub fn load_or_create(store: &dyn KeyStore) -> Result<DeviceIdentity, IdentityError> {
    if let Some(bytes) = store.load()? {
        return DeviceIdentity::from_pkcs8_der(&bytes);
    }
    let fresh = DeviceIdentity::generate();
    let der = fresh.to_pkcs8_der();
    store.save(&der)?;
    match store.load()? {
        Some(back) if back == der => Ok(fresh),
        _ => {
            let _ = store.delete();
            Err(IdentityError::Store(
                "the store did not keep the key".into(),
            ))
        }
    }
}
