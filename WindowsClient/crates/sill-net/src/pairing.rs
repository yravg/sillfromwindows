//! Pairing: the proofs both sides exchange, the Bonjour recognition tag, the home door's TXT `p`,
//! the `sill://pair` link, the strict address parser the link needs, SafeText, and the check of the
//! Mac's signed kind 18.
//!
//! Ports `PairingProof`, `RecognitionTag`, `HomeDoorTXT` and `PairLink` (Sources/StreamProtocol/
//! Pairing.swift), `SafeText` (SafeText.swift), `AddressParser` (AddressParser.swift) and
//! `SignedMacInfo.verified` (Remote.swift:93-104). Pure: no I/O, no clock; randomness only through
//! the caller (`RecognitionTag::make` takes the nonce).

use crate::identity::{
    base64url_decode, base64url_encode, mac_id, mac_id_is_well_formed, spki_fingerprint,
};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use hmac::{Hmac, Mac};
use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::net::Ipv6Addr;

type HmacSha256 = Hmac<Sha256>;

// ---------------------------------------------------------------------------------------------
// PairingProof (Pairing.swift:67-135)

/// Keys and proofs of one pairing exchange. Each side takes the other's fingerprint from the TLS
/// session, never from a message (Pairing.swift:67-70).
pub mod proof {
    use super::*;

    /// PBKDF2 rounds of the typed path (Pairing.swift:78 `rounds`).
    pub const ROUNDS: u32 = 600_000;
    /// The label of every message and of the salt (Pairing.swift:79 `label`).
    pub const LABEL: &str = "sill-pair-v1";

    /// The typed path's key: PBKDF2-HMAC-SHA256(password = the 12 ASCII digits, salt = "sill-pair-v1"
    /// || fp_mac, 600,000 rounds, 32 bytes) (Pairing.swift:82-98; sillclient.py:389). None unless
    /// `code` is exactly 12 ASCII digits. `mac_fingerprint` is the Mac's SPKI hash from THIS TLS
    /// session (32 bytes, not enforced: Swift salts with whatever it is given).
    pub fn code_key(code: &str, mac_fingerprint: &[u8]) -> Option<[u8; 32]> {
        if code.chars().count() != crate::damm::LENGTH || !code.chars().all(|c| c.is_ascii_digit())
        {
            return None;
        }
        let mut salt = LABEL.as_bytes().to_vec();
        salt.extend_from_slice(mac_fingerprint);
        let mut out = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha256>(code.as_bytes(), &salt, ROUNDS, &mut out);
        Some(out)
    }

    /// The QR path's key: the link's 16-byte secret, used directly (Pairing.swift:101).
    pub fn qr_key(secret: &[u8]) -> Vec<u8> {
        secret.to_vec()
    }

    /// `"sill-pair-v1 device" || 0x00 || fp_device || fp_mac` (Pairing.swift:103-106).
    pub fn device_message(device_fp: &[u8], mac_fp: &[u8]) -> Vec<u8> {
        let mut m = format!("{LABEL} device").into_bytes();
        m.push(0);
        m.extend_from_slice(device_fp);
        m.extend_from_slice(mac_fp);
        m
    }

    /// `"sill-pair-v1 mac" || 0x00 || fp_mac || fp_device` (Pairing.swift:108-111).
    pub fn mac_message(mac_fp: &[u8], device_fp: &[u8]) -> Vec<u8> {
        let mut m = format!("{LABEL} mac").into_bytes();
        m.push(0);
        m.extend_from_slice(mac_fp);
        m.extend_from_slice(device_fp);
        m
    }

    fn hmac(key: &[u8]) -> HmacSha256 {
        HmacSha256::new_from_slice(key).expect("HMAC takes any key length")
    }

    /// proof_D, which the device sends in kind 19, base64url-encoded by the caller
    /// (Pairing.swift:115-117).
    pub fn device_proof(key: &[u8], device_fp: &[u8], mac_fp: &[u8]) -> [u8; 32] {
        let mut m = hmac(key);
        m.update(&device_message(device_fp, mac_fp));
        m.finalize().into_bytes().into()
    }

    /// proof_M, which the Mac sends in kind 20 (Pairing.swift:121-123).
    pub fn mac_proof(key: &[u8], mac_fp: &[u8], device_fp: &[u8]) -> [u8; 32] {
        let mut m = hmac(key);
        m.update(&mac_message(mac_fp, device_fp));
        m.finalize().into_bytes().into()
    }

    /// The Mac's check of proof_D, constant time (Pairing.swift:126-129).
    pub fn is_valid_device_proof(
        proof: &[u8],
        key: &[u8],
        device_fp: &[u8],
        mac_fp: &[u8],
    ) -> bool {
        let mut m = hmac(key);
        m.update(&device_message(device_fp, mac_fp));
        m.verify_slice(proof).is_ok()
    }

    /// The device's check of proof_M, constant time (Pairing.swift:132-135). Do this before saving
    /// anything from a kind 20 ok.
    pub fn is_valid_mac_proof(proof: &[u8], key: &[u8], mac_fp: &[u8], device_fp: &[u8]) -> bool {
        let mut m = hmac(key);
        m.update(&mac_message(mac_fp, device_fp));
        m.verify_slice(proof).is_ok()
    }
}

// ---------------------------------------------------------------------------------------------
// RecognitionTag (Pairing.swift:137-182)

/// The Bonjour TXT record's `r`: base64url (16 characters) of 12 bytes `p || HMAC-SHA256(
/// recognitionKey, "sill-tag-v1" || p)[0..6]`, `p` six fresh random bytes at every registration. A
/// recognition hint, never authentication.
pub mod recognition_tag {
    use super::*;

    /// The TXT key (Pairing.swift:143).
    pub const TXT_KEY: &str = "r";
    const LABEL: &str = "sill-tag-v1";

    fn mac6(recognition_key: &[u8], nonce: &[u8]) -> [u8; 6] {
        let mut m = HmacSha256::new_from_slice(recognition_key).expect("any key length");
        m.update(LABEL.as_bytes());
        m.update(nonce);
        let full = m.finalize().into_bytes();
        let mut out = [0u8; 6];
        out.copy_from_slice(&full[..6]);
        out
    }

    /// A tag for `recognition_key` with the given 6-byte nonce (Pairing.swift:149-160; the Swift
    /// default draws the nonce at random). Only a host makes tags; a device needs this for tests.
    pub fn make(recognition_key: &[u8], nonce: &[u8; 6]) -> String {
        let mut tag = nonce.to_vec();
        tag.extend_from_slice(&mac6(recognition_key, nonce));
        base64url_encode(&tag)
    }

    /// Whether `tag` was made with `recognition_key` (Pairing.swift:163-168). Constant time in the 6
    /// compared bytes; None-canonical base64url or a length other than 12 bytes is false.
    pub fn matches(tag: &str, recognition_key: &[u8]) -> bool {
        let Some(bytes) = base64url_decode(tag) else {
            return false;
        };
        if bytes.len() != 12 {
            return false;
        }
        let expected = mac6(recognition_key, &bytes[..6]);
        let mut diff = 0u8;
        for (a, b) in expected.iter().zip(&bytes[6..]) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

// ---------------------------------------------------------------------------------------------
// HomeDoorTXT (Pairing.swift:184-222)

/// How the home door speaks, from the TXT `p`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeDoor {
    /// No `p`: plain TCP and no pairing (a development build or the CLI without --pairing). A
    /// production device never dials it.
    Plain,
    /// `p=1` or any value but "0": TLS, paired devices only.
    PairingRequired,
    /// `p=0`: TLS, any device.
    Open,
}

/// The TXT key (Pairing.swift:192).
pub const HOME_DOOR_KEY: &str = "p";

/// The value a TLS home door advertises (Pairing.swift:206).
pub fn home_door_value(require_pairing: bool) -> &'static str {
    if require_pairing {
        "1"
    } else {
        "0"
    }
}

/// The door a TXT dictionary describes; keys are case-insensitive ("p", else "P")
/// (Pairing.swift:210-213).
pub fn home_door(txt: &BTreeMap<String, String>) -> HomeDoor {
    match txt.get("p").or_else(|| txt.get("P")) {
        None => HomeDoor::Plain,
        Some(v) if v == "0" => HomeDoor::Open,
        Some(_) => HomeDoor::PairingRequired,
    }
}

/// The door a received record's `p` entry describes (Pairing.swift:219-226). `entry` is None when the
/// key is absent, `Some(None)` for a key with no value (a boolean attribute, which reads as "1"),
/// `Some(Some(bytes))` for a value: exactly b"0" is open, anything else (empty, other, non-UTF-8)
/// requires pairing. Case-insensitive lookup is the caller's (mdns-sd's `get` is case-insensitive).
pub fn home_door_entry(entry: Option<Option<&[u8]>>) -> HomeDoor {
    match entry {
        None => HomeDoor::Plain,
        Some(Some(b"0")) => HomeDoor::Open,
        Some(_) => HomeDoor::PairingRequired,
    }
}

// ---------------------------------------------------------------------------------------------
// SafeText (SafeText.swift)

/// The longest label kept, in characters (SafeText.swift:12).
pub const LABEL_LIMIT: usize = 64;

fn is_format_char(c: char) -> bool {
    // Unicode general category Cf (Unicode 15.1).
    matches!(c as u32,
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x0890..=0x0891 | 0x08E2 | 0x180E
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF
        | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3
        | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F)
}

fn is_private_use_or_noncharacter(c: char) -> bool {
    let u = c as u32;
    matches!(u, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD | 0xFDD0..=0xFDEF)
        || (u & 0xFFFE) == 0xFFFE
}

/// Extends a grapheme cluster: combining marks, ZWJ sequences, variation selectors, emoji skin
/// tones. An approximation of Swift's `Character` (no unicode-segmentation crate in the workspace).
fn extends_cluster(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x0610..=0x061A
        | 0x064B..=0x065F | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF | 0x200D | 0x20D0..=0x20FF | 0x3099..=0x309A | 0xFE00..=0xFE0F
        | 0xFE20..=0xFE2F | 0x1F3FB..=0x1F3FF | 0xE0100..=0xE01EF)
}

/// `text` as a one-line label (SafeText.swift:22-47): whitespace of every kind becomes one space
/// (never leading), control, format and private-use characters are removed, runs of spaces
/// collapse, the ends are trimmed and at most `limit` characters are kept. Empty when nothing
/// printable remains. UNCLEAR vs Swift: unassigned code points (category Cn) are only caught for
/// noncharacters, and "characters" are approximated grapheme clusters.
pub fn safe_label(text: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut pending_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if c.is_control() || is_format_char(c) || is_private_use_or_noncharacter(c) {
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c);
    }
    // Truncate by approximate clusters.
    let mut clusters = 0usize;
    let mut cut = out.len();
    let mut prev_zwj = false;
    for (i, c) in out.char_indices() {
        if !(extends_cluster(c) || prev_zwj) {
            if clusters == limit {
                cut = i;
                break;
            }
            clusters += 1;
        }
        prev_zwj = c == '\u{200D}';
    }
    out.truncate(cut);
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

// ---------------------------------------------------------------------------------------------
// AddressParser (AddressParser.swift)

/// What kind of host an address is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressKind {
    Ipv4,
    Ipv6,
    Name,
}

/// One address as Sill dials it (AddressParser.swift:9-30).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParsedAddress {
    /// "100.101.102.103", "fd7a:115c:a1e0::1234" (no brackets, compressed, lowercase) or a name as typed.
    pub host: String,
    /// None: the Mac's remote port applies (7455 when unknown).
    pub port: Option<u16>,
    pub kind: AddressKind,
}

impl ParsedAddress {
    /// "host" or "host:port", an IPv6 address in brackets: the form a link's `a` carries
    /// (AddressParser.swift:25-28).
    pub fn text(&self) -> String {
        let h = if self.kind == AddressKind::Ipv6 {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        match self.port {
            Some(p) => format!("{h}:{p}"),
            None => h,
        }
    }
}

/// Why an input was refused (AddressParser.swift:38-58).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressFailure {
    Empty,
    Malformed,
    Zone,
    Ipv4,
    Ipv6,
    Name,
    Port,
    PairingLink,
}

/// Parses `raw` as the strict parser does (AddressParser.swift:62-122): ends trimmed, whitespace
/// inside refused, zones refused, exactly four decimal octets for IPv4 (anything inet_aton would
/// read is refused), RFC 1123 names, bare IPv6 for two or more colons, brackets for IPv6 with a port.
pub fn parse_address(raw: &str) -> Result<ParsedAddress, AddressFailure> {
    use AddressFailure as F;
    let s = raw.trim_matches(|c: char| c.is_whitespace());
    if s.is_empty() {
        return Err(F::Empty);
    }
    if s.to_lowercase().starts_with("sill:") {
        return Err(F::PairingLink);
    }
    if s.chars()
        .any(|c| c.is_whitespace() || (c as u32) < 0x20 || c as u32 == 0x7F)
    {
        return Err(F::Malformed);
    }
    if s.contains('%') {
        return Err(F::Zone);
    }
    let (host_part, port_part): (&str, Option<&str>);
    if let Some(rest) = s.strip_prefix('[') {
        let close = rest.find(']').ok_or(F::Ipv6)?;
        let inner = &rest[..close];
        let after = &rest[close + 1..];
        let mut port_s = None;
        if !after.is_empty() {
            port_s = Some(after.strip_prefix(':').ok_or(F::Malformed)?);
        }
        let v6 = parse_ipv6(inner).ok_or(F::Ipv6)?;
        return match port_s {
            Some(p) => Ok(ParsedAddress {
                port: Some(parse_port(p).ok_or(F::Port)?),
                ..v6
            }),
            None => Ok(v6),
        };
    }
    let colons = s.matches(':').count();
    if colons >= 2 {
        // A bare IPv6 address, never an address plus a port (AddressParser.swift:93-98).
        return parse_ipv6(s).ok_or(F::Ipv6);
    }
    if colons == 1 {
        let i = s.find(':').unwrap();
        host_part = &s[..i];
        port_part = Some(&s[i + 1..]);
    } else {
        host_part = s;
        port_part = None;
    }
    let host = host_only(host_part)?;
    match port_part {
        Some(p) => Ok(ParsedAddress {
            port: Some(parse_port(p).ok_or(F::Port)?),
            ..host
        }),
        None => Ok(host),
    }
}

fn host_only(h: &str) -> Result<ParsedAddress, AddressFailure> {
    use AddressFailure as F;
    if h.is_empty() {
        return Err(F::Malformed);
    }
    let body = h.strip_suffix('.').unwrap_or(h);
    let labels: Vec<&str> = body.split('.').collect();
    let all_numeric = !labels.is_empty()
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()));
    if all_numeric {
        let ok = !h.ends_with('.')
            && labels.len() == 4
            && labels.iter().all(|l| {
                (*l == "0" || !l.starts_with('0'))
                    && l.len() <= 3
                    && l.parse::<u32>().map(|n| n <= 255).unwrap_or(false)
            });
        if !ok {
            return Err(F::Ipv4);
        }
        return Ok(ParsedAddress {
            host: h.to_string(),
            port: None,
            kind: AddressKind::Ipv4,
        });
    }
    // Whatever inet_aton would take as IPv4 and is not the canonical form above is refused.
    if accepted_by_inet_aton(h) {
        return Err(F::Ipv4);
    }
    if !is_name(h) {
        return Err(F::Name);
    }
    Ok(ParsedAddress {
        host: h.to_string(),
        port: None,
        kind: AddressKind::Name,
    })
}

/// RFC 1123 host name (AddressParser.swift:144-153).
fn is_name(h: &str) -> bool {
    let body = h.strip_suffix('.').unwrap_or(h);
    if body.is_empty() || body.chars().count() > 253 {
        return false;
    }
    body.split('.').all(|l| {
        (1..=63).contains(&l.chars().count())
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// A bare IPv6 address, compressed lowercase; IPv4 for an IPv4-mapped one (AddressParser.swift:
/// 157-170). UNCLEAR: Apple's `inet_ntop` prints IPv4-compatible forms like "::1.2.3.4" where Rust
/// prints "::102:304"; not reached by any real link.
fn parse_ipv6(h: &str) -> Option<ParsedAddress> {
    if h.is_empty() || h.contains('%') {
        return None;
    }
    let a: Ipv6Addr = h.parse().ok()?;
    let o = a.octets();
    if o[..10].iter().all(|b| *b == 0) && o[10] == 0xFF && o[11] == 0xFF {
        return Some(ParsedAddress {
            host: format!("{}.{}.{}.{}", o[12], o[13], o[14], o[15]),
            port: None,
            kind: AddressKind::Ipv4,
        });
    }
    Some(ParsedAddress {
        host: a.to_string(),
        port: None,
        kind: AddressKind::Ipv6,
    })
}

/// C's `inet_aton` acceptance: 1 to 4 parts, each decimal, octal (leading 0) or hex (0x), the last
/// part filling the remaining bytes (AddressParser.swift:172-175 calls the real one).
fn accepted_by_inet_aton(h: &str) -> bool {
    let parts: Vec<&str> = h.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return false;
    }
    let mut vals = Vec::new();
    for p in &parts {
        let v = if let Some(x) = p.strip_prefix("0x").or_else(|| p.strip_prefix("0X")) {
            // glibc/BSD accept "0x" alone as 0; require hex digits otherwise.
            if x.is_empty() {
                Some(0u64)
            } else {
                u64::from_str_radix(x, 16).ok()
            }
        } else if p.len() > 1 && p.starts_with('0') {
            u64::from_str_radix(&p[1..], 8).ok()
        } else if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) {
            p.parse::<u64>().ok()
        } else {
            None
        };
        match v {
            Some(v) => vals.push(v),
            None => return false,
        }
    }
    let last = *vals.last().unwrap();
    let n = vals.len();
    if vals[..n - 1].iter().any(|v| *v > 255) {
        return false;
    }
    last < (1u64 << (8 * (5 - n as u32)))
}

fn parse_port(p: &str) -> Option<u16> {
    if p.is_empty() || p.len() > 5 || !p.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let v: u32 = p.parse().ok()?;
    (1..=65535).contains(&v).then_some(v as u16)
}

// ---------------------------------------------------------------------------------------------
// PairLink (Pairing.swift:223-307)

/// Most addresses a link keeps (Pairing.swift:228).
pub const MAX_ADDRESSES: usize = 5;
/// Longest Mac name kept (Pairing.swift:229).
pub const NAME_LIMIT: usize = 64;

/// The QR code's content: `sill://pair?v=1&m=&k=&s=&n=&p=&a=...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairLink {
    /// 16 Crockford characters; always `mac_id(fingerprint)`.
    pub mac_id: String,
    /// The Mac's pin: SHA-256 of its SPKI (32 bytes).
    pub fingerprint: Vec<u8>,
    /// The QR path's one-time key (16 bytes).
    pub secret: Vec<u8>,
    /// The Mac's name after SafeText, at most 64 characters; "Mac" when empty.
    pub name: String,
    /// The remote port (1-65535).
    pub port: u16,
    /// At most 5, in the Mac's dial order; each with its own port when it has one.
    pub addresses: Vec<ParsedAddress>,
}

/// Why a link was refused (Pairing.swift:259-270).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkFailure {
    NotALink,
    Version,
    Field(String),
    MacIdMismatch,
    Address(String),
}

impl PairLink {
    /// Builds a link (Pairing.swift:241-250): the Mac ID is derived, the name cleaned, addresses cut to 5.
    pub fn new(
        fingerprint: &[u8],
        secret: &[u8],
        name: &str,
        port: u16,
        addresses: Vec<ParsedAddress>,
    ) -> PairLink {
        let clean = safe_label(name, NAME_LIMIT);
        PairLink {
            mac_id: mac_id(fingerprint),
            fingerprint: fingerprint.to_vec(),
            secret: secret.to_vec(),
            name: if clean.is_empty() {
                "Mac".into()
            } else {
                clean
            },
            port,
            addresses: addresses.into_iter().take(MAX_ADDRESSES).collect(),
        }
    }

    /// The link as the QR code carries it (Pairing.swift:274-281): values percent-encoded down to
    /// RFC 3986's unreserved characters and ":".
    pub fn url(&self) -> String {
        let mut items: Vec<(&str, String)> = vec![
            ("v", "1".into()),
            ("m", self.mac_id.clone()),
            ("k", base64url_encode(&self.fingerprint)),
            ("s", base64url_encode(&self.secret)),
            ("n", self.name.clone()),
            ("p", self.port.to_string()),
        ];
        items.extend(
            self.addresses
                .iter()
                .take(MAX_ADDRESSES)
                .map(|a| ("a", a.text())),
        );
        let q: Vec<String> = items
            .iter()
            .map(|(k, v)| format!("{k}={}", escape(v)))
            .collect();
        format!("sill://pair?{}", q.join("&"))
    }

    /// Parses a link (Pairing.swift:289-307). Refuses `v` != 1; a missing, repeated or malformed `m`,
    /// `k`, `s` or `p`; `m` != MacID(k); any `a` the address parser refuses. Ignores unknown
    /// parameters, keeps the first 5 addresses, cleans `n` with SafeText.
    pub fn parse(text: &str) -> Result<PairLink, LinkFailure> {
        use LinkFailure as L;
        let items =
            split_link(text.trim_matches(|c: char| c.is_whitespace())).ok_or(L::NotALink)?;
        let single = |key: &str| -> Result<String, L> {
            let vals: Vec<&(String, Option<String>)> =
                items.iter().filter(|(k, _)| k == key).collect();
            if vals.len() == 1 {
                if let Some(v) = &vals[0].1 {
                    if !v.is_empty() {
                        return Ok(v.clone());
                    }
                }
            }
            Err(L::Field(key.to_string()))
        };
        let v = single("v").map_err(|_| L::Version)?;
        if v != "1" {
            return Err(L::Version);
        }
        let m = single("m")
            .ok()
            .filter(|m| mac_id_is_well_formed(m))
            .ok_or_else(|| L::Field("m".into()))?;
        let fp = single("k")
            .ok()
            .and_then(|k| base64url_decode(&k))
            .filter(|f| f.len() == 32)
            .ok_or_else(|| L::Field("k".into()))?;
        let secret = single("s")
            .ok()
            .and_then(|s| base64url_decode(&s))
            .filter(|s| s.len() == 16)
            .ok_or_else(|| L::Field("s".into()))?;
        let port = single("p")
            .ok()
            .filter(|p| p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()))
            .and_then(|p| p.parse::<u32>().ok())
            .filter(|p| (1..=65535).contains(p))
            .ok_or_else(|| L::Field("p".into()))? as u16;
        if m != mac_id(&fp) {
            return Err(L::MacIdMismatch);
        }
        let mut addresses = Vec::new();
        for (k, val) in &items {
            if k == "a" {
                let raw = val.clone().unwrap_or_default();
                match parse_address(&raw) {
                    Ok(a) => addresses.push(a),
                    Err(_) => return Err(L::Address(raw)),
                }
            }
        }
        let name = items
            .iter()
            .find(|(k, _)| k == "n")
            .and_then(|(_, v)| v.clone())
            .unwrap_or_default();
        Ok(PairLink::new(&fp, &secret, &name, port, addresses))
    }
}

/// Percent-encodes down to RFC 3986 unreserved characters and ":" (Pairing.swift:283-286).
fn escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b':') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(
                u8::from_str_radix(h, 16)
                    .ok()
                    .filter(|_| h.bytes().all(|c| c.is_ascii_hexdigit()))?,
            );
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Splits `sill://pair?query` into decoded (name, value) items as URLComponents.queryItems does:
/// scheme and host case-insensitive, path empty or "/", a fragment ignored, a name without `=` has
/// no value (None). None when this is not that URL, or a percent escape is invalid.
fn split_link(text: &str) -> Option<Vec<(String, Option<String>)>> {
    let (scheme, rest) = text.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("sill") {
        return None;
    }
    let rest = rest.split('#').next().unwrap_or("");
    let (authority_path, query) = match rest.split_once('?') {
        Some((a, q)) => (a, q),
        None => (rest, ""),
    };
    let (host, path) = match authority_path.find('/') {
        Some(i) => (&authority_path[..i], &authority_path[i..]),
        None => (authority_path, ""),
    };
    if !host.eq_ignore_ascii_case("pair") || !(path.is_empty() || path == "/") {
        return None;
    }
    if query.is_empty() {
        // URLComponents' queryItems is nil for no query (notALink).
        return None;
    }
    let mut items = Vec::new();
    for part in query.split('&') {
        if part.is_empty() {
            continue;
        }
        match part.split_once('=') {
            Some((k, v)) => items.push((percent_decode(k)?, Some(percent_decode(v)?))),
            None => items.push((percent_decode(part)?, None)),
        }
    }
    Some(items)
}

// ---------------------------------------------------------------------------------------------
// SignedMacInfo (Remote.swift:76-109)

/// Why a kind 18 did not verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacInfoFailure {
    /// A field is not standard padded base64, or the key is not a 65-byte `04 ||X||Y` point.
    Malformed,
    /// The DER ECDSA P-256/SHA-256 signature does not verify over the exact `info` bytes.
    BadSignature,
    /// `info` is not JSON with a string `macID`.
    BadInfo,
    /// `macID` is not the Mac ID of the key that signed it.
    MacIdMismatch,
}

/// What `verify_signed_mac_info` returns: the exact JSON bytes of MacInfo (decode them with
/// sill-proto's MacInfo; never re-serialize before verifying) and the signer's fingerprint. A device
/// that saved this Mac must also require `fingerprint` == its pin and `issuedAt` newer than stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedMacInfo {
    pub info_json: Vec<u8>,
    pub fingerprint: [u8; 32],
    pub mac_id: String,
}

/// `SignedMacInfo.verified()` (Remote.swift:93-104; sillclient.py `verify_macinfo`): base64 (standard,
/// padded) `info`, `key` (65-byte X9.63 point) and `sig` (DER ECDSA); the signature must verify over
/// the exact `info` bytes; the JSON's `macID` must be the Mac ID of that key.
pub fn verify_signed_mac_info(
    info_b64: &str,
    key_b64: &str,
    sig_b64: &str,
) -> Result<VerifiedMacInfo, MacInfoFailure> {
    use MacInfoFailure as F;
    let info = STANDARD.decode(info_b64).map_err(|_| F::Malformed)?;
    let point = STANDARD.decode(key_b64).map_err(|_| F::Malformed)?;
    let sig = STANDARD.decode(sig_b64).map_err(|_| F::Malformed)?;
    if point.len() != 65 || point[0] != 0x04 {
        return Err(F::Malformed);
    }
    let vk = VerifyingKey::from_sec1_bytes(&point).map_err(|_| F::Malformed)?;
    let signature = Signature::from_der(&sig).map_err(|_| F::Malformed)?;
    vk.verify(&info, &signature).map_err(|_| F::BadSignature)?;
    #[derive(serde::Deserialize)]
    struct Id {
        #[serde(rename = "macID")]
        mac_id: String,
    }
    let id: Id = serde_json::from_slice(&info).map_err(|_| F::BadInfo)?;
    let fingerprint = spki_fingerprint(&point).ok_or(F::Malformed)?;
    if id.mac_id != mac_id(&fingerprint) {
        return Err(F::MacIdMismatch);
    }
    Ok(VerifiedMacInfo {
        info_json: info,
        fingerprint,
        mac_id: id.mac_id,
    })
}

/// Host-side counterpart (Remote.swift:84-91), for tests and tools: base64 of the info bytes, the
/// signer's point and a DER signature over the info bytes.
pub fn sign_mac_info(
    info_json: &[u8],
    signer: &crate::identity::DeviceIdentity,
) -> (String, String, String) {
    (
        STANDARD.encode(info_json),
        STANDARD.encode(signer.public_point()),
        STANDARD.encode(signer.sign_der(info_json)),
    )
}
