//! Pairing with a Mac as a device (W-15): one `sill-pair/1` connection per exchange, exactly one
//! kind 19 out, one kind 20 back (docs/windows-client/protocol-spec.md section 4; Pairing.swift,
//! StreamClient+Home.swift, Scripts/sillclient.py `pair` and `ask`).
//!
//! Trust: the Mac's key is the one THIS TLS session saw, never one a message names; the proofs bind
//! both keys, so a man in the middle cannot make a valid one. The typed path's PBKDF2 (600,000 rounds)
//! can only start once the handshake has shown the Mac's key, and the host allows 4 s from TLS ready
//! to the kind 19: use an optimized build (the workspace builds dependencies at opt-level 3).

use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sill_proto::remote::{PairRequest, PairResult};
use sill_proto::{wire, Caps, Message, MessageKind, MessageReader};

use crate::damm;
use crate::identity::{base64url_decode, base64url_encode, mac_id, DeviceIdentity};
use crate::pairing::{proof, safe_label, LinkFailure, PairLink, LABEL_LIMIT};
use crate::session::{dial, now_timestamp, DialError};
use crate::tls::PAIRING_ALPN;

/// How long a device waits for kind 20 (RemoteDialPolicy.pairingReplyDeadline).
pub const REPLY_DEADLINE: Duration = Duration::from_secs(15);

/// What pairing saves about a Mac (StreamClient+Home.swift:949-957).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedMac {
    pub name: String,
    #[serde(rename = "macID")]
    pub mac_id: String,
    /// The Mac's SPKI SHA-256, base64url without padding: the pin.
    pub fingerprint: String,
    /// `host` or `host:port` (IPv6 in brackets), in dial order.
    pub addresses: Vec<String>,
    /// The port that applies to an address with none of its own.
    pub port: u16,
    /// base64url of the 32-byte recognition key (for the Bonjour tag), when the Mac sent one.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "recognitionKey"
    )]
    pub recognition_key: Option<String>,
    /// "code" or "qr".
    pub method: String,
}

impl SavedMac {
    /// The pin as bytes.
    pub fn pin(&self) -> Option<[u8; 32]> {
        base64url_decode(&self.fingerprint)?.try_into().ok()
    }

    /// Every `host:port` to dial, in order.
    pub fn dial_targets(&self) -> Vec<String> {
        self.addresses
            .iter()
            .map(|a| {
                let has_port = if let Some(rest) = a.strip_prefix('[') {
                    rest.contains("]:")
                } else {
                    a.matches(':').count() == 1
                };
                if has_port {
                    a.clone()
                } else if a.contains(':') && !a.starts_with('[') {
                    format!("[{a}]:{}", self.port)
                } else {
                    format!("{a}:{}", self.port)
                }
            })
            .collect()
    }
}

/// The Mac's refusal of a pairing exchange: every `PairResult` reason, typed.
#[derive(Debug, Clone, PartialEq)]
pub enum PairRefusal {
    /// "code": the proof was wrong (a typo that passed the check digit, or a wrong Mac).
    WrongCode { tries_left: Option<i64> },
    /// "closed": no pairing window is open (or the door does not take this).
    Closed,
    /// "expired": the code's five minutes are over.
    Expired,
    /// "stopped": the Mac closed the window.
    Stopped,
    /// "busy": try again after `retry_after` seconds.
    Busy { retry_after: Option<f64> },
    /// "shown" (an ask's answer): the Mac now shows a code.
    Shown,
    /// "openOnMac": a pairing window is already open on the Mac.
    OpenOnMac,
    /// "locked": the Mac is locked.
    Locked,
    /// A reason this build does not know; `message` is the Mac's words (SafeText, 300 characters).
    Other {
        reason: Option<String>,
        message: Option<String>,
    },
}

impl PairRefusal {
    /// Maps a refused kind 20 (`ok` false) to its reason.
    pub fn from_result(r: &PairResult) -> PairRefusal {
        match r.reason.as_deref() {
            Some(PairResult::CODE) => PairRefusal::WrongCode {
                tries_left: r.tries_left,
            },
            Some(PairResult::CLOSED) => PairRefusal::Closed,
            Some(PairResult::EXPIRED) => PairRefusal::Expired,
            Some(PairResult::STOPPED) => PairRefusal::Stopped,
            Some(PairResult::BUSY) => PairRefusal::Busy {
                retry_after: r.retry_after,
            },
            Some(PairResult::SHOWN) => PairRefusal::Shown,
            Some(PairResult::OPEN_ON_MAC) => PairRefusal::OpenOnMac,
            Some(PairResult::LOCKED) => PairRefusal::Locked,
            other => PairRefusal::Other {
                reason: other.map(str::to_string),
                message: r.message.as_deref().map(|m| safe_label(m, 300)),
            },
        }
    }
}

impl fmt::Display for PairRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PairRefusal::WrongCode {
                tries_left: Some(n),
            } => {
                write!(f, "that code did not match ({n} tries left)")
            }
            PairRefusal::WrongCode { tries_left: None } => write!(f, "that code did not match"),
            PairRefusal::Closed => write!(f, "the Mac has no pairing window open"),
            PairRefusal::Expired => write!(f, "that code has expired"),
            PairRefusal::Stopped => write!(f, "the Mac stopped pairing"),
            PairRefusal::Busy {
                retry_after: Some(s),
            } => write!(f, "the Mac is busy, retry in {s} s"),
            PairRefusal::Busy { retry_after: None } => write!(f, "the Mac is busy"),
            PairRefusal::Shown => write!(f, "the Mac is showing a code"),
            PairRefusal::OpenOnMac => write!(f, "a pairing window is already open on the Mac"),
            PairRefusal::Locked => write!(f, "the Mac is locked"),
            PairRefusal::Other { reason, message } => write!(
                f,
                "the Mac refused ({}): {}",
                reason.as_deref().unwrap_or("no reason"),
                message.as_deref().unwrap_or("no message")
            ),
        }
    }
}

#[derive(Debug)]
pub enum PairError {
    /// The typed code is not 12 digits with a valid check digit; nothing was sent.
    BadCode(damm::Problem),
    /// The link did not parse; nothing was sent.
    BadLink(LinkFailure),
    /// The link carries no address and none was given.
    NoAddress,
    Dial(DialError),
    /// Writing kind 19 or reading kind 20 failed or timed out.
    Io(String),
    /// The connection ended without a kind 20.
    NoReply,
    /// The Mac refused.
    Refused(PairRefusal),
    /// A kind 20 ok whose proof does not check: nothing was saved.
    BadMacProof,
    /// The ok's macID is not the Mac ID of the key this session saw.
    MacIdMismatch,
    /// Anything else wrong with the answer.
    Protocol(String),
}

impl fmt::Display for PairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PairError::BadCode(damm::Problem::Length) => write!(f, "a code has 12 digits"),
            PairError::BadCode(damm::Problem::Typo) => {
                write!(f, "that code has a typo; check it against your Mac")
            }
            PairError::BadLink(l) => write!(f, "not a usable pairing link: {l:?}"),
            PairError::NoAddress => write!(f, "no address to dial"),
            PairError::Dial(e) => write!(f, "{e}"),
            PairError::Io(s) => write!(f, "I/O: {s}"),
            PairError::NoReply => write!(f, "the Mac closed the connection without answering"),
            PairError::Refused(r) => write!(f, "{r}"),
            PairError::BadMacProof => write!(f, "the Mac's proof does not check; nothing saved"),
            PairError::MacIdMismatch => write!(f, "the Mac's ID is not its key's; nothing saved"),
            PairError::Protocol(s) => write!(f, "protocol: {s}"),
        }
    }
}
impl std::error::Error for PairError {}

impl From<DialError> for PairError {
    fn from(e: DialError) -> Self {
        PairError::Dial(e)
    }
}

// --- the messages (pure) -------------------------------------------------------------------------

/// Kind 19's payload. `proof_bytes` None is an ask or cancel (proof ""). `cable` is never sent.
pub fn build_request(
    method: &str,
    proof_bytes: Option<&[u8; 32]>,
    name: &str,
    model: Option<&str>,
) -> PairRequest {
    PairRequest::new(
        method,
        &proof_bytes.map(|p| base64url_encode(p)).unwrap_or_default(),
        &safe_label(name, LABEL_LIMIT),
        model,
    )
}

/// The whole kind 19 message: 14-byte header (kind 19, wall clock `timestamp`, no key flag, length)
/// and the compact JSON.
pub fn request_bytes(request: &PairRequest, timestamp: f64) -> Vec<u8> {
    Message::new(
        MessageKind::PairRequest,
        timestamp,
        false,
        wire::encode(request),
    )
    .serialize()
    .expect("a pair request fits a header")
}

/// What a typed-code exchange sends, for tests and `--dry-run`: PBKDF2 key, proof, request, bytes.
pub fn code_request_bytes(
    digits: &str,
    device_fp: &[u8; 32],
    mac_fp: &[u8; 32],
    name: &str,
    model: Option<&str>,
    timestamp: f64,
) -> Option<Vec<u8>> {
    let key = proof::code_key(digits, mac_fp)?;
    let p = proof::device_proof(&key, device_fp, mac_fp);
    Some(request_bytes(
        &build_request(PairRequest::CODE, Some(&p), name, model),
        timestamp,
    ))
}

// --- the exchange --------------------------------------------------------------------------------

enum How<'a> {
    Code(&'a str),
    Qr(&'a [u8]),
    Ask,
}

struct Exchange {
    result: PairResult,
    mac_fp: [u8; 32],
    key: Option<Vec<u8>>,
    device_fp: [u8; 32],
}

fn exchange(
    target: &str,
    identity: &DeviceIdentity,
    pin: Option<[u8; 32]>,
    how: How,
    name: &str,
    model: Option<&str>,
    timeout: Duration,
) -> Result<Exchange, PairError> {
    let mut d = dial(target, PAIRING_ALPN, identity, pin, timeout)?;
    let mac_fp = d.mac_fingerprint;
    let device_fp = identity.fingerprint();
    // Key and proof from THIS session's Mac key, straight after the handshake (the host's 4 s).
    let (method, key) = match how {
        How::Code(digits) => (
            PairRequest::CODE,
            Some(
                proof::code_key(digits, &mac_fp)
                    .ok_or(PairError::BadCode(damm::Problem::Length))?
                    .to_vec(),
            ),
        ),
        How::Qr(secret) => (PairRequest::QR, Some(proof::qr_key(secret))),
        How::Ask => (PairRequest::ASK, None),
    };
    let device_proof = key
        .as_ref()
        .map(|k| proof::device_proof(k, &device_fp, &mac_fp));
    let request = build_request(method, device_proof.as_ref(), name, model);
    d.stream
        .write_all(&request_bytes(&request, now_timestamp()))
        .and_then(|_| d.stream.flush())
        .map_err(|e| PairError::Io(format!("kind 19 not sent: {e}")))?;
    let result = read_result(&mut d.stream, timeout)?;
    Ok(Exchange {
        result,
        mac_fp,
        key,
        device_fp,
    })
}

fn read_result<S: Read>(stream: &mut S, _timeout: Duration) -> Result<PairResult, PairError> {
    // The socket's read timeout was set by `dial` to the connect timeout; the reply deadline is
    // checked here as a whole.
    let deadline = Instant::now() + REPLY_DEADLINE;
    let mut reader = MessageReader::new(Caps::DEVICE);
    let mut buf = [0u8; 4096];
    loop {
        while let Some(m) = reader
            .next_message()
            .map_err(|e| PairError::Protocol(e.to_string()))?
        {
            if m.kind == MessageKind::PairResult {
                return wire::decode_object::<PairResult>(&m.payload)
                    .ok_or_else(|| PairError::Protocol("kind 20 does not decode".into()));
            }
            // anything else is skipped
        }
        if Instant::now() > deadline {
            return Err(PairError::Io("no kind 20 within 15 s".into()));
        }
        match stream.read(&mut buf) {
            Ok(0) => return Err(PairError::NoReply),
            Ok(n) => reader.push(&buf[..n]),
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Err(PairError::NoReply),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => return Err(PairError::Io(format!("read: {e}"))),
        }
    }
}

/// An ok result checked and turned into the record to save: proof_M against the same key, macID
/// against the key this session saw.
fn saved_from_ok(
    ex: &Exchange,
    method: &str,
    addresses: Vec<String>,
    port: u16,
) -> Result<SavedMac, PairError> {
    let r = &ex.result;
    let key = ex
        .key
        .as_ref()
        .ok_or_else(|| PairError::Protocol("an ok to a request that sent no proof".into()))?;
    let mac_proof = r
        .proof
        .as_deref()
        .and_then(base64url_decode)
        .ok_or(PairError::BadMacProof)?;
    if !proof::is_valid_mac_proof(&mac_proof, key, &ex.mac_fp, &ex.device_fp) {
        return Err(PairError::BadMacProof);
    }
    let id = mac_id(&ex.mac_fp);
    if r.mac_id.as_deref() != Some(id.as_str()) {
        return Err(PairError::MacIdMismatch);
    }
    let name = r
        .name
        .as_deref()
        .map(|n| safe_label(n, crate::pairing::NAME_LIMIT))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Mac".into());
    Ok(SavedMac {
        name,
        mac_id: id,
        fingerprint: base64url_encode(&ex.mac_fp),
        addresses,
        port,
        recognition_key: r.recognition_key.clone(),
        method: method.into(),
    })
}

fn split_host_port(target: &str, default_port: u16) -> (String, u16) {
    if let Some(rest) = target.strip_prefix('[') {
        if let Some((host, tail)) = rest.split_once(']') {
            let port = tail
                .strip_prefix(':')
                .and_then(|p| p.parse().ok())
                .unwrap_or(default_port);
            return (format!("[{host}]"), port);
        }
    }
    if target.matches(':').count() == 1 {
        if let Some((h, p)) = target.rsplit_once(':') {
            if let Ok(p) = p.parse() {
                return (h.to_string(), p);
            }
        }
    }
    (target.to_string(), default_port)
}

/// Pairs by the typed code. `target` is `host:port`; `pin` is None for a first contact (the Mac's
/// key is learned from the handshake) or the key an earlier `ask` saw. On success the Mac's proof
/// was checked and its key is in the record.
pub fn pair_with_code(
    target: &str,
    code: &str,
    pin: Option<[u8; 32]>,
    identity: &DeviceIdentity,
    device_name: &str,
    model: Option<&str>,
    timeout: Duration,
) -> Result<SavedMac, PairError> {
    // A typo is refused here: it never costs one of the Mac's five tries.
    let digits = damm::check(code).map_err(PairError::BadCode)?;
    let ex = exchange(
        target,
        identity,
        pin,
        How::Code(&digits),
        device_name,
        model,
        timeout,
    )?;
    if !ex.result.ok {
        return Err(PairError::Refused(PairRefusal::from_result(&ex.result)));
    }
    let (host, port) = split_host_port(target, 7455);
    saved_from_ok(&ex, PairRequest::CODE, vec![host], port)
}

/// Pairs by a `sill://pair?...` link text: pinned to the link's key, the QR secret as the key,
/// every address of the link tried in order (its own port, else the link's).
pub fn pair_with_link(
    link_text: &str,
    identity: &DeviceIdentity,
    device_name: &str,
    model: Option<&str>,
    timeout: Duration,
) -> Result<SavedMac, PairError> {
    let link = PairLink::parse(link_text).map_err(PairError::BadLink)?;
    pair_with_parsed_link(&link, None, identity, device_name, model, timeout)
}

/// As [`pair_with_link`] for a parsed link; `only` dials that one `host:port` instead of the
/// link's addresses (a Mac reached another way, or a test).
pub fn pair_with_parsed_link(
    link: &PairLink,
    only: Option<&str>,
    identity: &DeviceIdentity,
    device_name: &str,
    model: Option<&str>,
    timeout: Duration,
) -> Result<SavedMac, PairError> {
    let pin: [u8; 32] = link
        .fingerprint
        .as_slice()
        .try_into()
        .map_err(|_| PairError::BadLink(LinkFailure::Field("k".into())))?;
    let targets: Vec<String> = match only {
        Some(t) => vec![t.to_string()],
        None => link
            .addresses
            .iter()
            .map(|a| {
                let mut a = a.clone();
                a.port.get_or_insert(link.port);
                a.text()
            })
            .collect(),
    };
    if targets.is_empty() {
        return Err(PairError::NoAddress);
    }
    let mut last = PairError::NoAddress;
    for t in &targets {
        match exchange(
            t,
            identity,
            Some(pin),
            How::Qr(&link.secret),
            device_name,
            model,
            timeout,
        ) {
            Ok(ex) => {
                if !ex.result.ok {
                    return Err(PairError::Refused(PairRefusal::from_result(&ex.result)));
                }
                let addrs = match only {
                    Some(t) => vec![t.to_string()],
                    None => link.addresses.iter().map(|a| a.text()).collect(),
                };
                let mut saved = saved_from_ok(&ex, PairRequest::QR, addrs, link.port)?;
                if saved.name == "Mac" && !link.name.is_empty() {
                    saved.name = link.name.clone();
                }
                return Ok(saved);
            }
            // only a connection that did not happen moves on to the next address
            Err(e @ PairError::Dial(DialError::Connect(_) | DialError::Resolve(_))) => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// What an ask got back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskAnswer {
    /// The Mac shows a code now: type it with [`pair_with_code`], pinned to `mac_fingerprint`.
    Shown { mac_fingerprint: [u8; 32] },
    /// A pairing window is already open on the Mac (look at it).
    OpenOnMac { mac_fingerprint: [u8; 32] },
    /// The Mac is locked: unlock it and ask again.
    Locked { mac_fingerprint: [u8; 32] },
}

/// The home door's "pair me" (kind 19 `ask`, proof ""). Unpinned: the Mac's key is remembered for
/// the code that follows. Every other answer is an error carrying the Mac's reason.
pub fn ask(
    target: &str,
    identity: &DeviceIdentity,
    device_name: &str,
    model: Option<&str>,
    timeout: Duration,
) -> Result<AskAnswer, PairError> {
    let ex = exchange(
        target,
        identity,
        None,
        How::Ask,
        device_name,
        model,
        timeout,
    )?;
    let fp = ex.mac_fp;
    if ex.result.ok {
        // Only the cable's ok answers an ask, and this client never claims the cable.
        return Err(PairError::Protocol(format!(
            "an ok to an ask (method {:?})",
            ex.result.method
        )));
    }
    match PairRefusal::from_result(&ex.result) {
        PairRefusal::Shown => Ok(AskAnswer::Shown {
            mac_fingerprint: fp,
        }),
        PairRefusal::OpenOnMac => Ok(AskAnswer::OpenOnMac {
            mac_fingerprint: fp,
        }),
        PairRefusal::Locked => Ok(AskAnswer::Locked {
            mac_fingerprint: fp,
        }),
        other => Err(PairError::Refused(other)),
    }
}

/// Convenience for tools: standard base64 is never used on pairing; this is the url-safe form.
pub fn b64u(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
