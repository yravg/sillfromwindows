//! Sources/StreamProtocol/HostSettings.swift: kinds 16 (host to device) and 17 (device to host).
//! Every field added later is optional; ints are emitted as integers.

use serde::{Deserialize, Serialize};

/// `StreamSettings` (HostSettings.swift:64-99).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamSettings {
    #[serde(rename = "maxFPS")]
    pub max_fps: i64,
    pub bitrate: i64,
    pub capture_scale: f64,
    pub prioritize_speed: bool,
    pub virtual_display: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_wireless: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_audio: Option<bool>,
}

/// `RunningStream` (HostSettings.swift:101-120).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningStream {
    pub width: i64,
    pub height: i64,
    pub fps: i64,
    pub mbps: i64,
    pub on_virtual_display: bool,
}

/// `AwayQuality` (HostSettings.swift:122-145).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwayQuality {
    pub home_bitrate: i64,
    pub home_capture_scale: f64,
    pub away_bitrate: i64,
    pub away_capture_scale: f64,
    pub this_connection_away: bool,
    pub away_running: bool,
}

/// `LinkReport` (HostSettings.swift:147-183). `state` is a string ("behind", "stalled"); read
/// anything else as keeping up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkReport {
    pub state: String,
    pub withheld_per_second: i64,
    pub bitrate: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carried_kbps: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_bitrate: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_capture_scale: Option<f64>,
}

impl LinkReport {
    pub const BEHIND: &'static str = "behind";
    pub const STALLED: &'static str = "stalled";

    pub fn is_behind(&self) -> bool {
        self.state == Self::BEHIND
    }
}

/// `HostSettingsState` (HostSettings.swift:185-241), kind 16.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSettingsState {
    pub settings: StreamSettings,
    pub persistent: bool,
    pub virtual_display_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_display_note: Option<String>,
    pub software_encoder: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<RunningStream>,
    /// The token of the kind 17 this answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answering: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub away: Option<AwayQuality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_paired: Option<bool>,
}

/// `HostSettingsChange` (HostSettings.swift:243-295), kind 17: only the changed fields, absolute
/// values, a token strictly increasing per process.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSettingsChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "maxFPS")]
    pub max_fps: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prioritize_speed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_display: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_wireless: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_audio: Option<bool>,
}

impl HostSettingsChange {
    /// `isEmpty`: nothing to change (the token does not count).
    pub fn is_empty(&self) -> bool {
        self.max_fps.is_none()
            && self.bitrate.is_none()
            && self.capture_scale.is_none()
            && self.prioritize_speed.is_none()
            && self.virtual_display.is_none()
            && self.direct_wireless.is_none()
            && self.send_audio.is_none()
    }

    /// `applied(to:)`.
    pub fn applied(&self, s: &StreamSettings) -> StreamSettings {
        let mut r = s.clone();
        if let Some(v) = self.max_fps {
            r.max_fps = v
        }
        if let Some(v) = self.bitrate {
            r.bitrate = v
        }
        if let Some(v) = self.capture_scale {
            r.capture_scale = v
        }
        if let Some(v) = self.prioritize_speed {
            r.prioritize_speed = v
        }
        if let Some(v) = self.virtual_display {
            r.virtual_display = v
        }
        if let Some(v) = self.direct_wireless {
            r.direct_wireless = Some(v)
        }
        if let Some(v) = self.send_audio {
            r.send_audio = Some(v)
        }
        r
    }
}

/// `SettingsChoices` (HostSettings.swift:297-308): the values a host accepts.
pub struct SettingsChoices;

impl SettingsChoices {
    pub const MAX_FPS: [i64; 2] = [60, 120];
    pub const CAPTURE_SCALE: [f64; 2] = [2.0, 1.0];

    /// The seven preset bitrates, ascending.
    pub fn bitrate() -> Vec<i64> {
        QualityPreset::ALL.iter().map(|p| p.bitrate()).collect()
    }
}

/// `QualityPreset` (HostSettings.swift:310-362): bitrate per 60 fps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QualityPreset {
    Low,
    Efficient,
    Balanced,
    High,
    Pro,
    Ultra,
    Extreme,
}

impl QualityPreset {
    pub const ALL: [QualityPreset; 7] = [
        QualityPreset::Low,
        QualityPreset::Efficient,
        QualityPreset::Balanced,
        QualityPreset::High,
        QualityPreset::Pro,
        QualityPreset::Ultra,
        QualityPreset::Extreme,
    ];

    pub fn bitrate(self) -> i64 {
        match self {
            Self::Low => 4_000_000,
            Self::Efficient => 8_000_000,
            Self::Balanced => 15_000_000,
            Self::High => 25_000_000,
            Self::Pro => 40_000_000,
            Self::Ultra => 80_000_000,
            Self::Extreme => 150_000_000,
        }
    }

    pub fn from_bitrate(b: i64) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.bitrate() == b)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Efficient => "Efficient",
            Self::Balanced => "Balanced",
            Self::High => "High",
            Self::Pro => "Pro",
            Self::Ultra => "Ultra",
            Self::Extreme => "Extreme",
        }
    }

    /// "Pro — 40 Mbps".
    pub fn title(self) -> String {
        format!("{} — {} Mbps", self.name(), self.bitrate() / 1_000_000)
    }

    pub fn title_for_bitrate(b: i64) -> String {
        match Self::from_bitrate(b) {
            Some(p) => p.title(),
            None => format!("Custom — {} Mbps", Self::mbps(b)),
        }
    }

    pub fn name_for_bitrate(b: i64) -> String {
        match Self::from_bitrate(b) {
            Some(p) => p.name().into(),
            None => format!("{} Mbps", Self::mbps(b)),
        }
    }

    /// "Low · Standard" / "Pro · Retina" (scale 1.5 or more is Retina).
    pub fn short_title(bitrate: i64, capture_scale: f64) -> String {
        format!(
            "{} · {}",
            Self::name_for_bitrate(bitrate),
            if capture_scale >= 1.5 {
                "Retina"
            } else {
                "Standard"
            }
        )
    }

    /// Whole megabits as an integer, else one decimal (`%.1f`).
    pub fn mbps(bits_per_second: i64) -> String {
        if bits_per_second % 1_000_000 == 0 {
            format!("{}", bits_per_second / 1_000_000)
        } else {
            format!("{:.1}", bits_per_second as f64 / 1_000_000.0)
        }
    }
}
