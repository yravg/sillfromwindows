//! sill_net::tls against a local rustls server on 127.0.0.1 (rcgen certs, and a hand-made
//! Apple-style certificate: v3, no extensions, year-9999 notAfter).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rustls::client::ClientConnection;
use rustls::crypto::{ring as ring_provider, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::{ResolvesServerCert, ServerConfig};
use rustls::sign::CertifiedKey;
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use sha2::{Digest, Sha256};
use sill_net::tls::{self, ObservedHandle, PAIRING_ALPN, SESSION_ALPN};

struct Ident {
    cert: Vec<u8>,
    pkcs8: Vec<u8>,
    spki_fp: [u8; 32],
}

fn rcgen_ident() -> Ident {
    let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let cert = rcgen::CertificateParams::new(vec!["sill".to_string()])
        .unwrap()
        .self_signed(&kp)
        .unwrap();
    let spki = tls::spki_der(cert.der()).unwrap().to_vec();
    assert_eq!(spki, kp.public_key_der());
    Ident {
        cert: cert.der().to_vec(),
        pkcs8: kp.serialize_der(),
        spki_fp: Sha256::digest(&spki).into(),
    }
}

// --- DER writer for the Apple-style certificate (RemoteIdentity.swift:258-288) -------------------
fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut v = vec![tag];
    match content.len() {
        n if n < 0x80 => v.push(n as u8),
        n if n < 0x100 => v.extend([0x81, n as u8]),
        n => v.extend([0x82, (n >> 8) as u8, n as u8]),
    }
    v.extend(content);
    v
}

fn apple_style_ident() -> Ident {
    use p256::ecdsa::{signature::Signer, Signature, SigningKey};
    use p256::pkcs8::DecodePrivateKey;
    let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let pkcs8 = kp.serialize_der();
    let sk = SigningKey::from_pkcs8_der(&pkcs8).unwrap();
    let spki = kp.public_key_der();
    let sigalg = der(
        0x30,
        &der(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02]),
    ); // no parameters
    let name = der(
        0x30,
        &der(
            0x31,
            &der(
                0x30,
                &[
                    der(0x06, &[0x55, 0x04, 0x03]),
                    der(0x0c, b"0123456789abcdef"),
                ]
                .concat(),
            ),
        ),
    );
    let validity = der(
        0x30,
        &[der(0x17, b"260101000000Z"), der(0x18, b"99991231235959Z")].concat(),
    );
    let tbs = der(
        0x30,
        &[
            der(0xa0, &der(0x02, &[2])),
            der(
                0x02,
                &[
                    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                    0xab, 0xcd, 0xef,
                ],
            ),
            sigalg.clone(),
            name.clone(),
            validity,
            name,
            spki.to_vec(),
        ]
        .concat(),
    );
    let sig: Signature = sk.sign(&tbs);
    let sig_der = sig.to_der();
    let mut bits = vec![0u8];
    bits.extend(sig_der.as_bytes());
    let cert = der(0x30, &[tbs, sigalg, der(0x03, &bits)].concat());
    let fp = tls::spki_fingerprint(&cert).unwrap();
    assert_eq!(fp, <[u8; 32]>::from(Sha256::digest(&spki)));
    Ident {
        cert,
        pkcs8,
        spki_fp: fp,
    }
}

// --- test server -------------------------------------------------------------------------------

/// Accepts any client certificate, verifying its signature, and remembers its SPKI fingerprint.
#[derive(Debug)]
struct AnyClient {
    provider: Arc<CryptoProvider>,
    seen: Arc<Mutex<Option<[u8; 32]>>>,
}

impl ClientCertVerifier for AnyClient {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _i: &[CertificateDer<'_>],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        *self.seen.lock().unwrap() = tls::spki_fingerprint(end_entity.as_ref());
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("tls12".into()))
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

/// Presents `cert` but signs with `key` (no match check), to prove the client verifies signatures.
#[derive(Debug)]
struct Resolver(Arc<CertifiedKey>);
impl ResolvesServerCert for Resolver {
    fn resolve(&self, _: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}

struct Server {
    port: u16,
    client_seen: Arc<Mutex<Option<[u8; 32]>>>,
    negotiated: Arc<Mutex<Option<Vec<u8>>>>,
}

/// One connection, then the thread ends. `sign_with` is the key that signs (default: cert's own).
fn spawn_server(cert: &Ident, sign_with: Option<&Ident>, tls12_only: bool) -> Server {
    let provider = Arc::new(ring_provider::default_provider());
    let key = PrivateKeyDer::try_from(sign_with.unwrap_or(cert).pkcs8.clone()).unwrap();
    let signer = ring_provider::sign::any_ecdsa_type(&key).unwrap();
    let ck = Arc::new(CertifiedKey::new(
        vec![CertificateDer::from(cert.cert.clone())],
        signer,
    ));
    let seen = Arc::new(Mutex::new(None));
    let versions: &[&rustls::SupportedProtocolVersion] = if tls12_only {
        &[&rustls::version::TLS12]
    } else {
        &[&rustls::version::TLS13]
    };
    let mut cfg = ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(versions)
        .unwrap()
        .with_client_cert_verifier(Arc::new(AnyClient {
            provider,
            seen: seen.clone(),
        }))
        .with_cert_resolver(Arc::new(Resolver(ck)));
    cfg.alpn_protocols = vec![SESSION_ALPN.into(), PAIRING_ALPN.into()];
    let cfg = Arc::new(cfg);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let negotiated = Arc::new(Mutex::new(None));
    let neg = negotiated.clone();
    thread::spawn(move || {
        let (mut tcp, _) = listener.accept().unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut conn = rustls::ServerConnection::new(cfg).unwrap();
        loop {
            if conn.complete_io(&mut tcp).is_err() {
                break; // handshake failed (alert already flushed by complete_io where possible)
            }
            if !conn.is_handshaking() {
                *neg.lock().unwrap() = conn.alpn_protocol().map(|p| p.to_vec());
                let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
                let _ = tls.write_all(b"hi");
                let _ = tls.flush();
                break;
            }
        }
    });
    Server {
        port,
        client_seen: seen,
        negotiated,
    }
}

struct Outcome {
    result: Result<Vec<u8>, String>,
    observed: ObservedHandle,
    alpn: Option<Vec<u8>>,
    version: Option<rustls::ProtocolVersion>,
}

fn dial(port: u16, alpn: &str, client: &Ident, pin: Option<[u8; 32]>) -> Outcome {
    let (cfg, observed) =
        tls::client_config(alpn, client.cert.clone(), client.pkcs8.clone(), pin).unwrap();
    let mut conn = ClientConnection::new(cfg, tls::server_name()).unwrap();
    let mut tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut buf = [0u8; 2];
    let result = (|| {
        while conn.is_handshaking() {
            conn.complete_io(&mut tcp).map_err(|e| e.to_string())?;
        }
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        tls.read_exact(&mut buf).map_err(|e| e.to_string())?;
        Ok(buf.to_vec())
    })();
    Outcome {
        result,
        observed,
        alpn: conn.alpn_protocol().map(|p| p.to_vec()),
        version: conn.protocol_version(),
    }
}

#[test]
fn pin_match_passes_and_client_auth_reaches_server() {
    let (mac, dev) = (rcgen_ident(), rcgen_ident());
    let s = spawn_server(&mac, None, false);
    let o = dial(s.port, SESSION_ALPN, &dev, Some(mac.spki_fp));
    assert_eq!(o.result.unwrap(), b"hi");
    assert_eq!(o.version, Some(rustls::ProtocolVersion::TLSv1_3));
    let seen = o.observed.snapshot();
    assert!(seen.client_cert_resolved);
    assert_eq!(seen.spki_sha256, Some(mac.spki_fp));
    assert!(!seen.used_p256_fallback);
    // the server received our certificate and its SPKI hash is ours
    assert_eq!(*s.client_seen.lock().unwrap(), Some(dev.spki_fp));
    assert_eq!(tls::spki_fingerprint(&dev.cert), Some(dev.spki_fp));
}

#[test]
fn pin_mismatch_fails_without_sending_our_cert() {
    let (mac, dev) = (rcgen_ident(), rcgen_ident());
    let wrong = rcgen_ident().spki_fp;
    let s = spawn_server(&mac, None, false);
    let o = dial(s.port, SESSION_ALPN, &dev, Some(wrong));
    assert!(o.result.is_err(), "mismatched pin must fail");
    let seen = o.observed.snapshot();
    assert_eq!(
        seen.spki_sha256,
        Some(mac.spki_fp),
        "fingerprint still recorded for the error message"
    );
    // the server never received our certificate (sent only after the Mac verified)
    std::thread::sleep(Duration::from_millis(100));
    assert!(s.client_seen.lock().unwrap().is_none());
}

#[test]
fn no_pin_records_fingerprint() {
    let (mac, dev) = (rcgen_ident(), rcgen_ident());
    let s = spawn_server(&mac, None, false);
    let o = dial(s.port, PAIRING_ALPN, &dev, None);
    assert_eq!(o.result.unwrap(), b"hi");
    let seen = o.observed.snapshot();
    assert_eq!(seen.spki_sha256, Some(mac.spki_fp));
    assert_eq!(seen.cert_der_len, Some(mac.cert.len()));
    use base64::Engine;
    assert_eq!(
        seen.spki_base64url().unwrap(),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.spki_fp)
    );
    assert_eq!(seen.spki_hex().unwrap().len(), 64);
}

#[test]
fn alpn_is_negotiated_exactly() {
    for alpn in [SESSION_ALPN, PAIRING_ALPN] {
        let (mac, dev) = (rcgen_ident(), rcgen_ident());
        let s = spawn_server(&mac, None, false);
        let o = dial(s.port, alpn, &dev, Some(mac.spki_fp));
        o.result.unwrap();
        assert_eq!(o.alpn.as_deref(), Some(alpn.as_bytes()));
        assert_eq!(
            s.negotiated.lock().unwrap().as_deref(),
            Some(alpn.as_bytes())
        );
    }
}

#[test]
fn apple_style_cert_no_extensions_year_9999_is_accepted() {
    let (mac, dev) = (apple_style_ident(), rcgen_ident());
    let s = spawn_server(&mac, None, false);
    let o = dial(s.port, SESSION_ALPN, &dev, Some(mac.spki_fp));
    let seen = o.observed.snapshot();
    assert_eq!(o.result.clone().unwrap(), b"hi", "observed: {seen:?}");
    // Records whether rustls' own parser coped (risk 1); either path verified the signature.
    eprintln!(
        "apple-style cert: used_p256_fallback={} webpki_error={:?}",
        seen.used_p256_fallback, seen.webpki_error
    );
    assert_eq!(seen.spki_sha256, Some(mac.spki_fp));
}

#[test]
fn wrong_certificate_verify_signature_is_rejected() {
    // The server presents the Mac's cert (right pin) but signs with another key: CertificateVerify
    // must fail, or a replayed certificate would pass.
    for apple in [false, true] {
        let mac = if apple {
            apple_style_ident()
        } else {
            rcgen_ident()
        };
        let other = rcgen_ident();
        let dev = rcgen_ident();
        let s = spawn_server(&mac, Some(&other), false);
        let o = dial(s.port, SESSION_ALPN, &dev, Some(mac.spki_fp));
        assert!(
            o.result.is_err(),
            "forged CertificateVerify accepted (apple={apple})"
        );
    }
}

#[test]
fn tls12_server_is_refused() {
    let (mac, dev) = (rcgen_ident(), rcgen_ident());
    let s = spawn_server(&mac, None, true);
    let o = dial(s.port, SESSION_ALPN, &dev, Some(mac.spki_fp));
    assert!(o.result.is_err());
}

#[test]
fn spki_reader_rejects_garbage_and_non_p256() {
    assert!(tls::spki_der(&[]).is_none());
    assert!(tls::spki_der(&[0x30, 0x03, 0x01, 0x02, 0x03]).is_none());
    assert!(tls::spki_fingerprint(&[0xff; 40]).is_none());
    let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).unwrap();
    let cert = rcgen::CertificateParams::new(vec!["x".into()])
        .unwrap()
        .self_signed(&kp)
        .unwrap();
    assert!(tls::spki_der(cert.der()).is_some());
    assert!(
        tls::spki_fingerprint(cert.der()).is_none(),
        "only P-256 keys count"
    );
}
