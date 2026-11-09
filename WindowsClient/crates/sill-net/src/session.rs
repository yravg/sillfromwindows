//! A session with a paired Mac (W-17): TLS 1.3 ALPN `sill/1`, pinned to the saved Mac key, with the
//! device's identity; hello first; then one pump thread reads the Mac's messages, pings every
//! 0.25 s, sends stats every second and watches liveness (docs/windows-client/protocol-spec.md
//! section 5; iOSClient/StreamClient.swift, MessageReader.swift; Scripts/sillclient.py).
//!
//! Blocking `std::net` + `rustls::StreamOwned`. Everything that touches the socket happens on the
//! pump thread, so no lock is held across a blocking read: other threads hand messages in through
//! [`Session::send`] and read events from [`Session::events`].

use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustls::{ClientConnection, StreamOwned};
use sill_proto::compat::{Hello, PROTOCOL_CURRENT};
use sill_proto::remote::{Goodbye, MacInfo, SignedMacInfo};
use sill_proto::settings::HostSettingsState;
use sill_proto::switcher::{StreamSource, WindowList};
use sill_proto::viewport::{ClientStats, Viewport};
use sill_proto::{wire, Caps, Message, MessageKind, MessageReader};

use crate::identity::DeviceIdentity;
use crate::pairing::{verify_signed_mac_info, MacInfoFailure};
use crate::tls::{self, Observed};

/// The device's wall clock for a message header (sillclient.py `msg`: `time.time()`).
pub fn now_timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// --- dialling ------------------------------------------------------------------------------------

/// Why a TLS connection to a door could not be made.
#[derive(Debug)]
pub enum DialError {
    /// The address did not resolve.
    Resolve(String),
    /// TCP connect failed or timed out.
    Connect(String),
    /// The Mac presented a key other than the pinned one; nothing was sent.
    PinMismatch,
    /// The handshake failed for another reason (an alert, a TLS version, a timeout).
    Tls(String),
    /// The Mac negotiated another protocol than the one offered.
    Alpn(Option<String>),
    /// The device certificate or key was unusable.
    Identity(String),
}

impl fmt::Display for DialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DialError::Resolve(s) => write!(f, "address did not resolve: {s}"),
            DialError::Connect(s) => write!(f, "could not connect: {s}"),
            DialError::PinMismatch => {
                write!(f, "the Mac presented another key than the pinned one")
            }
            DialError::Tls(s) => write!(f, "TLS handshake failed: {s}"),
            DialError::Alpn(p) => write!(f, "unexpected ALPN {p:?}"),
            DialError::Identity(s) => write!(f, "device identity: {s}"),
        }
    }
}
impl std::error::Error for DialError {}

/// A finished handshake.
pub struct Dialed {
    pub stream: StreamOwned<ClientConnection, TcpStream>,
    pub observed: Observed,
    /// The Mac's key as this TLS session saw it (never taken from a message).
    pub mac_fingerprint: [u8; 32],
}

/// TCP and TLS 1.3 to `target` (`host:port`; an IPv6 address in brackets) with one ALPN and the
/// device certificate. `pin` is the Mac's SPKI SHA-256, or None to accept any P-256 key (pairing).
/// `timeout` bounds each connect attempt and every blocking step of the handshake.
pub fn dial(
    target: &str,
    alpn: &str,
    identity: &DeviceIdentity,
    pin: Option<[u8; 32]>,
    timeout: Duration,
) -> Result<Dialed, DialError> {
    let cert = identity
        .certificate_der()
        .map_err(|e| DialError::Identity(e.to_string()))?;
    let (config, observed) = tls::client_config(alpn, cert, identity.to_pkcs8_der(), pin)
        .map_err(|e| DialError::Identity(e.to_string()))?;
    let addrs: Vec<SocketAddr> = target
        .to_socket_addrs()
        .map_err(|e| DialError::Resolve(format!("{target}: {e}")))?
        .collect();
    let mut last = format!("{target}: no address");
    let mut tcp = None;
    for a in addrs {
        match TcpStream::connect_timeout(&a, timeout) {
            Ok(s) => {
                tcp = Some(s);
                break;
            }
            Err(e) => last = format!("{a}: {e}"),
        }
    }
    let mut tcp = tcp.ok_or(DialError::Connect(last))?;
    let _ = tcp.set_nodelay(true);
    let _ = tcp.set_read_timeout(Some(timeout));
    let _ = tcp.set_write_timeout(Some(timeout));
    let mut conn = ClientConnection::new(config, tls::server_name())
        .map_err(|e| DialError::Tls(e.to_string()))?;
    while conn.is_handshaking() {
        if let Err(e) = conn.complete_io(&mut tcp) {
            let seen = observed.snapshot();
            let text = e.to_string();
            let pinned_wrong =
                pin.is_some() && seen.spki_sha256.is_some() && seen.spki_sha256 != pin
                    || text.contains("ApplicationVerificationFailure");
            return Err(if pinned_wrong {
                DialError::PinMismatch
            } else {
                DialError::Tls(text)
            });
        }
    }
    let got = conn
        .alpn_protocol()
        .map(|p| String::from_utf8_lossy(p).into_owned());
    if got.as_deref() != Some(alpn) {
        return Err(DialError::Alpn(got));
    }
    let seen = observed.snapshot();
    let mac_fingerprint = seen
        .spki_sha256
        .ok_or_else(|| DialError::Tls("the Mac's key is not P-256".into()))?;
    Ok(Dialed {
        stream: StreamOwned::new(conn, tcp),
        observed: seen,
        mac_fingerprint,
    })
}

// --- the session ---------------------------------------------------------------------------------

/// What to connect to.
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// `host:port` of the Mac's door (remote door 7455 or the home door's Bonjour port).
    pub target: String,
    /// The Mac's SPKI SHA-256 from pairing.
    pub pin: [u8; 32],
    /// The name in hello and stats ("<hostname> (Windows)").
    pub device_name: String,
    /// List `aac-eld` in hello so the host sends sound.
    pub audio: bool,
    pub connect_timeout: Duration,
}

/// One event from the Mac, in arrival order.
#[derive(Debug)]
pub enum SessionEvent {
    WindowList(WindowList),
    HostSettings(HostSettingsState),
    /// Kind 18. `verified` is the signature and Mac ID check AND the saved pin matching the signer.
    MacInfo {
        verified: bool,
        problem: Option<String>,
        info: Option<MacInfo>,
    },
    /// Kind 22. The session ends right after.
    Goodbye(Goodbye),
    /// A pong: the round trip of one ping in milliseconds.
    Pong {
        rtt_ms: f64,
    },
    /// Every other message (video, thumbnails, icons, cursor shape, pointer, audio, ...), whole.
    /// Unknown kinds arrive here too: the caller skips them.
    Message(Message),
    /// Always the last event.
    Closed(CloseReason),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CloseReason {
    /// The Mac sent kind 22 (its words are in the Goodbye event).
    Goodbye(String),
    /// The connection ended. `served` is false when nothing at all had arrived: for a paired key
    /// that means the Mac refused this device (not paired, or removed).
    Eof { served: bool },
    /// No byte arrived for this many seconds (max(6 s, 4 x worst round trip)).
    Lost(f64),
    /// A read or write error, or a message over the caps.
    Error(String),
    /// [`Session::close`].
    Requested,
}

enum Command {
    Send(Vec<u8>),
    Close,
}

/// An open session. Dropping it closes the connection.
pub struct Session {
    pub events: Receiver<SessionEvent>,
    commands: Sender<Command>,
    join: Option<JoinHandle<()>>,
    /// The Mac's key from the handshake (equals the pin).
    pub mac_fingerprint: [u8; 32],
    /// The hello that was sent first.
    pub hello: Hello,
}

/// The hello a device sends (Compatibility.swift:104-124).
pub fn make_hello(device_name: &str, audio: bool) -> Hello {
    Hello {
        app_version: Some(env!("CARGO_PKG_VERSION").into()),
        build: Some("1".into()),
        protocol: Some(PROTOCOL_CURRENT),
        device: Some(device_name.into()),
        audio: audio.then(|| vec!["aac-eld".to_string()]),
    }
}

fn message(kind: MessageKind, payload: Vec<u8>) -> Vec<u8> {
    Message::new(kind, now_timestamp(), false, payload)
        .serialize()
        .expect("payload fits a header")
}

impl Session {
    /// Connects, sends the hello first, starts the pump. Fails before any message when the TLS
    /// handshake does (a pin mismatch sends nothing at all).
    pub fn open(config: &SessionConfig, identity: &DeviceIdentity) -> Result<Session, DialError> {
        let mut dialed = dial(
            &config.target,
            tls::SESSION_ALPN,
            identity,
            Some(config.pin),
            config.connect_timeout,
        )?;
        let hello = make_hello(&config.device_name, config.audio);
        dialed
            .stream
            .write_all(&message(MessageKind::Hello, wire::encode(&hello)))
            .and_then(|_| dialed.stream.flush())
            .map_err(|e| DialError::Tls(format!("hello not sent: {e}")))?;
        let (ev_tx, ev_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let pin = config.pin;
        let name = config.device_name.clone();
        let stream = dialed.stream;
        let join = std::thread::Builder::new()
            .name("sill-session".into())
            .spawn(move || pump(stream, pin, name, cmd_rx, ev_tx))
            .map_err(|e| DialError::Tls(e.to_string()))?;
        Ok(Session {
            events: ev_rx,
            commands: cmd_tx,
            join: Some(join),
            mac_fingerprint: dialed.mac_fingerprint,
            hello,
        })
    }

    /// Queues a message for the Mac (false once the session ended).
    pub fn send(&self, kind: MessageKind, payload: Vec<u8>) -> bool {
        self.commands
            .send(Command::Send(message(kind, payload)))
            .is_ok()
    }

    /// Kind 9. Held to what a host takes (1 to 16384 points a side, scale 0.25 to 4): a viewport
    /// outside is not sent (the host would refuse it whole); returns false then.
    pub fn send_viewport(&self, viewport: &Viewport) -> bool {
        if sill_proto::viewport::ViewportBounds::refusal(viewport).is_some() {
            return false;
        }
        self.send(MessageKind::Viewport, wire::encode(viewport))
    }

    /// Kind 6: pick the Desktop, a window or nothing.
    pub fn select_source(&self, source: StreamSource) -> bool {
        self.send(MessageKind::SelectSource, wire::encode(&source))
    }

    /// Kind 44, empty (at most one a second reaches the host's encoder).
    pub fn request_keyframe(&self) -> bool {
        self.send(MessageKind::KeyframeRequest, Vec::new())
    }

    /// Ends the session and waits for the pump.
    pub fn close(mut self) {
        let _ = self.commands.send(Command::Close);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Close);
    }
}

/// Liveness of a session (StreamClient.swift: no byte for max(6 s, 4 x the worst recent round trip)).
pub fn liveness_limit(worst_rtt: Duration) -> Duration {
    Duration::from_secs(6).max(worst_rtt * 4)
}

const PING_EVERY: Duration = Duration::from_millis(250);
const STATS_EVERY: Duration = Duration::from_secs(1);
const READ_SLICE: Duration = Duration::from_millis(20);

fn pump(
    mut stream: StreamOwned<ClientConnection, TcpStream>,
    pin: [u8; 32],
    device: String,
    commands: Receiver<Command>,
    events: Sender<SessionEvent>,
) {
    let _ = stream.sock.set_read_timeout(Some(READ_SLICE));
    let _ = stream.sock.set_write_timeout(Some(Duration::from_secs(5)));
    let start = Instant::now();
    let mut reader = MessageReader::new(Caps::DEVICE);
    let mut buf = vec![0u8; 64 * 1024];
    let mut last_byte = Instant::now();
    let mut next_ping = Instant::now();
    let mut next_stats = Instant::now() + STATS_EVERY;
    let mut served = false;
    let mut frames_this_second = 0i64;
    let mut rtt_window_max = 0f64; // this second's worst
    let mut worst_rtt = Duration::ZERO; // the last second that measured one
    let mut last_rtt_ms = -1i64;
    let mut last_macinfo_issued = f64::NEG_INFINITY;

    let reason = 'outer: loop {
        // commands from other threads
        loop {
            match commands.try_recv() {
                Ok(Command::Send(bytes)) => {
                    if let Err(e) = stream.write_all(&bytes).and_then(|_| stream.flush()) {
                        break 'outer CloseReason::Error(format!("write: {e}"));
                    }
                }
                Ok(Command::Close) | Err(mpsc::TryRecvError::Disconnected) => {
                    break 'outer CloseReason::Requested;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        let now = Instant::now();
        if now >= next_ping {
            // 8-byte payload the host echoes: this device's monotonic seconds, a big-endian Double.
            let payload = start.elapsed().as_secs_f64().to_be_bytes().to_vec();
            if let Err(e) = stream
                .write_all(&message(MessageKind::Ping, payload))
                .and_then(|_| stream.flush())
            {
                break 'outer CloseReason::Error(format!("write: {e}"));
            }
            next_ping = now + PING_EVERY;
        }
        if now >= next_stats {
            let stats = ClientStats {
                fps: frames_this_second,
                frame_age_ms: -1, // no sample: this client does not measure it yet
                rtt_ms: last_rtt_ms,
                device: device.clone(),
                frame_age_max_ms: None,
                rtt_max_ms: (rtt_window_max > 0.0).then(|| rtt_window_max.round() as i64),
                audio_behind_ms: None,
                audio_late: None,
            };
            if rtt_window_max > 0.0 {
                worst_rtt = Duration::from_secs_f64(rtt_window_max / 1000.0);
            }
            frames_this_second = 0;
            rtt_window_max = 0.0;
            if let Err(e) = stream
                .write_all(&message(MessageKind::ClientStats, wire::encode(&stats)))
                .and_then(|_| stream.flush())
            {
                break 'outer CloseReason::Error(format!("write: {e}"));
            }
            next_stats = now + STATS_EVERY;
        }
        if last_byte.elapsed() > liveness_limit(worst_rtt) {
            break 'outer CloseReason::Lost(last_byte.elapsed().as_secs_f64());
        }
        match stream.read(&mut buf) {
            Ok(0) => break 'outer CloseReason::Eof { served },
            Ok(n) => {
                last_byte = Instant::now();
                reader.push(&buf[..n]);
                loop {
                    let m = match reader.next_message() {
                        Ok(Some(m)) => m,
                        Ok(None) => break,
                        Err(e) => break 'outer CloseReason::Error(e.to_string()),
                    };
                    served = true;
                    let ev = match m.kind {
                        MessageKind::WindowList => wire::decode_object(&m.payload)
                            .map(SessionEvent::WindowList)
                            .unwrap_or(SessionEvent::Message(m)),
                        MessageKind::HostSettings => wire::decode_object(&m.payload)
                            .map(SessionEvent::HostSettings)
                            .unwrap_or(SessionEvent::Message(m)),
                        MessageKind::MacInfo => {
                            mac_info_event(&m.payload, &pin, &mut last_macinfo_issued)
                        }
                        MessageKind::Goodbye => match wire::decode_object::<Goodbye>(&m.payload) {
                            Some(g) => {
                                let why = g.reason.clone();
                                let _ = events.send(SessionEvent::Goodbye(g));
                                break 'outer CloseReason::Goodbye(why);
                            }
                            None => SessionEvent::Message(m),
                        },
                        MessageKind::Pong => {
                            if let Ok(b) = <[u8; 8]>::try_from(m.payload.as_slice()) {
                                let sent = f64::from_be_bytes(b);
                                let rtt = (start.elapsed().as_secs_f64() - sent) * 1000.0;
                                if rtt.is_finite() && rtt >= 0.0 {
                                    last_rtt_ms = rtt.round() as i64;
                                    rtt_window_max = rtt_window_max.max(rtt);
                                    SessionEvent::Pong { rtt_ms: rtt }
                                } else {
                                    continue;
                                }
                            } else {
                                continue;
                            }
                        }
                        MessageKind::Tick => continue,
                        MessageKind::Frame => {
                            frames_this_second += 1;
                            SessionEvent::Message(m)
                        }
                        _ => SessionEvent::Message(m),
                    };
                    if events.send(ev).is_err() {
                        break 'outer CloseReason::Requested; // nobody listens any more
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) =>
            {
                continue; // nothing within the slice; commands and timers run at the loop's top
            }
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                break 'outer CloseReason::Eof { served }
            }
            Err(e) => break 'outer CloseReason::Error(format!("read: {e}")),
        }
    };
    stream.conn.send_close_notify();
    let _ = stream.flush();
    let _ = stream.sock.shutdown(std::net::Shutdown::Both);
    let _ = events.send(SessionEvent::Closed(reason));
}

/// Kind 18: verify (signature over the exact info bytes, Mac ID against the key), then require the
/// signer to be the saved Mac and `issuedAt` not older than the last one seen on this session.
fn mac_info_event(payload: &[u8], pin: &[u8; 32], last_issued: &mut f64) -> SessionEvent {
    let Some(signed) = wire::decode_object::<SignedMacInfo>(payload) else {
        return SessionEvent::MacInfo {
            verified: false,
            problem: Some("not a SignedMacInfo".into()),
            info: None,
        };
    };
    match verify_signed_mac_info(&signed.info, &signed.key, &signed.sig) {
        Err(f) => SessionEvent::MacInfo {
            verified: false,
            problem: Some(
                match f {
                    MacInfoFailure::Malformed => "malformed",
                    MacInfoFailure::BadSignature => "bad signature",
                    MacInfoFailure::BadInfo => "bad info",
                    MacInfoFailure::MacIdMismatch => "Mac ID is not the signer's",
                }
                .into(),
            ),
            info: signed.unverified_info(),
        },
        Ok(v) => {
            let info: Option<MacInfo> = wire::decode_object(&v.info_json);
            let mut problem = None;
            if &v.fingerprint != pin {
                problem = Some("signed by another key than the saved Mac".to_string());
            } else if let Some(i) = &info {
                if i.issued_at < *last_issued {
                    problem = Some("older than the last kind 18".to_string());
                } else {
                    *last_issued = i.issued_at;
                }
            }
            SessionEvent::MacInfo {
                verified: problem.is_none(),
                problem,
                info,
            }
        }
    }
}
