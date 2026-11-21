//! Live TLS probe against a real Mac (W-14, risks 1 and 2): handshake only, ALPN `sill/1`, a
//! throwaway unpaired client key, no pin. Sends NO Sill message; reads up to 3 s and prints.
//!
//! Safety: at most 3 attempts in total, at least 30 s apart, enforced through a state file
//! (`SILL_PROBE_STATE`, default `<tmp>/sill_tls_probe_attempts`). Never uses `sill-pair/1`.
//!
//!   cargo run -p sill-net --example tls_probe [HOST:PORT]    (default 100.88.89.4:7455)

use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sill_net::tls::{self, SESSION_ALPN};

const MAX_ATTEMPTS: usize = 3;
const MIN_GAP_SECS: u64 = 30;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn main() {
    let target = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "100.88.89.4:7455".into());
    let state = std::env::var("SILL_PROBE_STATE").unwrap_or_else(|_| {
        std::env::temp_dir()
            .join("sill_tls_probe_attempts")
            .display()
            .to_string()
    });
    let past: Vec<u64> = std::fs::read_to_string(&state)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    if past.len() >= MAX_ATTEMPTS {
        eprintln!(
            "REFUSED: {} attempts already made ({state}); the limit is {MAX_ATTEMPTS}",
            past.len()
        );
        std::process::exit(2);
    }
    if let Some(last) = past.last() {
        let gap = now().saturating_sub(*last);
        if gap < MIN_GAP_SECS {
            eprintln!(
                "REFUSED: last attempt {gap} s ago; wait {} s more",
                MIN_GAP_SECS - gap
            );
            std::process::exit(2);
        }
    }
    let mut all = past.clone();
    all.push(now());
    std::fs::write(
        &state,
        all.iter().map(|t| format!("{t}\n")).collect::<String>(),
    )
    .unwrap();
    println!("attempt {} of {MAX_ATTEMPTS} to {target} (ALPN {SESSION_ALPN}, no pin, throwaway client key)", all.len());

    // throwaway unpaired device identity
    let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let cert = rcgen::CertificateParams::new(vec!["sillprobe".into()])
        .unwrap()
        .self_signed(&kp)
        .unwrap();
    let (cfg, observed) =
        tls::client_config(SESSION_ALPN, cert.der().to_vec(), kp.serialize_der(), None).unwrap();

    let addr = target.to_socket_addrs().unwrap().next().unwrap();
    let mut tcp = match TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
        Ok(t) => t,
        Err(e) => {
            println!("TCP connect failed: {e}");
            return;
        }
    };
    tcp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    tcp.set_nodelay(true).ok();
    let mut conn = rustls::ClientConnection::new(cfg, tls::server_name()).unwrap();

    let mut handshake = Ok(());
    while conn.is_handshaking() {
        if let Err(e) = conn.complete_io(&mut tcp) {
            handshake = Err(e);
            break;
        }
    }
    match &handshake {
        Ok(()) => println!("handshake: OK (client considers it complete)"),
        Err(e) => println!("handshake: FAILED: {e:?} (kind {:?})", e.kind()),
    }
    let o = observed.snapshot();
    println!(
        "negotiated ALPN: {:?}",
        conn.alpn_protocol()
            .map(|p| String::from_utf8_lossy(p).into_owned())
    );
    println!("TLS version: {:?}", conn.protocol_version());
    println!(
        "cipher suite: {:?}",
        conn.negotiated_cipher_suite().map(|c| c.suite())
    );
    println!("Mac cert DER length: {:?}", o.cert_der_len);
    println!("Mac SPKI SHA-256 base64url: {:?}", o.spki_base64url());
    println!("Mac SPKI SHA-256 hex: {:?}", o.spki_hex());
    println!(
        "webpki parsed the Mac cert / signature verified by rustls: {}",
        o.cert_der_len.is_some() && !o.used_p256_fallback
    );
    println!(
        "p256 fallback used: {} ({:?})",
        o.used_p256_fallback, o.webpki_error
    );
    println!("CertificateRequest received: {}", o.client_cert_resolved);
    println!(
        "  signature schemes asked: {:?}",
        o.certificate_request_schemes
    );
    println!(
        "  certificate_authorities count: {:?}",
        o.certificate_request_root_hints
    );

    if handshake.is_ok() {
        // Read only: whatever the Mac sends first (an alert, a kind 22 goodbye, or nothing).
        tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let start = std::time::Instant::now();
        let mut plain = Vec::new();
        loop {
            if start.elapsed() >= Duration::from_secs(3) {
                println!("read: nothing more within 3 s");
                break;
            }
            // pull TLS records, then drain plaintext
            match conn.complete_io(&mut tcp) {
                Ok((rd, _)) if rd == 0 && !conn.wants_read() => {
                    println!("read: EOF");
                    break;
                }
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    println!("read: nothing more within 3 s");
                    break;
                }
                Err(e) => {
                    println!("read: error/alert: {e:?} (kind {:?})", e.kind());
                    break;
                }
            }
            let mut buf = [0u8; 4096];
            match conn.reader().read(&mut buf) {
                Ok(0) => {
                    println!("read: peer closed the TLS stream (close_notify)");
                    break;
                }
                Ok(n) => plain.extend(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    println!("read: {e:?}");
                    break;
                }
            }
        }
        let mut i = 0;
        while plain.len() >= i + 14 {
            let kind = plain[i];
            let len = u32::from_be_bytes(plain[i + 10..i + 14].try_into().unwrap()) as usize;
            let have = plain.len().saturating_sub(i + 14).min(len);
            println!("message: kind {kind}, payload length {len} (have {have})");
            if kind == 22 {
                println!(
                    "  goodbye JSON: {}",
                    String::from_utf8_lossy(&plain[i + 14..i + 14 + have])
                );
            }
            i += 14 + len;
        }
        if plain.is_empty() {
            println!("no application bytes received");
        }
    }
    // close politely; never any Sill message
    conn.send_close_notify();
    let _ = conn.complete_io(&mut tcp);
}
