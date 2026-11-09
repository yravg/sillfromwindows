//! Sources/StreamProtocol/Switcher.swift: the catalog types (kinds 2, 5, 6, 7, 15).

use serde::{Deserialize, Serialize};

/// `StreamSource` (Switcher.swift:4-8), synthesized Codable of an enum with associated values:
/// `{"none":{}}`, `{"desktop":{}}`, `{"window":{"_0":123}}` (confirmed by Scripts/sillclient.py:190-195).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "SourceRepr", into = "SourceRepr")]
pub enum StreamSource {
    None,
    /// A CGWindowID.
    Window(u32),
    Desktop,
}

#[derive(Serialize, Deserialize)]
enum SourceRepr {
    #[serde(rename = "none")]
    None {},
    #[serde(rename = "window")]
    Window {
        #[serde(rename = "_0")]
        id: u32,
    },
    #[serde(rename = "desktop")]
    Desktop {},
}

impl From<SourceRepr> for StreamSource {
    fn from(r: SourceRepr) -> Self {
        match r {
            SourceRepr::None {} => StreamSource::None,
            SourceRepr::Window { id } => StreamSource::Window(id),
            SourceRepr::Desktop {} => StreamSource::Desktop,
        }
    }
}
impl From<StreamSource> for SourceRepr {
    fn from(s: StreamSource) -> Self {
        match s {
            StreamSource::None => SourceRepr::None {},
            StreamSource::Window(id) => SourceRepr::Window { id },
            StreamSource::Desktop => SourceRepr::Desktop {},
        }
    }
}

/// `WindowInfo` (Switcher.swift:11-24). Sizes are points.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_name: String,
    #[serde(rename = "bundleID")]
    pub bundle_id: String,
    pub width: i64,
    pub height: i64,
}

/// `AppInfo` (Switcher.swift:26-32): kind 5 is a JSON array of these.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    #[serde(rename = "bundleID")]
    pub bundle_id: String,
}

/// `WindowList` (Switcher.swift:34-89), kind 2. Optional fields are nil from older hosts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowList {
    pub mac_name: String,
    pub windows: Vec<WindowInfo>,
    pub active: StreamSource,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "launchID")]
    pub launch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<i64>,
    /// 1: the host takes kind 28.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gestures: Option<i64>,
    /// 1: the host takes kind 32 text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<i64>,
    /// 1: the host repeats a held key; send hardware characters as keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_repeat: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_console: Option<bool>,
}

/// `LockedMac.Why` (Switcher.swift:95-109).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockedWhy {
    ScreenLocked,
    OtherUser,
}

impl WindowList {
    /// `LockedMac.why`: why the list is empty, when it is because the Mac is locked.
    pub fn locked_why(&self) -> Option<LockedWhy> {
        if self.locked != Some(true) {
            return None;
        }
        Some(if self.off_console == Some(true) {
            LockedWhy::OtherUser
        } else {
            LockedWhy::ScreenLocked
        })
    }
}

/// `LaunchApp` (Switcher.swift:166-170), kind 7.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchApp {
    #[serde(rename = "bundleID")]
    pub bundle_id: String,
}

/// `WindowCommand.Action` (Switcher.swift:172-177): the raw strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowAction {
    Close,
    Minimize,
    FullScreen,
}

/// `WindowCommand` (Switcher.swift:172-178), kind 15.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowCommand {
    pub id: u32,
    pub action: WindowAction,
}
