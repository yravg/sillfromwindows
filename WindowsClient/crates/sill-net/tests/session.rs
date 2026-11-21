//! sill_net::session offline: a fake Mac on 127.0.0.1 (sill-net's own crypto) that expects hello
//! first, sends a window list, a signed kind 18, a pong, frames and a goodbye. Nothing here dials a
//! non-loopback host.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rustls::crypto::{ring as ring_provider, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::{ResolvesServerCert, ServerConfig};
use rustls::sign::CertifiedKey;
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use serde_json::json;
use sill_net::identity::DeviceIdentity;
use sill_net::pairing::sign_mac_info;
use sill_net::session::*;
use sill_net::tls::{self, SESSION_ALPN};
use sill_proto::compat::Hello;
use sill_proto::switcher::StreamSource;
use sill_proto::viewport::Viewport;
use sill_proto::{wire, Caps, Message, MessageKind, MessageReader};

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

type Got = Arc<Mutex<Vec<(MessageKind, Vec<u8>)>>>;

trait Rw: Read + Write {}
impl<T: Read + Write> Rw for T {}

struct Mac {
    port: u16,
    /// Every message the device sent, in order: (kind, payload).
    got: Got,
    client_fp: Arc<Mutex<Option<[u8; 32]>>>,
}

/// One connection: handshake, then `script(&mut stream, got)` plays the Mac.
fn spawn_mac(mac: &DeviceIdentity, script: impl FnOnce(&mut dyn Rw, &Got) + Send + 'static) -> Mac {
    let provider = Arc::new(ring_provider::default_provider());
    let key = PrivateKeyDer::try_from(mac.to_pkcs8_der()).unwrap();
    let ck = Arc::new(CertifiedKey::new(
        vec![CertificateDer::from(mac.certificate_der().unwrap())],
        ring_provider::sign::any_ecdsa_type(&key).unwrap(),
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
    cfg.alpn_protocols = vec![SESSION_ALPN.as_bytes().to_vec()];
    let cfg = Arc::new(cfg);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let got = Arc::new(Mutex::new(Vec::new()));
    let g = got.clone();
    thread::spawn(move || {
        let (mut tcp, _) = listener.accept().unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut conn = rustls::ServerConnection::new(cfg).unwrap();
        while conn.is_handshaking() {
            if conn.complete_io(&mut tcp).is_err() {
                return;
            }
        }
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        script(&mut tls, &g);
        let _ = tls.flush();
        // linger and read what the device still sends: closing with unread data would reset the
        // connection and discard what the device has not read yet
        let mut reader = MessageReader::new(Caps::HOST);
        let sink: Got = Arc::new(Mutex::new(Vec::new()));
        read_until(
            &mut tls,
            &sink,
            &mut reader,
            Duration::from_millis(600),
            |_| false,
        );
        conn.send_close_notify();
        let _ = conn.complete_io(&mut tcp);
    });
    Mac {
        port,
        got,
        client_fp: seen,
    }
}

/// Reads whole messages until `want(&got)` holds or the deadline passes.
fn read_until(
    s: &mut dyn Rw,
    got: &Got,
    reader: &mut MessageReader,
    limit: Duration,
    want: impl Fn(&[(MessageKind, Vec<u8>)]) -> bool,
) {
    let end = Instant::now() + limit;
    let mut buf = [0u8; 4096];
    while Instant::now() < end && !want(&got.lock().unwrap()) {
        while let Ok(Some(m)) = reader.next_message() {
            got.lock().unwrap().push((m.kind, m.payload));
        }
        if want(&got.lock().unwrap()) {
            break;
        }
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => reader.push(&buf[..n]),
            Err(_) => {}
        }
    }
}

fn send(s: &mut dyn Rw, kind: MessageKind, payload: Vec<u8>) {
    s.write_all(&Message::new(kind, 1.0, false, payload).serialize().unwrap())
        .unwrap();
    s.flush().unwrap();
}

fn list_json() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "macName": "Fake Mac",
        "windows": [{"id": 7, "title": "Notes", "appName": "Notes", "bundleID": "com.apple.Notes", "width": 800, "height": 600}],
        "active": {"none": {}},
        "hostVersion": "0.6.1", "protocol": 1
    }))
    .unwrap()
}

fn info_json(mac: &DeviceIdentity, issued: f64) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "v": 1, "macID": mac.mac_id(), "name": "Fake Mac", "issuedAt": issued,
        "remoteAccess": true, "remotePort": 7455, "internet": false, "addresses": []
    }))
    .unwrap()
}

fn cfg(port: u16, pin: [u8; 32]) -> SessionConfig {
    SessionConfig {
        target: format!("127.0.0.1:{port}"),
        pin,
        device_name: "TESTPC (Windows)".into(),
        audio: false,
        connect_timeout: Duration::from_secs(10),
    }
}

fn drain(s: &Session, limit: Duration) -> Vec<SessionEvent> {
    let end = Instant::now() + limit;
    let mut out = Vec::new();
    while Instant::now() < end {
        if let Ok(e) = s.events.recv_timeout(Duration::from_millis(100)) {
            let closed = matches!(e, SessionEvent::Closed(_));
            out.push(e);
            if closed {
                break;
            }
        }
    }
    out
}

#[test]
fn hello_first_events_in_order_pings_viewport_pick_and_goodbye() {
    let mac = DeviceIdentity::generate();
    let mac2 = mac.clone();
    let server = spawn_mac(&mac, move |s, got| {
        let mut reader = MessageReader::new(Caps::HOST);
        // hello must be the first message
        read_until(s, got, &mut reader, Duration::from_secs(5), |g| {
            !g.is_empty()
        });
        send(s, MessageKind::WindowList, list_json());
        let (i, k, sg) = sign_mac_info(&info_json(&mac2, 10.0), &mac2);
        send(
            s,
            MessageKind::MacInfo,
            serde_json::to_vec(&json!({"info": i, "key": k, "sig": sg})).unwrap(),
        );
        // answer the first ping with a pong carrying the same 8 bytes
        read_until(s, got, &mut reader, Duration::from_secs(5), |g| {
            g.iter().any(|(k, _)| *k == MessageKind::Ping)
        });
        let ping = got
            .lock()
            .unwrap()
            .iter()
            .find(|(k, _)| *k == MessageKind::Ping)
            .unwrap()
            .1
            .clone();
        send(s, MessageKind::Pong, ping);
        send(s, MessageKind::Tick, vec![]);
        send(s, MessageKind::Frame, vec![0, 0, 0, 1]);
        // wait for the viewport, the pick, the keyframe request and a stats report (1 s)
        read_until(s, got, &mut reader, Duration::from_secs(5), |g| {
            [
                MessageKind::Viewport,
                MessageKind::SelectSource,
                MessageKind::KeyframeRequest,
                MessageKind::ClientStats,
            ]
            .iter()
            .all(|w| g.iter().any(|(k, _)| k == w))
        });
        send(
            s,
            MessageKind::Goodbye,
            serde_json::to_vec(&json!({"reason": "quit", "message": "bye"})).unwrap(),
        );
    });
    let id = DeviceIdentity::generate();
    let session = Session::open(&cfg(server.port, mac.fingerprint()), &id).unwrap();
    assert_eq!(session.mac_fingerprint, mac.fingerprint());
    assert_eq!(session.hello.device.as_deref(), Some("TESTPC (Windows)"));
    assert!(session.send_viewport(&Viewport {
        width: 1280.0,
        height: 720.0,
        scale: None,
        local_cursor: None,
        fps: Some(60)
    }));
    // outside what a host takes: not sent
    assert!(!session.send_viewport(&Viewport {
        width: 1e300,
        height: 720.0,
        scale: None,
        local_cursor: None,
        fps: None
    }));
    assert!(session.select_source(StreamSource::Desktop));
    assert!(session.request_keyframe());
    let events = drain(&session, Duration::from_secs(10));

    let kinds: Vec<&str> = events
        .iter()
        .map(|e| match e {
            SessionEvent::WindowList(_) => "list",
            SessionEvent::HostSettings(_) => "settings",
            SessionEvent::MacInfo { verified: true, .. } => "macinfo-ok",
            SessionEvent::MacInfo { .. } => "macinfo-bad",
            SessionEvent::Goodbye(_) => "goodbye",
            SessionEvent::Pong { .. } => "pong",
            SessionEvent::Message(_) => "message",
            SessionEvent::Closed(_) => "closed",
        })
        .collect();
    assert_eq!(
        kinds,
        ["list", "macinfo-ok", "pong", "message", "goodbye", "closed"],
        "{events:?}"
    );
    if let SessionEvent::WindowList(l) = &events[0] {
        assert_eq!(l.mac_name, "Fake Mac");
        assert_eq!(l.windows[0].title, "Notes");
    }
    if let SessionEvent::Pong { rtt_ms } = &events[2] {
        assert!(*rtt_ms >= 0.0 && *rtt_ms < 5000.0, "{rtt_ms}");
    }
    if let SessionEvent::Message(m) = &events[3] {
        assert_eq!(m.kind, MessageKind::Frame); // the tick was swallowed
    }
    assert!(
        matches!(events.last(), Some(SessionEvent::Closed(CloseReason::Goodbye(w))) if w == "quit")
    );

    // what the Mac received: hello FIRST, then the rest; our key reached it
    let got = server.got.lock().unwrap();
    assert_eq!(got[0].0, MessageKind::Hello, "hello must come first");
    let hello: Hello = wire::decode_object(&got[0].1).unwrap();
    assert_eq!(hello.device.as_deref(), Some("TESTPC (Windows)"));
    assert_eq!(hello.protocol, Some(1));
    assert_eq!(hello.audio, None);
    let pick = got
        .iter()
        .find(|(k, _)| *k == MessageKind::SelectSource)
        .unwrap();
    assert_eq!(
        String::from_utf8(pick.1.clone()).unwrap(),
        r#"{"desktop":{}}"#
    );
    let vps = got
        .iter()
        .filter(|(k, _)| *k == MessageKind::Viewport)
        .count();
    assert_eq!(vps, 1, "the out-of-bounds viewport was not sent");
    let ping = got.iter().find(|(k, _)| *k == MessageKind::Ping).unwrap();
    assert_eq!(ping.1.len(), 8);
    let stats = got
        .iter()
        .find(|(k, _)| *k == MessageKind::ClientStats)
        .unwrap();
    let st: sill_proto::viewport::ClientStats = wire::decode_object(&stats.1).unwrap();
    assert_eq!(st.device, "TESTPC (Windows)");
    assert_eq!(st.frame_age_ms, -1);
    assert_eq!(*server.client_fp.lock().unwrap(), Some(id.fingerprint()));
    session.close();
}

#[test]
fn hello_lists_aac_eld_only_when_asked() {
    assert_eq!(
        make_hello("x", true).audio,
        Some(vec!["aac-eld".to_string()])
    );
    assert_eq!(make_hello("x", false).audio, None);
}

#[test]
fn pins_the_mac_key_and_sends_nothing_on_a_mismatch() {
    let mac = DeviceIdentity::generate();
    let server = spawn_mac(&mac, |_s, _got| {});
    let e = Session::open(&cfg(server.port, [3u8; 32]), &DeviceIdentity::generate())
        .err()
        .unwrap();
    assert!(matches!(e, DialError::PinMismatch), "{e:?}");
    thread::sleep(Duration::from_millis(200));
    assert!(server.got.lock().unwrap().is_empty());
    assert!(
        server.client_fp.lock().unwrap().is_none(),
        "our certificate was not sent"
    );
}

#[test]
fn a_mac_that_refuses_us_closes_without_serving() {
    // the Mac completes the handshake, reads our hello and closes: an unpaired key
    let mac = DeviceIdentity::generate();
    let server = spawn_mac(&mac, |s, got| {
        let mut reader = MessageReader::new(Caps::HOST);
        read_until(s, got, &mut reader, Duration::from_secs(5), |g| {
            !g.is_empty()
        });
    });
    let session = Session::open(
        &cfg(server.port, mac.fingerprint()),
        &DeviceIdentity::generate(),
    )
    .unwrap();
    let events = drain(&session, Duration::from_secs(10));
    assert!(
        matches!(
            events.last(),
            Some(SessionEvent::Closed(CloseReason::Eof { served: false }))
        ),
        "{events:?}"
    );
}

#[test]
fn a_kind_18_signed_by_another_key_is_not_verified() {
    let mac = DeviceIdentity::generate();
    let other = DeviceIdentity::generate();
    let (o2, m2) = (other.clone(), mac.clone());
    let server = spawn_mac(&mac, move |s, got| {
        let mut reader = MessageReader::new(Caps::HOST);
        read_until(s, got, &mut reader, Duration::from_secs(5), |g| {
            !g.is_empty()
        });
        // validly signed, but by a key that is not the saved Mac's
        let (i, k, sg) = sign_mac_info(&info_json(&o2, 1.0), &o2);
        send(
            s,
            MessageKind::MacInfo,
            serde_json::to_vec(&json!({"info": i, "key": k, "sig": sg})).unwrap(),
        );
        // a forged signature
        let (i, k, _) = sign_mac_info(&info_json(&m2, 2.0), &m2);
        let (_, _, bad) = sign_mac_info(b"other bytes", &m2);
        send(
            s,
            MessageKind::MacInfo,
            serde_json::to_vec(&json!({"info": i, "key": k, "sig": bad})).unwrap(),
        );
        // an older issuedAt after a good one
        let (i, k, sg) = sign_mac_info(&info_json(&m2, 50.0), &m2);
        send(
            s,
            MessageKind::MacInfo,
            serde_json::to_vec(&json!({"info": i, "key": k, "sig": sg})).unwrap(),
        );
        let (i, k, sg) = sign_mac_info(&info_json(&m2, 40.0), &m2);
        send(
            s,
            MessageKind::MacInfo,
            serde_json::to_vec(&json!({"info": i, "key": k, "sig": sg})).unwrap(),
        );
        send(
            s,
            MessageKind::Goodbye,
            serde_json::to_vec(&json!({"reason": "quit"})).unwrap(),
        );
    });
    let session = Session::open(
        &cfg(server.port, mac.fingerprint()),
        &DeviceIdentity::generate(),
    )
    .unwrap();
    let events = drain(&session, Duration::from_secs(10));
    let verdicts: Vec<(bool, Option<String>)> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::MacInfo {
                verified, problem, ..
            } => Some((*verified, problem.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(verdicts.len(), 4, "{events:?}");
    assert!(!verdicts[0].0 && verdicts[0].1.as_deref().unwrap().contains("another key"));
    assert!(!verdicts[1].0 && verdicts[1].1.as_deref().unwrap().contains("bad signature"));
    assert!(verdicts[2].0);
    assert!(!verdicts[3].0 && verdicts[3].1.as_deref().unwrap().contains("older"));
}

#[test]
fn liveness_is_six_seconds_or_four_round_trips() {
    assert_eq!(liveness_limit(Duration::ZERO), Duration::from_secs(6));
    assert_eq!(
        liveness_limit(Duration::from_millis(1400)),
        Duration::from_secs(6)
    );
    assert_eq!(
        liveness_limit(Duration::from_secs(2)),
        Duration::from_secs(8)
    );
}

#[test]
fn closing_ends_the_session_with_requested() {
    let mac = DeviceIdentity::generate();
    let server = spawn_mac(&mac, |s, got| {
        let mut reader = MessageReader::new(Caps::HOST);
        read_until(s, got, &mut reader, Duration::from_secs(3), |_| false);
    });
    let session = Session::open(
        &cfg(server.port, mac.fingerprint()),
        &DeviceIdentity::generate(),
    )
    .unwrap();
    thread::sleep(Duration::from_millis(300));
    session.close();
}
