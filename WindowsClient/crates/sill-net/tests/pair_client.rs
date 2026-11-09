//! sill_net::pair_client offline: the kind 19 bytes against what Scripts/sillclient.py's `pair`
//! builds (python3 -I, no network), and a fake Mac's pairing door on 127.0.0.1 built from sill-net's
//! own crypto. Nothing here dials a non-loopback host.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rustls::crypto::{ring as ring_provider, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::{ResolvesServerCert, ServerConfig};
use rustls::sign::CertifiedKey;
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use sill_net::damm;
use sill_net::identity::{base64url_decode, base64url_encode, mac_id, DeviceIdentity};
use sill_net::pair_client::*;
use sill_net::pairing::{proof, PairLink, ParsedAddress};
use sill_net::session::DialError;
use sill_net::tls::{self, PAIRING_ALPN};
use sill_proto::remote::{PairRequest, PairResult};
use sill_proto::{wire, Caps, Message, MessageKind, MessageReader};

const T: Duration = Duration::from_secs(10);
const CODE: &str = "482913557208"; // placeholder; fixed below to a valid check digit

fn valid_code() -> String {
    // 11 digits and their Damm check digit, so the local check passes
    let digits: Vec<u8> = CODE.bytes().take(11).map(|b| b - b'0').collect();
    let mut s: String = digits.iter().map(|d| char::from(b'0' + d)).collect();
    s.push(char::from(b'0' + damm::check_digit(&digits)));
    s
}

// --- oracle ------------------------------------------------------------------------------------

/// A transcription of sillclient.py's `pair()` (lines using hashlib.pbkdf2_hmac, hmac and `msg`),
/// run with `python3 -I`; `drift_guard` fails if the script's lines change.
const ORACLE: &str = r#"
import hashlib, hmac, struct, json, base64, sys
b64u = lambda b: base64.urlsafe_b64encode(b).rstrip(b"=").decode()
method, secret_or_code, fp_mac, fp_dev, name, ts = sys.argv[1:7]
fp_mac = bytes.fromhex(fp_mac); fp_dev = bytes.fromhex(fp_dev)
if method == "code":
    k = hashlib.pbkdf2_hmac("sha256", secret_or_code.encode(), b"sill-pair-v1" + fp_mac, 600_000, 32)
else:
    k = bytes.fromhex(secret_or_code)
proof = hmac.new(k, b"sill-pair-v1 device\x00" + fp_dev + fp_mac, hashlib.sha256).digest()
mac_proof = hmac.new(k, b"sill-pair-v1 mac\x00" + fp_mac + fp_dev, hashlib.sha256).digest()
req = {"v": 1, "method": method, "proof": b64u(proof), "name": name, "model": "Windows"}
payload = json.dumps(req, separators=(",", ":")).encode()
print((struct.pack(">BdBI", 19, float(ts), 0, len(payload)) + payload).hex())
print(b64u(mac_proof))
"#;

fn python(args: &[&str]) -> Vec<String> {
    let out = Command::new("python3")
        .arg("-I")
        .arg("-c")
        .arg(ORACLE)
        .args(args)
        .output()
        .expect("python3 is needed for the sillclient cross-check");
    assert!(
        out.status.success(),
        "python failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn drift_guard_the_oracle_still_matches_sillclient_py() {
    let script = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Scripts/sillclient.py"),
    )
    .unwrap();
    for needle in [
        r#"b"sill-pair-v1" + fp_mac, 600_000, 32"#,
        r#"b"sill-pair-v1 device\x00" + fp_dev + fp_mac"#,
        r#"b"sill-pair-v1 mac\x00" + fp_mac + fp_dev"#,
        r#"{"v": pair_v, "method": method, "proof": b64u(proof), "name": device, "model": "sillclient"}"#,
        r#"struct.pack(">BdBI", kind, time.time(), 1 if key else 0, len(payload))"#,
    ] {
        assert!(script.contains(needle), "sillclient.py changed: {needle}");
    }
}

#[test]
fn kind19_typed_code_bytes_equal_sillclient_py() {
    let (mac_fp, dev_fp) = ([0x11u8; 32], [0x22u8; 32]);
    let ts = 1_700_000_000.25;
    let code = valid_code();
    let py = python(&[
        "code",
        &code,
        &hex(&mac_fp),
        &hex(&dev_fp),
        "TESTPC",
        &ts.to_string(),
    ]);
    let rust = code_request_bytes(&code, &dev_fp, &mac_fp, "TESTPC", Some("Windows"), ts).unwrap();
    assert_eq!(hex(&rust), py[0]);
    // 000000000000 (the dry-run code) too
    let py = python(&[
        "code",
        "000000000000",
        &hex(&mac_fp),
        &hex(&dev_fp),
        "TESTPC",
        "0",
    ]);
    let rust = code_request_bytes(
        "000000000000",
        &dev_fp,
        &mac_fp,
        "TESTPC",
        Some("Windows"),
        0.0,
    )
    .unwrap();
    assert_eq!(hex(&rust), py[0]);
}

#[test]
fn kind19_qr_bytes_and_mac_proof_equal_sillclient_py() {
    let (mac_fp, dev_fp) = ([0x33u8; 32], [0x44u8; 32]);
    let secret = [7u8; 16];
    let py = python(&[
        "qr",
        &hex(&secret),
        &hex(&mac_fp),
        &hex(&dev_fp),
        "Win",
        "12.5",
    ]);
    let p = proof::device_proof(&proof::qr_key(&secret), &dev_fp, &mac_fp);
    let req = build_request("qr", Some(&p), "Win", Some("Windows"));
    assert_eq!(hex(&request_bytes(&req, 12.5)), py[0]);
    let mp = proof::mac_proof(&secret, &mac_fp, &dev_fp);
    assert_eq!(base64url_encode(&mp), py[1]);
}

#[test]
fn ask_and_cancel_send_an_empty_proof_and_no_cable() {
    let req = build_request("ask", None, "Win", Some("Windows"));
    assert_eq!(
        String::from_utf8(wire::encode(&req)).unwrap(),
        r#"{"v":1,"method":"ask","proof":"","name":"Win","model":"Windows"}"#
    );
}

#[test]
fn every_reason_maps() {
    let r = |reason: &str| PairResult {
        ok: false,
        reason: Some(reason.into()),
        tries_left: Some(3),
        retry_after: Some(2.5),
        message: Some("Hello\u{0007} there".into()),
        ..Default::default()
    };
    assert_eq!(
        PairRefusal::from_result(&r("code")),
        PairRefusal::WrongCode {
            tries_left: Some(3)
        }
    );
    assert_eq!(PairRefusal::from_result(&r("closed")), PairRefusal::Closed);
    assert_eq!(
        PairRefusal::from_result(&r("expired")),
        PairRefusal::Expired
    );
    assert_eq!(
        PairRefusal::from_result(&r("stopped")),
        PairRefusal::Stopped
    );
    assert_eq!(
        PairRefusal::from_result(&r("busy")),
        PairRefusal::Busy {
            retry_after: Some(2.5)
        }
    );
    assert_eq!(PairRefusal::from_result(&r("shown")), PairRefusal::Shown);
    assert_eq!(
        PairRefusal::from_result(&r("openOnMac")),
        PairRefusal::OpenOnMac
    );
    assert_eq!(PairRefusal::from_result(&r("locked")), PairRefusal::Locked);
    match PairRefusal::from_result(&r("newThing")) {
        PairRefusal::Other { reason, message } => {
            assert_eq!(reason.as_deref(), Some("newThing"));
            assert!(!message.unwrap().contains('\u{0007}'), "SafeText applied");
        }
        o => panic!("{o:?}"),
    }
    let none = PairResult {
        ok: false,
        ..Default::default()
    };
    assert!(matches!(
        PairRefusal::from_result(&none),
        PairRefusal::Other { reason: None, .. }
    ));
}

#[test]
fn saved_mac_dial_targets() {
    let m = SavedMac {
        name: "x".into(),
        mac_id: "A".into(),
        fingerprint: String::new(),
        addresses: vec![
            "100.1.2.3".into(),
            "mac.example:9000".into(),
            "[fd7a::1]".into(),
            "[fd7a::2]:81".into(),
        ],
        port: 7455,
        recognition_key: None,
        method: "code".into(),
    };
    assert_eq!(
        m.dial_targets(),
        [
            "100.1.2.3:7455",
            "mac.example:9000",
            "[fd7a::1]:7455",
            "[fd7a::2]:81"
        ]
    );
}

// --- a fake Mac's pairing door ------------------------------------------------------------------

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
#[derive(Debug)]
struct Resolver(Arc<CertifiedKey>);
impl ResolvesServerCert for Resolver {
    fn resolve(&self, _: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}

/// What the door's handler is told: the request, the device's key from TLS, the Mac's own key.
type Handler = Box<dyn Fn(&PairRequest, [u8; 32], [u8; 32]) -> PairResult + Send + Sync>;

struct Door {
    port: u16,
    requests: Arc<Mutex<Vec<PairRequest>>>,
    alpn: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
}

/// Serves any number of connections on 127.0.0.1 (the thread ends with the test process).
fn spawn_door(mac: &DeviceIdentity, handler: Handler) -> Door {
    let provider = Arc::new(ring_provider::default_provider());
    let key = PrivateKeyDer::try_from(mac.to_pkcs8_der()).unwrap();
    let signer = ring_provider::sign::any_ecdsa_type(&key).unwrap();
    let ck = Arc::new(CertifiedKey::new(
        vec![CertificateDer::from(mac.certificate_der().unwrap())],
        signer,
    ));
    let seen = Arc::new(Mutex::new(None));
    let mut cfg = ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_client_cert_verifier(Arc::new(AnyClient {
            provider,
            seen: seen.clone(),
        }))
        .with_cert_resolver(Arc::new(Resolver(ck)));
    cfg.alpn_protocols = vec![b"sill/1".to_vec(), PAIRING_ALPN.as_bytes().to_vec()];
    let cfg = Arc::new(cfg);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mac_fp = mac.fingerprint();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let alpn = Arc::new(Mutex::new(Vec::new()));
    let (req_log, alpn_log) = (requests.clone(), alpn.clone());
    thread::spawn(move || {
        for tcp in listener.incoming() {
            let Ok(mut tcp) = tcp else { continue };
            tcp.set_read_timeout(Some(Duration::from_secs(120)))
                .unwrap(); // debug PBKDF2 is slow
            let mut conn = rustls::ServerConnection::new(cfg.clone()).unwrap();
            let ok = (|| {
                while conn.is_handshaking() {
                    conn.complete_io(&mut tcp).ok()?;
                }
                Some(())
            })();
            if ok.is_none() {
                continue;
            }
            alpn_log
                .lock()
                .unwrap()
                .push(conn.alpn_protocol().map(|p| p.to_vec()));
            let dev_fp = seen.lock().unwrap().take().unwrap_or([0; 32]);
            let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
            let mut reader = MessageReader::new(Caps::PAIRING);
            let mut buf = [0u8; 1024];
            let msg = loop {
                match reader.next_message() {
                    Ok(Some(m)) => break Some(m),
                    Ok(None) => {}
                    Err(_) => break None,
                }
                match tls.read(&mut buf) {
                    Ok(0) | Err(_) => break None,
                    Ok(n) => reader.push(&buf[..n]),
                }
            };
            let Some(m) = msg else { continue };
            assert_eq!(m.kind, MessageKind::PairRequest);
            let req: PairRequest = wire::decode_object(&m.payload).unwrap();
            req_log.lock().unwrap().push(req.clone());
            let result = handler(&req, dev_fp, mac_fp);
            let reply = Message::new(MessageKind::PairResult, 0.0, false, wire::encode(&result))
                .serialize()
                .unwrap();
            let _ = tls.write_all(&reply);
            let _ = tls.flush();
            conn.send_close_notify();
            let _ = conn.complete_io(&mut tcp);
        }
    });
    Door {
        port,
        requests,
        alpn,
    }
}

/// A Mac that accepts `code` (typed path, key derived from its own key) or `secret` (qr), and
/// answers a correct ok. `tamper` changes the ok.
fn honest(code: Option<String>, secret: Option<[u8; 16]>, tamper: u8) -> Handler {
    Box::new(move |req, dev_fp, mac_fp| {
        let key: Vec<u8> = match req.method.as_str() {
            "code" => proof::code_key(code.as_ref().unwrap(), &mac_fp)
                .unwrap()
                .to_vec(),
            "qr" => secret.unwrap().to_vec(),
            "ask" => {
                return PairResult {
                    ok: false,
                    reason: Some("shown".into()),
                    ..Default::default()
                }
            }
            _ => {
                return PairResult {
                    ok: false,
                    reason: Some("closed".into()),
                    ..Default::default()
                }
            }
        };
        let sent = base64url_decode(&req.proof).unwrap_or_default();
        if !proof::is_valid_device_proof(&sent, &key, &dev_fp, &mac_fp) {
            return PairResult {
                ok: false,
                reason: Some("code".into()),
                tries_left: Some(4),
                ..Default::default()
            };
        }
        let mut mp = proof::mac_proof(&key, &mac_fp, &dev_fp);
        if tamper == 1 {
            mp[0] ^= 1;
        }
        PairResult {
            ok: true,
            proof: Some(base64url_encode(&mp)),
            mac_id: Some(if tamper == 2 {
                mac_id(&[9u8; 32])
            } else {
                mac_id(&mac_fp)
            }),
            name: Some("Fake Mac".into()),
            recognition_key: Some(base64url_encode(&[5u8; 32])),
            ..Default::default()
        }
    })
}

fn me() -> DeviceIdentity {
    DeviceIdentity::generate()
}

#[test]
fn typed_code_pairs_and_the_record_is_right() {
    let mac = me();
    let code = valid_code();
    let door = spawn_door(&mac, honest(Some(code.clone()), None, 0));
    let id = me();
    let target = format!("127.0.0.1:{}", door.port);
    // grouped input, as a person types it
    let typed = damm::grouped(&code);
    let saved = pair_with_code(
        &target,
        &typed,
        None,
        &id,
        "TESTPC (Windows)",
        Some("Windows"),
        T,
    )
    .unwrap();
    assert_eq!(saved.name, "Fake Mac");
    assert_eq!(saved.mac_id, mac.mac_id());
    assert_eq!(saved.pin(), Some(mac.fingerprint()));
    assert_eq!(saved.addresses, vec!["127.0.0.1".to_string()]);
    assert_eq!(saved.port, door.port);
    assert_eq!(saved.recognition_key, Some(base64url_encode(&[5u8; 32])));
    assert_eq!(saved.method, "code");
    // exactly one kind 19 on one pairing connection, ALPN sill-pair/1, name as given
    let reqs = door.requests.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].method, "code");
    assert_eq!(reqs[0].name, "TESTPC (Windows)");
    assert_eq!(
        door.alpn.lock().unwrap()[0].as_deref(),
        Some(PAIRING_ALPN.as_bytes())
    );
}

#[test]
fn wrong_code_is_refused_with_tries_left_and_nothing_is_saved() {
    let mac = me();
    let door = spawn_door(&mac, honest(Some(valid_code()), None, 0));
    // another valid code
    let other = "000000000000";
    assert_ne!(other, valid_code());
    let e = pair_with_code(
        &format!("127.0.0.1:{}", door.port),
        other,
        None,
        &me(),
        "w",
        None,
        T,
    )
    .unwrap_err();
    assert!(
        matches!(
            e,
            PairError::Refused(PairRefusal::WrongCode {
                tries_left: Some(4)
            })
        ),
        "{e:?}"
    );
}

#[test]
fn a_typo_never_reaches_the_network() {
    // nothing listens on port 1: a connection attempt would be a Dial error
    let mut typo = valid_code().into_bytes();
    typo[11] = if typo[11] == b'9' { b'0' } else { typo[11] + 1 };
    let typo = String::from_utf8(typo).unwrap();
    let e = pair_with_code("127.0.0.1:1", &typo, None, &me(), "w", None, T).unwrap_err();
    assert!(
        matches!(e, PairError::BadCode(damm::Problem::Typo)),
        "{e:?}"
    );
    let e = pair_with_code("127.0.0.1:1", "123", None, &me(), "w", None, T).unwrap_err();
    assert!(
        matches!(e, PairError::BadCode(damm::Problem::Length)),
        "{e:?}"
    );
}

#[test]
fn a_bad_mac_proof_or_mac_id_saves_nothing() {
    for (tamper, want) in [(1u8, "BadMacProof"), (2u8, "MacIdMismatch")] {
        let mac = me();
        let door = spawn_door(&mac, honest(Some(valid_code()), None, tamper));
        let e = pair_with_code(
            &format!("127.0.0.1:{}", door.port),
            &valid_code(),
            None,
            &me(),
            "w",
            None,
            T,
        )
        .unwrap_err();
        assert!(format!("{e:?}").starts_with(want), "{e:?}");
    }
}

#[test]
fn link_pairs_pinned_with_the_qr_secret() {
    let mac = me();
    let secret = [9u8; 16];
    let door = spawn_door(&mac, honest(None, Some(secret), 0));
    let link = PairLink::new(
        &mac.fingerprint(),
        &secret,
        "Linked Mac",
        7455,
        vec![ParsedAddress {
            host: "127.0.0.1".into(),
            port: Some(door.port),
            kind: sill_net::pairing::AddressKind::Ipv4,
        }],
    );
    let saved = pair_with_link(&link.url(), &me(), "w", Some("Windows"), T).unwrap();
    assert_eq!(saved.mac_id, mac.mac_id());
    assert_eq!(saved.method, "qr");
    assert_eq!(saved.addresses, vec![format!("127.0.0.1:{}", door.port)]);
    assert_eq!(door.requests.lock().unwrap()[0].method, "qr");
}

#[test]
fn link_pinned_to_another_key_fails_before_sending_anything() {
    let mac = me();
    let secret = [9u8; 16];
    let door = spawn_door(&mac, honest(None, Some(secret), 0));
    let impostor = me(); // the link names this key, the door has another
    let link = PairLink::new(
        &impostor.fingerprint(),
        &secret,
        "x",
        door.port,
        vec![ParsedAddress {
            host: "127.0.0.1".into(),
            port: None,
            kind: sill_net::pairing::AddressKind::Ipv4,
        }],
    );
    let e = pair_with_link(&link.url(), &me(), "w", None, T).unwrap_err();
    assert!(
        matches!(e, PairError::Dial(DialError::PinMismatch)),
        "{e:?}"
    );
    assert!(
        door.requests.lock().unwrap().is_empty(),
        "no kind 19 was sent"
    );
}

#[test]
fn a_bad_link_is_refused_locally() {
    let e = pair_with_link("sill://pair?v=2", &me(), "w", None, T).unwrap_err();
    assert!(matches!(e, PairError::BadLink(_)), "{e:?}");
}

#[test]
fn ask_returns_shown_with_the_key_then_the_code_pins_to_it() {
    let mac = me();
    let code = valid_code();
    let door = spawn_door(&mac, honest(Some(code.clone()), None, 0));
    let target = format!("127.0.0.1:{}", door.port);
    let id = me();
    let answer = ask(&target, &id, "w", Some("Windows"), T).unwrap();
    let AskAnswer::Shown { mac_fingerprint } = answer else {
        panic!("{answer:?}")
    };
    assert_eq!(mac_fingerprint, mac.fingerprint());
    assert_eq!(door.requests.lock().unwrap()[0].method, "ask");
    assert_eq!(door.requests.lock().unwrap()[0].proof, "");
    // the second connection is pinned to the key the ask saw
    pair_with_code(&target, &code, Some(mac_fingerprint), &id, "w", None, T).unwrap();
    // pinned to another key: refused before kind 19
    let n = door.requests.lock().unwrap().len();
    let e = pair_with_code(&target, &code, Some([1u8; 32]), &id, "w", None, T).unwrap_err();
    assert!(
        matches!(e, PairError::Dial(DialError::PinMismatch)),
        "{e:?}"
    );
    assert_eq!(door.requests.lock().unwrap().len(), n);
}

#[test]
fn ask_answers_and_refusals() {
    for (reason, want) in [("openOnMac", "OpenOnMac"), ("locked", "Locked")] {
        let mac = me();
        let r = reason.to_string();
        let door = spawn_door(
            &mac,
            Box::new(move |_, _, _| PairResult {
                ok: false,
                reason: Some(r.clone()),
                ..Default::default()
            }),
        );
        let a = ask(&format!("127.0.0.1:{}", door.port), &me(), "w", None, T).unwrap();
        assert!(format!("{a:?}").starts_with(want), "{a:?}");
    }
    for (reason, want) in [("closed", "Closed"), ("busy", "Busy"), ("weird", "Other")] {
        let mac = me();
        let r = reason.to_string();
        let door = spawn_door(
            &mac,
            Box::new(move |_, _, _| PairResult {
                ok: false,
                reason: Some(r.clone()),
                message: Some("words".into()),
                retry_after: Some(1.0),
                ..Default::default()
            }),
        );
        let e = ask(&format!("127.0.0.1:{}", door.port), &me(), "w", None, T).unwrap_err();
        assert!(format!("{e:?}").contains(want), "{e:?}");
    }
}

#[test]
fn a_door_that_closes_without_kind_20_is_no_reply() {
    // a TCP listener that accepts and drops: the handshake fails, a Dial error, not a hang
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    thread::spawn(move || {
        for s in l.incoming() {
            drop(s);
        }
    });
    let e = pair_with_code(
        &format!("127.0.0.1:{port}"),
        &valid_code(),
        None,
        &me(),
        "w",
        None,
        T,
    )
    .unwrap_err();
    assert!(matches!(e, PairError::Dial(DialError::Tls(_))), "{e:?}");
    let _ = TcpStream::connect(("127.0.0.1", port));
}
