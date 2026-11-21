//! Sources/StreamProtocol/Input.swift: kind 8, `InputEvent` (a Swift enum with associated values,
//! so one top-level key per case; unlabeled values are `_0`; confirmed by sillclient.py:710-714).

use serde::{Deserialize, Serialize};

/// `PointerAction` (Input.swift:6-12): raw strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PointerAction {
    Move,
    LeftDown,
    LeftUp,
    RightDown,
    RightUp,
}

/// `ScrollPhase` (Input.swift:37-39): raw strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScrollPhase {
    Began,
    Ended,
    MomentumBegan,
    MomentumEnded,
}

/// `InputEvent` (Input.swift:14-35). Coordinates are fractions 0..1 of the streamed frame.
/// Modifiers are UIKeyModifierFlags bits (0x10000 caps, 0x20000 shift, 0x40000 ctrl, 0x80000 option,
/// 0x100000 command). `Text` is `{"text":{"_0":"hello"}}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
    #[serde(rename = "pointer")]
    Pointer {
        #[serde(rename = "_0")]
        action: PointerAction,
        x: f64,
        y: f64,
    },
    #[serde(rename = "scroll")]
    Scroll { x: f64, y: f64, dx: f64, dy: f64 },
    #[serde(rename = "text")]
    Text {
        #[serde(rename = "_0")]
        text: String,
    },
    #[serde(rename = "scrollGesture")]
    ScrollGesture {
        #[serde(rename = "_0")]
        phase: ScrollPhase,
        x: f64,
        y: f64,
    },
    #[serde(rename = "key")]
    Key {
        #[serde(rename = "hidUsage")]
        hid_usage: u16,
        down: bool,
        modifiers: u64,
    },
}
