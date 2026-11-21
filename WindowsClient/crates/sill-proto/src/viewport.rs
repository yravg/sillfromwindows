//! Sources/StreamProtocol/Viewport.swift: kinds 9 and 12, and the viewport bounds a host enforces.

use serde::{Deserialize, Serialize};

/// 2^63: the first Double that is no Int (Swift `Int(exactly:)` fails from here).
const TWO_63: f64 = 9_223_372_036_854_775_808.0;

/// `Viewport` (Viewport.swift:11-27), kind 9. `scale` omitted = leave the Mac window alone
/// (sillclient sends an explicit null, which decodes the same).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    pub width: f64,
    pub height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_cursor: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<i64>,
}

/// `ViewportBounds` (Viewport.swift:45-96): what a host takes. Every device build holds what it
/// sends to these.
pub struct ViewportBounds;

impl ViewportBounds {
    pub const SIDES: (f64, f64) = (1.0, 16384.0);
    pub const SCALES: (f64, f64) = (0.25, 4.0);

    fn in_sides(x: f64) -> bool {
        x >= Self::SIDES.0 && x <= Self::SIDES.1
    }

    /// `refusal`: why a host would refuse `v` whole, or `None` when it takes it.
    pub fn refusal(v: &Viewport) -> Option<String> {
        let points = format!(
            "({} to {} points)",
            Self::number(Self::SIDES.0),
            Self::number(Self::SIDES.1)
        );
        let mut why = Vec::new();
        if !Self::in_sides(v.width) {
            why.push(format!("width {} {points}", Self::number(v.width)));
        }
        if !Self::in_sides(v.height) {
            why.push(format!("height {} {points}", Self::number(v.height)));
        }
        if let Some(s) = v.scale {
            if !(s >= Self::SCALES.0 && s <= Self::SCALES.1) {
                why.push(format!(
                    "scale {} ({} to {})",
                    Self::number(s),
                    Self::number(Self::SCALES.0),
                    Self::number(Self::SCALES.1)
                ));
            }
        }
        if why.is_empty() {
            None
        } else {
            Some(why.join(", "))
        }
    }

    /// `held`: `v` clamped into the bounds; `None` when a side is not finite or not above 0. A
    /// scale that is not finite or not above 0 becomes none.
    pub fn held(v: &Viewport) -> Option<Viewport> {
        if !(v.width.is_finite() && v.height.is_finite() && v.width > 0.0 && v.height > 0.0) {
            return None;
        }
        let clamp = |x: f64, (lo, hi): (f64, f64)| x.max(lo).min(hi);
        let mut held = *v;
        held.width = clamp(v.width, Self::SIDES);
        held.height = clamp(v.height, Self::SIDES);
        held.scale = v.scale.and_then(|s| {
            if s.is_finite() && s > 0.0 {
                Some(clamp(s, Self::SCALES))
            } else {
                None
            }
        });
        Some(held)
    }

    /// `number`: an integer without a decimal point, else Swift's `"\(x)"` description.
    pub fn number(x: f64) -> String {
        if x == x.round() && x.abs() < TWO_63 {
            return format!("{}", x as i64);
        }
        swift_description(x)
    }

    /// `whole`: the number toward zero, as an integer when it fits, else its description.
    pub fn whole(x: f64) -> String {
        let t = x.trunc();
        if t.is_finite() && t.abs() < TWO_63 {
            return format!("{}", t as i64);
        }
        swift_description(x)
    }
}

/// Swift's `Double.description`, enough for log lines: plain decimal between 1e-4 and 2^63,
/// otherwise `d.ddde+XX` with at least two exponent digits.
fn swift_description(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-inf".into() } else { "inf".into() };
    }
    let a = x.abs();
    if a == 0.0 || (1e-4..TWO_63).contains(&a) {
        let s = format!("{x}");
        return if s.contains('.') { s } else { format!("{s}.0") };
    }
    let s = format!("{x:e}"); // 1e300, 1.5e-5
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let e: i32 = exp.parse().unwrap_or(0);
    let sign = if e < 0 { '-' } else { '+' };
    format!("{mant}e{sign}{:02}", e.abs())
}

/// `ClientStats` (Viewport.swift:107-133), kind 12. -1 = no sample.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientStats {
    pub fps: i64,
    pub frame_age_ms: i64,
    pub rtt_ms: i64,
    pub device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_age_max_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_max_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_behind_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_late: Option<i64>,
}

impl ClientStats {
    /// `lateSince` (Viewport.swift:135): a late count added without ever overflowing.
    pub fn late_since(count: i64, adding: Option<i64>) -> i64 {
        let more = adding.unwrap_or(0).max(0);
        count.saturating_add(more)
    }
}
