//! Identity, SPKI fingerprint, Mac ID, certificate, key storage. Vectors: Tests/checks/protocol
//! ("Certificate, fingerprint, Mac ID", "Base64URL"), the plan, and Python/OpenSSL
//! (tests/fixtures/python_vectors.json from gen_fixtures.py).
use sha2::{Digest, Sha256};
use sill_net::identity::*;

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}
fn fixtures() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/python_vectors.json")).unwrap()
}

// ---- Mac ID (Tests/checks/protocol:199-203)

#[test]
fn mac_id_vectors() {
    let mut fp = unhex("50d858e0985ecc7f6041");
    fp.extend_from_slice(&[0u8; 22]);
    assert_eq!(mac_id(&fp), "A3C5HR4RBV67YR21");
    assert_eq!(mac_id(&Sha256::digest(b"example")), "A3C5HR4RBV67YR21");
    assert_eq!(mac_id(&[0u8; 32]), "0000000000000000");
    assert_eq!(mac_id(&[0xFFu8; 32]), "ZZZZZZZZZZZZZZZZ");
    assert_eq!(mac_id(&[1, 2, 3]), ""); // fewer than 10 bytes, as Swift
    assert_eq!(short_name(&Sha256::digest(b"example")), "A3C5HR");
}

#[test]
fn mac_id_hand_computed() {
    // Ten bytes 00..00 01: the 80-bit value 1 gives 15 zeros then '1'.
    let mut fp = [0u8; 32];
    fp[9] = 1;
    assert_eq!(mac_id(&fp), "0000000000000001");
    // 0xF8 then zeros: the first 5 bits 11111 = 31 = 'Z'.
    fp = [0u8; 32];
    fp[0] = 0xF8;
    assert_eq!(mac_id(&fp), "Z000000000000000");
}

#[test]
fn mac_id_well_formed() {
    assert!(mac_id_is_well_formed("A3C5HR4RBV67YR21"));
    assert!(!mac_id_is_well_formed("A3C5HR4RBV67YR2I")); // I is not Crockford
    assert!(!mac_id_is_well_formed("a3c5hr4rbv67yr21")); // upper case only
    assert!(!mac_id_is_well_formed("A3C5HR4RBV67YR2")); // 15
    assert!(!mac_id_is_well_formed("A3C5HR4RBV67YR211"));
}

#[test]
fn mac_id_matches_sillclient_py() {
    for c in fixtures()["macid"].as_array().unwrap() {
        assert_eq!(
            mac_id(&unhex(c["fp"].as_str().unwrap())),
            c["macid"].as_str().unwrap()
        );
    }
}

// ---- base64url (Tests/checks/protocol:226-235)

#[test]
fn base64url_lengths_and_round_trip() {
    assert_eq!(base64url_encode(&[0u8; 32]).len(), 43);
    assert_eq!(base64url_encode(&[0u8; 16]).len(), 22);
    for n in 0..64usize {
        let b: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
        assert_eq!(
            base64url_decode(&base64url_encode(&b)).as_deref(),
            Some(&b[..])
        );
    }
    assert_eq!(base64url_encode(&[0xfb, 0xff]), "-_8");
}

#[test]
fn base64url_refuses_non_canonical() {
    assert_eq!(base64url_decode("AAAA="), None); // padding
    assert_eq!(base64url_decode("ab+c"), None); // standard alphabet
    assert_eq!(base64url_decode("ab/c"), None);
    assert_eq!(base64url_decode("abcde"), None); // length 1 mod 4
    assert_eq!(base64url_decode("AB"), None); // non-zero trailing bits
    assert_eq!(base64url_decode("AA AA"), None); // whitespace
    assert_eq!(base64url_decode(""), Some(vec![]));
}

// ---- SPKI

#[test]
fn spki_is_91_bytes_with_the_documented_prefix() {
    let id = DeviceIdentity::generate();
    let p = id.public_point();
    assert_eq!(p[0], 0x04);
    let der = spki_der(&p).unwrap();
    assert_eq!(der.len(), 91);
    assert_eq!(&der[..26], &SPKI_PREFIX);
    assert_eq!(&der[26..], &p);
    assert_eq!(
        spki_fingerprint(&p).unwrap().as_slice(),
        &Sha256::digest(&der)[..]
    );
    assert_eq!(id.fingerprint(), spki_fingerprint(&p).unwrap());
    assert_eq!(base64url_encode(&id.fingerprint()).len(), 43);
}

#[test]
fn spki_refuses_malformed_points() {
    assert!(spki_der(&[0u8; 64]).is_none());
    assert!(spki_der(&[0u8; 65]).is_none()); // no 0x04 tag
    assert!(spki_fingerprint(&[4u8; 66]).is_none());
}

// ---- Rust == OpenSSL/Python

#[test]
fn openssl_keys_fingerprints_and_certs_match_python() {
    let f = fixtures();
    for i in f["identities"].as_array().unwrap() {
        let id = DeviceIdentity::from_pkcs8_der(&unhex(i["pkcs8"].as_str().unwrap())).unwrap();
        assert_eq!(
            id.public_point().to_vec(),
            unhex(i["point"].as_str().unwrap())
        );
        assert_eq!(id.fingerprint().to_vec(), unhex(i["fp"].as_str().unwrap()));
        assert_eq!(id.mac_id(), i["macid"].as_str().unwrap());
        // The certificate openssl made for that key: its SPKI hash is the pin (sillclient.spki).
        let cert = unhex(i["cert_der"].as_str().unwrap());
        assert_eq!(
            certificate_fingerprint(&cert).unwrap().to_vec(),
            unhex(i["fp"].as_str().unwrap())
        );
    }
}

// ---- keys

#[test]
fn pkcs8_and_x963_round_trip_keep_the_fingerprint() {
    let id = DeviceIdentity::generate();
    let back = DeviceIdentity::from_pkcs8_der(&id.to_pkcs8_der()).unwrap();
    assert_eq!(back.fingerprint(), id.fingerprint());
    assert_eq!(back.public_point(), id.public_point());
    let x = id.export_x963();
    assert_eq!(x.len(), 97);
    assert_eq!(x[0], 0x04);
    assert_eq!(
        DeviceIdentity::import_x963(&x).unwrap().fingerprint(),
        id.fingerprint()
    );
}

#[test]
fn bad_keys_are_refused() {
    assert!(DeviceIdentity::from_pkcs8_der(&[1, 2, 3]).is_err());
    assert!(DeviceIdentity::import_x963(&[0u8; 96]).is_err());
    let id = DeviceIdentity::generate();
    let mut x = id.export_x963();
    x[10] ^= 1; // the point no longer matches the scalar
    assert!(DeviceIdentity::import_x963(&x).is_err());
    let mut zero = vec![4u8; 97];
    zero[65..].fill(0); // scalar 0 is not a key
    assert!(DeviceIdentity::import_x963(&zero).is_err());
}

#[test]
fn two_generated_keys_differ() {
    assert_ne!(
        DeviceIdentity::generate().fingerprint(),
        DeviceIdentity::generate().fingerprint()
    );
}

#[test]
fn debug_never_prints_the_key() {
    let id = DeviceIdentity::generate();
    let shown = format!("{id:?}");
    assert!(shown.starts_with("DeviceIdentity("));
    assert!(shown.len() < 40);
}

// ---- the certificate

#[test]
fn certificate_is_p256_ecdsa_and_carries_the_pin() {
    let id = DeviceIdentity::generate();
    let der = id.certificate_der().unwrap();
    assert_eq!(certificate_fingerprint(&der).unwrap(), id.fingerprint());
    assert_eq!(der[0], 0x30);
    let spki = certificate_spki(&der).unwrap();
    assert_eq!(spki, spki_der(&id.public_point()).unwrap().as_slice());
    // Signature algorithm ecdsa-with-SHA256 (1.2.840.10045.4.3.2) appears in it.
    let oid = [0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
    assert!(der.windows(oid.len()).any(|w| w == oid));
    // Rebuilt each time with another serial and CN, same pin.
    let der2 = id.certificate_der().unwrap();
    assert_ne!(der, der2);
    assert_eq!(certificate_fingerprint(&der2).unwrap(), id.fingerprint());
}

#[test]
fn certificate_and_key_pair_up_for_rustls() {
    let id = DeviceIdentity::generate();
    let cert = rustls::pki_types::CertificateDer::from(id.certificate_der().unwrap());
    let key = rustls::pki_types::PrivateKeyDer::try_from(id.to_pkcs8_der()).unwrap();
    let provider = rustls::crypto::ring::default_provider();
    let signing = provider.key_provider.load_private_key(key).unwrap();
    let ck = rustls::sign::CertifiedKey::new(vec![cert], signing);
    assert!(ck.keys_match().is_ok());
}

#[test]
fn certificate_fingerprint_refuses_garbage_and_truncation() {
    assert!(certificate_fingerprint(&[]).is_none());
    assert!(certificate_fingerprint(&[0x30, 0x03, 0x01, 0x02, 0x03]).is_none());
    let id = DeviceIdentity::generate();
    let mut der = id.certificate_der().unwrap();
    der.truncate(40);
    assert!(certificate_fingerprint(&der).is_none());
}

// ---- key store

struct Flaky {
    inner: MemoryKeyStore,
    fail_load: bool,
    drop_save: bool,
}
impl KeyStore for Flaky {
    fn load(&self) -> Result<Option<Vec<u8>>, IdentityError> {
        if self.fail_load {
            Err(IdentityError::Store("cannot read".into()))
        } else {
            self.inner.load()
        }
    }
    fn save(&self, k: &[u8]) -> Result<(), IdentityError> {
        if self.drop_save {
            Ok(())
        } else {
            self.inner.save(k)
        }
    }
    fn delete(&self) -> Result<(), IdentityError> {
        self.inner.delete()
    }
}

#[test]
fn load_or_create_makes_once_and_reuses() {
    let s = MemoryKeyStore::new();
    assert!(s.load().unwrap().is_none());
    let a = load_or_create(&s).unwrap();
    let b = load_or_create(&s).unwrap();
    assert_eq!(a.fingerprint(), b.fingerprint());
    s.delete().unwrap();
    assert_ne!(load_or_create(&s).unwrap().fingerprint(), a.fingerprint());
}

#[test]
fn a_failed_read_makes_no_second_key() {
    let s = Flaky {
        inner: MemoryKeyStore::new(),
        fail_load: true,
        drop_save: false,
    };
    assert!(load_or_create(&s).is_err());
    assert!(s.inner.load().unwrap().is_none()); // nothing written over a possibly-present key
}

#[test]
fn a_store_that_does_not_keep_the_key_is_an_error() {
    let s = Flaky {
        inner: MemoryKeyStore::new(),
        fail_load: false,
        drop_save: true,
    };
    assert!(load_or_create(&s).is_err());
}

#[test]
fn a_stored_value_that_is_not_a_key_is_never_silently_replaced() {
    let s = MemoryKeyStore::new();
    s.save(b"garbage").unwrap();
    assert!(load_or_create(&s).is_err());
    assert_eq!(s.load().unwrap().unwrap(), b"garbage");
}
