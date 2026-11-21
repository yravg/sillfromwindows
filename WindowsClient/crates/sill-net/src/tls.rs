//! The device's TLS 1.3 client for Sill's two doors (port of Sources/StreamProtocol/RemoteTLS.swift,
//! docs/windows-client/protocol-spec.md section 3).
//!
//! * TLS 1.3 only, tickets and resumption off, one ALPN (`sill/1` session or `sill-pair/1`
//!   pairing), SNI always `sill`.
//! * Client auth: the device's certificate and key are offered whatever the CertificateRequest
//!   lists (the Mac checks only the key's SPKI hash, never names, dates or chain).
//! * Trust is the pin alone: the Mac's leaf SPKI SHA-256. With no pin (pairing) any P-256 key is
//!   accepted and its fingerprint is RECORDED for the caller. Names, dates, chains and extensions
//!   are never examined (the Mac's certificate is hand-made: v3, no extensions, year-9999 notAfter),
//!   but the TLS 1.3 CertificateVerify signature is always verified with the leaf's key.

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{ClientConfig, ResolvesClientCert, Resumption};
use rustls::crypto::{ring as ring_provider, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::sign::CertifiedKey;
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

/// A session: paired devices only.
pub const SESSION_ALPN: &str = "sill/1";
/// Pairing: one kind 19 and one kind 20, then close.
pub const PAIRING_ALPN: &str = "sill-pair/1";
/// The SNI every client sends (the host ignores it).
pub const SERVER_NAME: &str = "sill";

/// DER prefix of a P-256 SubjectPublicKeyInfo; the 65-byte uncompressed point follows.
const SPKI_P256_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

#[derive(Debug)]
pub enum TlsError {
    /// The device certificate or key could not be used.
    Identity(String),
    Rustls(rustls::Error),
}

impl std::fmt::Display for TlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TlsError::Identity(s) => write!(f, "bad device certificate or key: {s}"),
            TlsError::Rustls(e) => write!(f, "rustls: {e}"),
        }
    }
}

impl std::error::Error for TlsError {}

impl From<rustls::Error> for TlsError {
    fn from(e: rustls::Error) -> Self {
        TlsError::Rustls(e)
    }
}

/// What the handshake showed, filled in by the verifier and the client-certificate resolver while
/// rustls drives the handshake. Read it after (or after a failure of) the handshake.
#[derive(Debug, Clone, Default)]
pub struct Observed {
    /// SHA-256 of the Mac leaf certificate's SPKI (None until a certificate arrived, or when its key
    /// is not P-256).
    pub spki_sha256: Option<[u8; 32]>,
    /// Length of the Mac leaf certificate's DER.
    pub cert_der_len: Option<usize>,
    /// True when rustls' own (webpki) parser could not read the Mac's certificate and the
    /// CertificateVerify signature was verified with p256 over the SPKI instead (risk 1).
    pub used_p256_fallback: bool,
    /// Why webpki refused the certificate, when `used_p256_fallback`.
    pub webpki_error: Option<String>,
    /// Set when the server sent a CertificateRequest: the signature schemes it listed.
    pub certificate_request_schemes: Option<Vec<SignatureScheme>>,
    /// Number of certificate_authorities (root hint subjects) in that request.
    pub certificate_request_root_hints: Option<usize>,
    /// True once rustls asked for our certificate (a CertificateRequest arrived). It is sent only after
    /// the Mac passed verification, so this alone does not mean it reached the Mac.
    pub client_cert_resolved: bool,
}

impl Observed {
    pub fn spki_base64url(&self) -> Option<String> {
        use base64::Engine;
        self.spki_sha256
            .map(|fp| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(fp))
    }

    pub fn spki_hex(&self) -> Option<String> {
        self.spki_sha256
            .map(|fp| fp.iter().map(|b| format!("{b:02x}")).collect())
    }
}

/// Shared view of an [`Observed`] that the caller keeps beside the connection.
#[derive(Debug, Clone, Default)]
pub struct ObservedHandle(Arc<Mutex<Observed>>);

impl ObservedHandle {
    pub fn snapshot(&self) -> Observed {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn with(&self, f: impl FnOnce(&mut Observed)) {
        f(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

// --- minimal DER reading (just enough to find the SPKI of a certificate) ------------------------

/// Reads one TLV at `at`; returns (tag, content start, content end).
fn tlv(der: &[u8], at: usize) -> Option<(u8, usize, usize)> {
    let tag = *der.get(at)?;
    let n = *der.get(at + 1)?;
    let (len, start) = if n & 0x80 == 0 {
        (n as usize, at + 2)
    } else {
        let k = (n & 0x7f) as usize;
        if k == 0 || k > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..k {
            len = (len << 8) | *der.get(at + 2 + i)? as usize;
        }
        (len, at + 2 + k)
    };
    let end = start.checked_add(len)?;
    (end <= der.len()).then_some((tag, start, end))
}

/// The SubjectPublicKeyInfo TLV (header included) of a certificate: the field after the optional
/// `[0]` version, serial, signature, issuer, validity and subject.
pub fn spki_der(cert: &[u8]) -> Option<&[u8]> {
    let (_, cs, _) = tlv(cert, 0)?;
    let (_, ts, te) = tlv(cert, cs)?;
    let mut fields = Vec::new();
    let mut i = ts;
    while i < te {
        let (tag, _, end) = tlv(cert, i)?;
        fields.push((tag, i, end));
        i = end;
    }
    if fields.first()?.0 == 0xa0 {
        fields.remove(0);
    }
    let (_, start, end) = *fields.get(5)?;
    cert.get(start..end)
}

/// SHA-256 of the leaf's SPKI when it is a P-256 key; None for any other key (nothing matches).
pub fn spki_fingerprint(cert: &[u8]) -> Option<[u8; 32]> {
    let spki = spki_der(cert)?;
    (spki.len() == 91 && spki.starts_with(&SPKI_P256_PREFIX)).then(|| Sha256::digest(spki).into())
}

// --- the verifier --------------------------------------------------------------------------------

/// Pins the Mac's key (or records it when no pin is known) and verifies CertificateVerify.
#[derive(Debug)]
pub struct SillVerifier {
    pin: Option<[u8; 32]>,
    provider: Arc<CryptoProvider>,
    observed: ObservedHandle,
}

impl SillVerifier {
    pub fn new(
        pin: Option<[u8; 32]>,
        provider: Arc<CryptoProvider>,
        observed: ObservedHandle,
    ) -> Self {
        Self {
            pin,
            provider,
            observed,
        }
    }
}

fn app_failure() -> rustls::Error {
    rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
}

impl ServerCertVerifier for SillVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let der = end_entity.as_ref();
        let fp = spki_fingerprint(der);
        self.observed.with(|o| {
            o.cert_der_len = Some(der.len());
            o.spki_sha256 = fp;
        });
        // Anything but a P-256 key gives None, which no pin matches and which pairing refuses too.
        let Some(fp) = fp else {
            return Err(app_failure());
        };
        match self.pin {
            Some(pin) if pin != fp => Err(app_failure()),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        match verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        ) {
            Ok(v) => Ok(v),
            // webpki could not parse the hand-made certificate: verify the same signature with the
            // key we pinned, never skip it. A parse failure is the only case that falls back.
            Err(rustls::Error::InvalidCertificate(e))
                if !matches!(e, rustls::CertificateError::BadSignature) =>
            {
                self.observed.with(|o| {
                    o.used_p256_fallback = true;
                    o.webpki_error = Some(format!("{e:?}"));
                });
                verify_p256(message, cert.as_ref(), dss)
            }
            Err(e) => Err(e),
        }
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

/// ECDSA P-256 / SHA-256 CertificateVerify over `message` with the key in the leaf's SPKI.
fn verify_p256(
    message: &[u8],
    cert: &[u8],
    dss: &DigitallySignedStruct,
) -> Result<HandshakeSignatureValid, rustls::Error> {
    use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    use p256::pkcs8::DecodePublicKey;
    if dss.scheme != SignatureScheme::ECDSA_NISTP256_SHA256 {
        return Err(rustls::Error::PeerMisbehaved(
            rustls::PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme,
        ));
    }
    let spki = spki_der(cert).ok_or_else(app_failure)?;
    let key = VerifyingKey::from_public_key_der(spki).map_err(|_| app_failure())?;
    let sig = Signature::from_der(dss.signature())
        .map_err(|_| rustls::Error::InvalidCertificate(rustls::CertificateError::BadSignature))?;
    key.verify(message, &sig)
        .map_err(|_| rustls::Error::InvalidCertificate(rustls::CertificateError::BadSignature))?;
    Ok(HandshakeSignatureValid::assertion())
}

// --- client auth ---------------------------------------------------------------------------------

/// Offers the device certificate to any CertificateRequest, ignoring the issuer list, and records
/// what the request said.
#[derive(Debug)]
struct SillClientAuth {
    key: Arc<CertifiedKey>,
    observed: ObservedHandle,
}

impl ResolvesClientCert for SillClientAuth {
    fn resolve(
        &self,
        root_hint_subjects: &[&[u8]],
        sigschemes: &[SignatureScheme],
    ) -> Option<Arc<CertifiedKey>> {
        self.observed.with(|o| {
            o.certificate_request_schemes = Some(sigschemes.to_vec());
            o.certificate_request_root_hints = Some(root_hint_subjects.len());
            o.client_cert_resolved = true;
        });
        Some(self.key.clone())
    }

    fn has_certs(&self) -> bool {
        true
    }
}

/// Builds the rustls client configuration.
///
/// `alpn` is [`SESSION_ALPN`] or [`PAIRING_ALPN`]. `cert_der` and `key_pkcs8_der` are the device's
/// P-256 certificate and key. `pin` is the Mac's SPKI SHA-256, or None to accept any P-256 key and
/// only record it (pairing). Returns the config and a handle to what the handshake observed.
pub fn client_config(
    alpn: &str,
    cert_der: Vec<u8>,
    key_pkcs8_der: Vec<u8>,
    pin: Option<[u8; 32]>,
) -> Result<(Arc<ClientConfig>, ObservedHandle), TlsError> {
    let provider = Arc::new(ring_provider::default_provider());
    let observed = ObservedHandle::default();

    let key =
        PrivateKeyDer::try_from(key_pkcs8_der).map_err(|e| TlsError::Identity(e.to_string()))?;
    let signing =
        ring_provider::sign::any_ecdsa_type(&key).map_err(|e| TlsError::Identity(e.to_string()))?;
    let certified = Arc::new(CertifiedKey::new(
        vec![CertificateDer::from(cert_der)],
        signing,
    ));

    let verifier = Arc::new(SillVerifier::new(pin, provider.clone(), observed.clone()));
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_cert_resolver(Arc::new(SillClientAuth {
            key: certified,
            observed: observed.clone(),
        }));
    config.alpn_protocols = vec![alpn.as_bytes().to_vec()];
    config.resumption = Resumption::disabled();
    config.enable_sni = true;
    Ok((Arc::new(config), observed))
}

/// The server name every connection sends, whatever address was dialled.
pub fn server_name() -> ServerName<'static> {
    ServerName::try_from(SERVER_NAME).expect("\"sill\" is a valid DNS name")
}
