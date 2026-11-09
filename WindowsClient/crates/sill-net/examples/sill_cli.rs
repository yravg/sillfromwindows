//! Developer CLI for the Windows client's network layer (W-15, W-17). Nothing here is the product's
//! UI; it exists to pair with and watch a real Mac from a terminal.
//!
//!   cargo run --release -p sill-net --example sill_cli -- identity
//!   cargo run --release -p sill-net --example sill_cli -- pair --addr HOST:PORT --code DDDDDDDDDDDD [--name NAME]
//!   cargo run --release -p sill-net --example sill_cli -- pair --link 'sill://pair?...'
//!   cargo run --release -p sill-net --example sill_cli -- pair --addr HOST:PORT --code 000000000000 --dry-run
//!   cargo run --release -p sill-net --example sill_cli -- session --mac MACID [--seconds 10] [--pick desktop|none|window:ID]
//!
//! Use --release for pairing: the typed path's PBKDF2 has 4 s on the Mac's clock.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::{Duration, Instant};

use sill_net::identity::DeviceIdentity;
use sill_net::pair_client::{self, SavedMac};
use sill_net::pairing::{proof, PairLink};
use sill_net::session::{CloseReason, Session, SessionConfig, SessionEvent};
use sill_proto::switcher::StreamSource;
use sill_proto::viewport::Viewport;
use sill_proto::MessageKind;

fn target_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target")
}

fn hostname() -> String {
    for var in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return v.trim().to_string();
            }
        }
    }
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "PC".into())
}

/// "<hostname> (Windows)" (plan D8).
fn device_name() -> String {
    format!("{} (Windows)", hostname())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let v: Option<Vec<u8>> = (0..64)
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect();
    v?.try_into().ok()
}

/// The device identity in a directory: `key.pk8` (PKCS#8 DER) and `cert.der`.
fn load_identity(dir: &Path) -> Result<DeviceIdentity, String> {
    let key_path = dir.join("key.pk8");
    if let Ok(der) = std::fs::read(&key_path) {
        return DeviceIdentity::from_pkcs8_der(&der).map_err(|e| e.to_string());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let id = DeviceIdentity::generate();
    std::fs::write(&key_path, id.to_pkcs8_der()).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
    }
    let cert = id.certificate_der().map_err(|e| e.to_string())?;
    std::fs::write(dir.join("cert.der"), cert).map_err(|e| e.to_string())?;
    Ok(id)
}

fn macs_path() -> PathBuf {
    target_dir().join("sill-dev-macs.json")
}

fn load_macs() -> Vec<SavedMac> {
    std::fs::read(macs_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_mac(mac: &SavedMac) -> Result<(), String> {
    let mut all = load_macs();
    all.retain(|m| m.mac_id != mac.mac_id);
    all.push(mac.clone());
    std::fs::create_dir_all(target_dir()).map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(&all).map_err(|e| e.to_string())?;
    std::fs::write(macs_path(), json).map_err(|e| e.to_string())
}

struct Args {
    flags: BTreeMap<String, String>,
    switches: Vec<String>,
}

fn parse(args: &[String]) -> Args {
    let mut flags = BTreeMap::new();
    let mut switches = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(name) = a.strip_prefix("--") {
            if let Some((k, v)) = name.split_once('=') {
                flags.insert(k.to_string(), v.to_string());
            } else if matches!(name, "dry-run" | "audio") {
                switches.push(name.to_string());
            } else if let Some(v) = args.get(i + 1) {
                flags.insert(name.to_string(), v.clone());
                i += 1;
            } else {
                die(&format!("--{name} needs a value"));
            }
        } else {
            die(&format!("unexpected argument {a:?}"));
        }
        i += 1;
    }
    Args { flags, switches }
}

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    exit(2)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first() else {
        die("usage: sill_cli identity | pair | session (see the file header)");
    };
    let args = parse(&argv[1..]);
    let identity_dir = args
        .flags
        .get("identity")
        .map(PathBuf::from)
        .unwrap_or_else(|| target_dir().join("sill-dev-identity"));
    match cmd.as_str() {
        "identity" => {
            let id = load_identity(&identity_dir).unwrap_or_else(|e| die(&e));
            println!("identity dir: {}", identity_dir.display());
            println!("device SPKI SHA-256: {}", hex(&id.fingerprint()));
            println!("device ID: {}", id.mac_id());
            println!("device name: {}", device_name());
        }
        "pair" => run_pair(&args, &identity_dir),
        "session" => run_session(&args, &identity_dir),
        other => die(&format!("unknown command {other:?}")),
    }
}

const MOCK_MAC_FP: [u8; 32] = [0x11; 32];
const MOCK_DEVICE_FP: [u8; 32] = [0x22; 32];

fn run_pair(args: &Args, identity_dir: &Path) {
    let name = args.flags.get("name").cloned().unwrap_or_else(device_name);
    let model = Some("Windows");
    let dry = args.switches.iter().any(|s| s == "dry-run");
    let timeout = Duration::from_secs(10);

    if dry {
        // No connection, no identity file: a mock Mac key (and device key unless a real identity is
        // asked for with --identity) so the output is reproducible.
        let mac_fp = args
            .flags
            .get("mock-mac-fp")
            .map(|h| unhex32(h).unwrap_or_else(|| die("--mock-mac-fp is 64 hex digits")))
            .unwrap_or(MOCK_MAC_FP);
        let device_fp = if args.flags.contains_key("identity") {
            load_identity(identity_dir)
                .unwrap_or_else(|e| die(&e))
                .fingerprint()
        } else {
            MOCK_DEVICE_FP
        };
        let (method, key, mac_fp) = if let Some(link) = args.flags.get("link") {
            let l = PairLink::parse(link).unwrap_or_else(|e| die(&format!("link: {e:?}")));
            let fp: [u8; 32] = l.fingerprint.clone().try_into().unwrap();
            ("qr", proof::qr_key(&l.secret), fp)
        } else {
            let code = args
                .flags
                .get("code")
                .unwrap_or_else(|| die("--code or --link"));
            let digits = sill_net::damm::check(code)
                .unwrap_or_else(|e| die(&format!("code refused locally: {e:?}")));
            let key = proof::code_key(&digits, &mac_fp).unwrap().to_vec();
            ("code", key, mac_fp)
        };
        let p = proof::device_proof(&key, &device_fp, &mac_fp);
        let req = pair_client::build_request(method, Some(&p), &name, model);
        let bytes = pair_client::request_bytes(&req, 0.0);
        println!("DRY RUN: nothing is sent, no connection is made.");
        println!("method:        {method}");
        println!("mock Mac fp:   {}", hex(&mac_fp));
        println!(
            "device fp:     {}{}",
            hex(&device_fp),
            if device_fp == MOCK_DEVICE_FP {
                " (mock)"
            } else {
                ""
            }
        );
        println!("key:           {}", hex(&key));
        println!("proof_D:       {}", hex(&p));
        println!(
            "kind 19 JSON:  {}",
            String::from_utf8_lossy(&bytes[sill_proto::HEADER_LEN..])
        );
        println!("header (14):   {}", hex(&bytes[..sill_proto::HEADER_LEN]));
        println!("bytes ({}):    {}", bytes.len(), hex(&bytes));
        println!("(timestamp is 0 in a dry run; a real send uses the wall clock)");
        return;
    }

    let identity = load_identity(identity_dir).unwrap_or_else(|e| die(&e));
    let started = Instant::now();
    let saved = if let Some(link) = args.flags.get("link") {
        println!("pairing by link (one try; the Mac allows five per code)");
        pair_client::pair_with_link(link, &identity, &name, model, timeout)
    } else {
        let addr = args
            .flags
            .get("addr")
            .unwrap_or_else(|| die("--addr HOST:PORT"));
        let code = args.flags.get("code").unwrap_or_else(|| die("--code"));
        println!("pairing by typed code to {addr} (one try; the Mac allows five per code)");
        pair_client::pair_with_code(addr, code, None, &identity, &name, model, timeout)
    };
    match saved {
        Ok(mac) => {
            println!(
                "PAIRED in {:.1} s: {} ({})",
                started.elapsed().as_secs_f64(),
                mac.name,
                mac.mac_id
            );
            println!(
                "Mac SPKI SHA-256 (pin): {}",
                hex(&mac.pin().unwrap_or([0; 32]))
            );
            if let Err(e) = save_mac(&mac) {
                die(&format!("paired, but the record could not be saved: {e}"));
            }
            println!("saved to {}", macs_path().display());
        }
        Err(e) => {
            eprintln!("PAIR FAILED: {e}");
            exit(1);
        }
    }
}

fn run_session(args: &Args, identity_dir: &Path) {
    let want = args
        .flags
        .get("mac")
        .unwrap_or_else(|| die("--mac MACID"))
        .to_uppercase();
    let seconds: f64 = args
        .flags
        .get("seconds")
        .map(|s| s.parse().unwrap_or_else(|_| die("--seconds N")))
        .unwrap_or(10.0);
    let pick = args.flags.get("pick").map(|p| match p.as_str() {
        "desktop" => StreamSource::Desktop,
        "none" => StreamSource::None,
        w if w.starts_with("window:") => StreamSource::Window(
            w[7..]
                .parse()
                .unwrap_or_else(|_| die("window:ID needs a number")),
        ),
        _ => die("--pick desktop|none|window:ID"),
    });
    let mac = load_macs()
        .into_iter()
        .find(|m| m.mac_id.starts_with(&want))
        .unwrap_or_else(|| die(&format!("no saved Mac {want} in {}", macs_path().display())));
    let identity = load_identity(identity_dir).unwrap_or_else(|e| die(&e));
    let pin = mac
        .pin()
        .unwrap_or_else(|| die("saved fingerprint is damaged"));

    let mut session = None;
    for target in mac.dial_targets() {
        let cfg = SessionConfig {
            target: target.clone(),
            pin,
            device_name: device_name(),
            audio: args.switches.iter().any(|s| s == "audio"),
            connect_timeout: Duration::from_secs(10),
        };
        println!(
            "dialing {target} (TLS 1.3, ALPN sill/1, pinned to {})",
            mac.mac_id
        );
        match Session::open(&cfg, &identity) {
            Ok(s) => {
                session = Some(s);
                break;
            }
            Err(e) => println!("  failed: {e}"),
        }
    }
    let Some(session) = session else {
        eprintln!("no address answered");
        exit(1);
    };
    println!(
        "hello sent first: {}",
        serde_json::to_string(&session.hello).unwrap()
    );
    if let Some(src) = pick {
        // 1280x720 at 60, no scale: the Mac leaves its windows alone.
        let vp = Viewport {
            width: 1280.0,
            height: 720.0,
            scale: None,
            local_cursor: None,
            fps: Some(60),
        };
        println!("sending viewport 1280x720 and pick {src:?}");
        session.send_viewport(&vp);
        session.select_source(src);
    }

    let t0 = Instant::now();
    let mut next_report = t0 + Duration::from_secs(1);
    let (mut frames, mut keyframes, mut bytes, mut sets) = (0u64, 0u64, 0usize, 0u64);
    let mut other: BTreeMap<u8, (u64, usize)> = BTreeMap::new();
    let mut worst_rtt = 0f64;
    while t0.elapsed().as_secs_f64() < seconds {
        match session.events.recv_timeout(Duration::from_millis(100)) {
            Ok(ev) => match ev {
                SessionEvent::WindowList(l) => {
                    let titles: Vec<String> = l
                        .windows
                        .iter()
                        .take(5)
                        .map(|w| {
                            let t: String = w.title.chars().take(24).collect();
                            format!("{}:{}", w.id, t)
                        })
                        .collect();
                    println!(
                        "kind 2  window list: Mac {:?}, {} windows, active {:?}, host {:?} proto {:?}, locked {:?}; first: {}",
                        l.mac_name, l.windows.len(), l.active, l.host_version, l.protocol, l.locked, titles.join(" | ")
                    );
                }
                SessionEvent::HostSettings(s) => println!("kind 16 host settings: {}", summary(&s)),
                SessionEvent::MacInfo {
                    verified,
                    problem,
                    info,
                } => println!(
                    "kind 18 mac info: verified {}{}{}",
                    if verified { "yes" } else { "NO" },
                    problem.map(|p| format!(" ({p})")).unwrap_or_default(),
                    info.map(|i| format!(
                        ", Mac {:?}, remote port {}, {} addresses",
                        i.name,
                        i.remote_port,
                        i.addresses.len()
                    ))
                    .unwrap_or_default()
                ),
                SessionEvent::Goodbye(g) => println!(
                    "kind 22 goodbye: reason {:?}, message {:?}, reconnect {:?}",
                    g.reason, g.message, g.reconnect
                ),
                SessionEvent::Pong { rtt_ms } => worst_rtt = worst_rtt.max(rtt_ms),
                SessionEvent::Message(m) => match m.kind {
                    MessageKind::Frame => {
                        frames += 1;
                        keyframes += u64::from(m.is_keyframe);
                        bytes += m.payload.len();
                    }
                    MessageKind::ParameterSets => sets += 1,
                    k => {
                        let e = other.entry(k.as_u8()).or_default();
                        e.0 += 1;
                        e.1 += m.payload.len();
                    }
                },
                SessionEvent::Closed(r) => {
                    println!("session closed: {}", close_words(&r));
                    return;
                }
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
        if Instant::now() >= next_report {
            println!(
                "t={:>4.1}s frames {} (key {}) {} KB, param sets {}, worst rtt {:.0} ms; other kinds {:?}",
                t0.elapsed().as_secs_f64(), frames, keyframes, bytes / 1024, sets, worst_rtt,
                other.iter().map(|(k, (n, b))| format!("{k}x{n}/{b}B")).collect::<Vec<_>>()
            );
            (frames, keyframes, bytes, sets, worst_rtt) = (0, 0, 0, 0, 0.0);
            other.clear();
            next_report += Duration::from_secs(1);
        }
    }
    println!("done after {seconds} s; closing");
    session.close();
}

fn summary(s: &sill_proto::settings::HostSettingsState) -> String {
    let j = serde_json::to_string(s).unwrap_or_default();
    j.chars().take(160).collect()
}

fn close_words(r: &CloseReason) -> String {
    match r {
        CloseReason::Goodbye(w) => format!("the Mac said goodbye ({w})"),
        CloseReason::Eof { served: false } => {
            "the Mac closed the connection before sending anything: it refused this device (not paired, or removed)".into()
        }
        CloseReason::Eof { served: true } => "the Mac closed the connection".into(),
        CloseReason::Lost(s) => format!("lost: no byte for {s:.1} s"),
        CloseReason::Error(e) => format!("error: {e}"),
        CloseReason::Requested => "closed".into(),
    }
}
