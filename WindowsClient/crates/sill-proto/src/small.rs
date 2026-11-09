//! Small JSON kinds: `MacPointer` (Pointer.swift, kind 26), `TrackpadGesture` (Gesture.swift,
//! kind 28), `Clipboard` (Clipboard.swift, kind 32).

use crate::wire;
use serde::{Deserialize, Serialize};

/// `MacPointer` (Pointer.swift:17-43), kind 26: where the Mac's pointer is in the streamed frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MacPointer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside: Option<bool>,
    /// The kind 8 messages the host had read on this connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<i64>,
}

impl MacPointer {
    /// `rounded`: four decimals.
    pub fn rounded(v: f64) -> f64 {
        (v * 10_000.0).round() / 10_000.0
    }

    /// `position`: only when inside and both coordinates are finite.
    pub fn position(&self) -> Option<(f64, f64)> {
        if self.inside != Some(true) {
            return None;
        }
        match (self.x, self.y) {
            (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Some((x, y)),
            _ => None,
        }
    }
}

/// `TrackpadGesture` (Gesture.swift:13-39), kind 28. The name is a string (a later host may send
/// more); only to a host whose window list says `gestures` 1 or more.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackpadGesture {
    pub gesture: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingers: Option<i64>,
}

impl TrackpadGesture {
    pub const SWIPE_UP: &'static str = "swipeUp";
    pub const SWIPE_DOWN: &'static str = "swipeDown";
    pub const SWIPE_LEFT: &'static str = "swipeLeft";
    pub const SWIPE_RIGHT: &'static str = "swipeRight";
    pub const PINCH: &'static str = "pinch";
    pub const SPREAD: &'static str = "spread";
    pub const NAMES: [&'static str; 6] = [
        "swipeUp",
        "swipeDown",
        "swipeLeft",
        "swipeRight",
        "pinch",
        "spread",
    ];
    pub const GENERATION: i64 = 1;
}

/// `Clipboard` (Clipboard.swift:19-39), kind 32. `image` is reserved (never sent); a device's
/// `stop` ends its subscription.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clipboard {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "wire::b64_opt"
    )]
    pub image: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<bool>,
}

impl Clipboard {
    /// Text is at most 256 KB of UTF-8 (Clipboard.swift:37).
    pub const MAX_TEXT_BYTES: usize = 256 * 1024;
}
