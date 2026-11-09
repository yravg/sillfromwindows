//! Sources/StreamProtocol/Remote.swift: kinds 18-22 (macInfo, pair request/result, goodbye).
//! The signature check and the proofs are crypto and live in `sill-net`; this module is the JSON.

use crate::wire;
use serde::{Deserialize, Serialize};

/// `MacAddress` (Remote.swift:18-36): how to reach a Mac away from home.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MacAddress {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<i64>,
    /// "vpn", "lan" or "internet".
    pub kind: String,
    pub via: String,
}

/// `MacInfo` (Remote.swift:39-65).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacInfo {
    pub v: i64,
    #[serde(rename = "macID")]
    pub mac_id: String,
    pub name: String,
    pub issued_at: f64,
    pub remote_access: bool,
    pub remote_port: i64,
    pub internet: bool,
    pub addresses: Vec<MacAddress>,
}

/// `SignedMacInfo` (Remote.swift:67-109), kind 18: standard padded base64 of the exact `MacInfo`
/// JSON bytes, of the Mac's 65-byte X9.63 P-256 point, and of a DER ECDSA-SHA256 signature over the
/// raw info bytes. Never re-serialize the info before verifying.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedMacInfo {
    pub info: String,
    pub key: String,
    pub sig: String,
}

impl SignedMacInfo {
    /// The raw info bytes the signature covers.
    pub fn info_bytes(&self) -> Option<Vec<u8>> {
        wire::b64_decode(&self.info)
    }

    /// The Mac's public point, only when it is 65 bytes (Remote.swift:93).
    pub fn key_point(&self) -> Option<Vec<u8>> {
        wire::b64_decode(&self.key).filter(|p| p.len() == 65)
    }

    /// The DER signature bytes.
    pub fn sig_der(&self) -> Option<Vec<u8>> {
        wire::b64_decode(&self.sig)
    }

    /// `unverifiedInfo()`: the info decoded for display, NOT verified.
    pub fn unverified_info(&self) -> Option<MacInfo> {
        self.info_bytes().and_then(|b| wire::decode_object(&b))
    }
}

/// `PairRequest` (Remote.swift:112-147), kind 19. `proof` is base64url (no padding) of the
/// device proof, "" for ask and cancel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairRequest {
    pub v: i64,
    pub method: String,
    pub proof: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cable: Option<bool>,
}

impl PairRequest {
    pub const VERSION: i64 = 1;
    pub const QR: &'static str = "qr";
    pub const CODE: &'static str = "code";
    pub const ASK: &'static str = "ask";
    pub const CANCEL: &'static str = "cancel";

    pub fn new(method: &str, proof: &str, name: &str, model: Option<&str>) -> Self {
        PairRequest {
            v: Self::VERSION,
            method: method.into(),
            proof: proof.into(),
            name: name.into(),
            model: model.map(Into::into),
            cable: None,
        }
    }
}

/// `PairResult` (Remote.swift:149-209), kind 20.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "macID")]
    pub mac_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognition_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tries_left: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl PairResult {
    pub const CODE: &'static str = "code";
    pub const CLOSED: &'static str = "closed";
    pub const EXPIRED: &'static str = "expired";
    pub const STOPPED: &'static str = "stopped";
    pub const BUSY: &'static str = "busy";
    pub const SHOWN: &'static str = "shown";
    pub const OPEN_ON_MAC: &'static str = "openOnMac";
    pub const LOCKED: &'static str = "locked";
    pub const CABLE: &'static str = "cable";

    /// `knownReasons`.
    pub const KNOWN_REASONS: [&'static str; 8] = [
        "code",
        "closed",
        "expired",
        "stopped",
        "busy",
        "shown",
        "openOnMac",
        "locked",
    ];

    /// True for a refusal whose reason this build does not know: show its `message` (SafeText,
    /// at most 300 characters, applied by the caller).
    pub fn has_unknown_reason(&self) -> bool {
        !self.ok
            && !self
                .reason
                .as_deref()
                .is_some_and(|r| Self::KNOWN_REASONS.contains(&r))
    }
}

/// `Goodbye` (Remote.swift:211-236), kind 22.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goodbye {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconnect: Option<bool>,
}

impl Goodbye {
    pub const REMOVED: &'static str = "removed";
    pub const REMOTE_OFF: &'static str = "remoteOff";
    pub const INTERNET_OFF: &'static str = "internetOff";
    pub const QUIT: &'static str = "quit";
    pub const BUSY: &'static str = "busy";
    pub const PAIRING_REQUIRED: &'static str = "pairingRequired";
    pub const UPDATE: &'static str = "update";
}
