//! The contract between the app's parts. OWNER: the lead. Workers read it and build against it; a
//! change goes through the lead.
//!
//! Shape (Sill's own: a client object owns the connection, the views read its state):
//! - The UI thread (egui) reads [`Shared`] every frame and sends [`Command`]s.
//! - The controller thread (controller.rs) owns the session, the decoders and the stores; it
//!   applies commands, updates [`Shared`] and asks egui for a repaint.
//! - Decoded video goes through [`VideoSlot`], newest frame wins (latency beats quality).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sill_proto::input::InputEvent;
use sill_proto::menu::MacMenu;
use sill_proto::settings::{HostSettingsChange, HostSettingsState};
use sill_proto::switcher::{AppInfo, StreamSource, WindowAction, WindowInfo};

/// What the UI asks for. Every control sends one of these; nothing else reaches the Mac.
/// (`WindowCommand` keeps the wire's name for kind 15.)
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone)]
pub enum Command {
    /// Connect to a row of the connect screen (its [`MacRow::id`]).
    Connect(String),
    /// Connect to a saved Mac through its Remote Access addresses (the row's "Connect Remotely").
    ConnectRemotely(String),
    /// Pair with the code a Mac shows: `addr` is "host" or "host:port" (port 7455 when absent).
    PairWithCode {
        addr: String,
        code: String,
    },
    /// Pair with a `sill://pair…` link (pasted, or from the protocol handler).
    PairWithLink(String),
    /// Ask a home-network Mac (row id) to show its code (kind 19 "ask").
    AskToPair(String),
    /// Forget a saved Mac (its Mac ID).
    Forget(String),
    /// End the session; back to the connect screen.
    Disconnect,
    /// Kind 6.
    Pick(StreamSource),
    /// Kind 7: a bundle ID from the app list.
    Launch(String),
    /// Kind 15.
    WindowCommand {
        window_id: u32,
        action: WindowAction,
    },
    /// Kind 8: one input event, positions already as fractions of the frame (0…1).
    Input(InputEvent),
    /// Kind 9: the stream panel's size in points and the Aa scale (None = Off).
    Viewport {
        width: f64,
        height: f64,
        scale: Option<f64>,
    },
    /// Kind 17 (through the settings ledger).
    ChangeSettings(HostSettingsChange),
    /// Kind 32: text from this PC's clipboard to the Mac.
    SendClipboard(String),
    /// Kind 27 / 25: the Mac's menus.
    FetchMenu {
        id: String,
        title: String,
    },
    PressMenu {
        id: String,
        title: String,
    },
    /// Kind 28 by name ("swipeUp", "swipeDown", "swipeLeft", "swipeRight", "pinch", "spread").
    Gesture(String),
    /// This device's own preferences (never sent): mute, share clipboard, reverse scrolling.
    SetPreference(Preference),
    /// The app is quitting: let every key and button go, say nothing more.
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Preference {
    Mute(bool),
    ShareClipboard(bool),
    ReverseScrolling(bool),
    /// D2: false = Ctrl is ⌘ (default), true = the Windows key is ⌘.
    WindowsKeyIsCommand(bool),
}

/// Which screen shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Screen {
    #[default]
    Connect,
    /// Connected, or carried by a reconnect: the stream screen stays.
    Stream,
}

/// One row of the connect screen.
#[derive(Debug, Clone, PartialEq)]
pub struct MacRow {
    /// Stable within a run: "saved:<macID>" or "bonjour:<service name>".
    pub id: String,
    pub name: String,
    /// The trailing word: "Wi-Fi", "Wired", "Remote", "Not paired", "Update Sill", or none.
    pub word: Option<String>,
    /// A saved (paired) Mac's ID.
    pub mac_id: Option<String>,
    /// True while the row's dial or ask is under way (drawn lit).
    pub busy: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum PairingState {
    #[default]
    Idle,
    /// Dialing and proving; the words say what is happening.
    Working(String),
    /// The Mac answered "shown": its code is on its screen now.
    CodeShown {
        mac: String,
    },
    Paired {
        mac: String,
    },
    /// What failed and what to do, in Sill's words.
    Failed(String),
}

/// The connect screen's state.
#[derive(Debug, Clone, Default)]
pub struct ConnectModel {
    pub rows: Vec<MacRow>,
    /// The status line under the title ("Looking for Macs on this network", "Connecting to …").
    pub status: String,
    /// "Update Sill" notice, when a Mac's goodbye asked for it.
    pub notice: Option<String>,
    pub pairing: PairingState,
    /// True once discovery has run 3 s with nothing found (the hint shows).
    pub nothing_found: bool,
    /// Windows treats this network as Public (D4), when known.
    pub public_network: bool,
}

/// A thumbnail or an icon, decoded to RGBA once, with a counter that changes when it changes.
#[derive(Debug, Clone)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
    pub generation: u64,
}

/// The Mac's cursor shape (kind 14) and where the Mac's pointer is (kind 26).
#[derive(Debug, Clone, Default)]
pub struct MacCursor {
    pub shape: Option<Picture>,
    /// Hotspot and size in points.
    pub hotspot: (f32, f32),
    pub size: (f32, f32),
    /// Fractions of the frame while the Mac (not this PC) moves the pointer over it.
    pub mac_position: Option<(f64, f64)>,
}

/// The stream screen's state.
#[derive(Debug, Clone, Default)]
pub struct StreamModel {
    pub mac_name: String,
    pub host_version: Option<String>,
    pub windows: Vec<WindowInfo>,
    pub apps: Vec<AppInfo>,
    pub active: Option<StreamSource>,
    pub thumbnails: HashMap<u32, Picture>,
    pub icons: HashMap<String, Picture>,
    pub settings: Option<HostSettingsState>,
    /// The settings ledger's view: what the controls show (the Mac's state with this PC's
    /// unanswered picks over it), and the last refusal's words.
    pub settings_shown: Option<HostSettingsState>,
    pub settings_problem: Option<String>,
    pub menus: Option<MacMenu>,
    /// Answers to menu fetches, by menu id.
    pub menu_items: HashMap<String, MacMenu>,
    pub cursor: MacCursor,
    /// The Mac is locked or another user is on it: LockedMac.line's words, for the Apps drawer and
    /// the Settings panel's header (Switcher.swift:131). Never drawn over the picture.
    pub locked: Option<String>,
    /// What covers the picture (LockedMac.shade, Switcher.swift:106-160): only while another user is
    /// on the Mac, then "Unlocking…" until the next keyframe (at most 4 s). A Mac whose own screen is
    /// locked has NO cover: its lock screen streams, so the person can type the password.
    pub cover: Option<Cover>,
    /// The link's line over the stream ("The link can't keep up with Pro. Lower it in Settings.").
    pub link_line: Option<String>,
    /// "Reconnecting to …" while the session is carried by a reconnect.
    pub reconnecting: Option<String>,
    /// The Mac shares its clipboard (WindowList.clipboard >= 1).
    pub mac_clipboard: bool,
    /// The Mac repeats held keys (WindowList.keyRepeat >= 1): send characters as keys.
    pub mac_key_repeat: bool,
    /// The Mac takes trackpad gestures (WindowList.gestures >= 1).
    pub mac_gestures: bool,
    /// Diagnostics: decoded frames a second, frame age and round trip in ms.
    pub fps: f64,
    pub frame_age_ms: f64,
    pub rtt_ms: f64,
}

/// A cover over the stream: LockedMac.title and LockedMac.detail.
#[derive(Debug, Clone, PartialEq)]
pub struct Cover {
    pub title: String,
    pub detail: Option<String>,
}

/// This PC's own preferences, persisted by the controller.
#[derive(Debug, Clone, PartialEq)]
pub struct Preferences {
    pub mute: bool,
    pub share_clipboard: bool,
    pub reverse_scrolling: bool,
    pub windows_key_is_command: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences {
            mute: false,
            share_clipboard: true,
            reverse_scrolling: false,
            windows_key_is_command: false,
        }
    }
}

/// Everything the UI draws, behind one lock. The controller writes, the UI reads (clone what you
/// keep across frames; hold the lock briefly).
#[derive(Debug, Clone, Default)]
pub struct AppModel {
    pub screen: Screen,
    pub connect: ConnectModel,
    pub stream: StreamModel,
    pub prefs: Preferences,
    /// This PC's name as the Mac sees it ("‹PC name› (Windows)", D8).
    pub device_name: String,
}

pub type Shared = Arc<Mutex<AppModel>>;

/// One decoded video frame: BGRA8, `stride` bytes a row.
#[derive(Debug)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bgra: Vec<u8>,
    /// Counts up; the UI uploads only a frame it has not shown.
    pub serial: u64,
}

/// Newest decoded frame; the decoder replaces, the UI takes.
pub type VideoSlot = Arc<Mutex<Option<VideoFrame>>>;

/// How the controller wakes the UI (egui's `Context::request_repaint`, or nothing in tests).
pub type Waker = Arc<dyn Fn() + Send + Sync>;
