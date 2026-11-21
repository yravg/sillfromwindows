//! OWNER: worker C1. The controller thread: commands in, model and video out.
//!
//! One thread owns discovery, the store, at most one [`Session`], the settings ledger and the
//! reconnect. Anything that blocks (a dial, a pairing exchange, decoding a picture) runs on a worker
//! thread and comes back as a message; the decoders have threads of their own (`feed.rs`: video
//! behind a queue of two frames, audio behind a short one). The pure rules (the ledger, the
//! reconnect schedule, which row dials where, the viewport's dedupe, the decoder's drop rule, the
//! words) are in `controller/` with their tests.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sill_media::audio::AacEldDecoder;
use sill_media::playout::Playout;
use sill_media::video::HevcDecoder;
use sill_net::identity::DeviceIdentity;
use sill_net::pair_client::{self, AskAnswer, PairError, SavedMac};
use sill_net::pairing::{parse_address, ParsedAddress};
use sill_net::session::{CloseReason, DialError, Session, SessionConfig, SessionEvent};
use sill_proto::audio::{AudioFormat, AudioMessage};
use sill_proto::blobs;
use sill_proto::framing::ParameterSets;
use sill_proto::menu::{FetchMenu, MacMenu, PressMenuItem};
use sill_proto::remote::{Goodbye, MacInfo};
use sill_proto::small::{Clipboard, MacPointer, TrackpadGesture};
use sill_proto::switcher::{AppInfo, LaunchApp, StreamSource, WindowCommand, WindowList};
use sill_proto::{wire, Message, MessageKind};

use crate::clipboard;
use crate::discovery::Discovery;
use crate::model::*;
use crate::store::Store;

#[path = "controller/feed.rs"]
mod feed;
#[path = "controller/ledger.rs"]
mod ledger;
#[path = "controller/reconnect.rs"]
mod reconnect;
#[path = "controller/targets.rs"]
mod targets;
#[path = "controller/viewport.rs"]
mod viewport;
#[path = "controller/words.rs"]
mod words;

use feed::{FeedPolicy, Job, JobQueue};
use ledger::Ledger;
use targets::Plan;
use viewport::ViewportTracker;

pub struct Handles {
    pub shared: Shared,
    pub commands: Sender<Command>,
    pub video: VideoSlot,
}

/// What the controller counted, for tests and the live run (the model carries only what is drawn).
#[derive(Default)]
pub struct Counters {
    pub frames_in: AtomicU64,
    pub keyframes_in: AtomicU64,
    pub frames_dropped: AtomicU64,
    pub frames_decoded: AtomicU64,
    pub bytes_in: AtomicU64,
    pub sets_in: AtomicU64,
    pub keyframe_requests: AtomicU64,
    pub audio_packets: AtomicU64,
    pub window_lists: AtomicU64,
    pub clip_sent: AtomicU64,
    pub clip_received: AtomicU64,
    /// Frames dropped because the decoder's queue held two already.
    pub dropped_full: AtomicU64,
    /// Frames dropped while a keyframe was awaited (a chain broken, or none yet).
    pub dropped_waiting: AtomicU64,
    /// Decode time of the frames decoded, microseconds, and the slowest.
    pub decode_micros: AtomicU64,
    pub decode_max_micros: AtomicU64,
}

pub fn start(waker: Waker) -> Handles {
    start_with_counters(waker).0
}

pub fn start_with_counters(waker: Waker) -> (Handles, Arc<Counters>) {
    let shared: Shared = Default::default();
    let video: VideoSlot = Default::default();
    let counters = Arc::new(Counters::default());
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    // commands reach the controller through the same channel as its workers' answers
    let fwd = msg_tx.clone();
    let _ = std::thread::Builder::new()
        .name("sill-commands".into())
        .spawn(move || {
            for c in cmd_rx {
                if fwd.send(Msg::Cmd(c)).is_err() {
                    return;
                }
            }
            let _ = fwd.send(Msg::Cmd(Command::Quit)); // the UI is gone
        });
    let c = Controller::new(
        shared.clone(),
        video.clone(),
        waker,
        counters.clone(),
        msg_tx,
    );
    let _ = std::thread::Builder::new()
        .name("sill-controller".into())
        .spawn(move || c.run(msg_rx));
    (
        Handles {
            shared,
            commands: cmd_tx,
            video,
        },
        counters,
    )
}

// --- messages -----------------------------------------------------------------------------------

enum Msg {
    Cmd(Command),
    Dialed(Box<DialDone>),
    Paired(PairDone),
    Asked(AskDone),
    Image(ImageDone),
}

struct DialDone {
    gen: u64,
    mac: SavedMac,
    auto: bool,
    result: Result<(Session, String), DialError>,
}
struct PairDone {
    gen: u64,
    result: Result<SavedMac, PairError>,
    name: String,
}
struct AskDone {
    gen: u64,
    name: String,
    target: String,
    result: Result<AskAnswer, PairError>,
}
enum ImageKind {
    Thumb(u32),
    Icon(String),
    Cursor {
        hotspot: (f32, f32),
        size: (f32, f32),
    },
}
struct ImageDone {
    kind: ImageKind,
    picture: Picture,
}
struct ImageJob {
    kind: ImageKind,
    bytes: Vec<u8>,
}

/// TEST ONLY: `SILL_FEED_TRACE=1` prints every frame and what the feed did with it.
fn feed_trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SILL_FEED_TRACE").is_some())
}

fn trace_clock() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

static PICTURE_GENERATION: AtomicU64 = AtomicU64::new(1);

const IDLE_STATUS: &str = "Looking for Macs on this network";
const DIAL_TIMEOUT: Duration = Duration::from_secs(4);
const PAIR_TIMEOUT: Duration = Duration::from_secs(20);
const AUTO_DESKTOP_EVERY: Duration = Duration::from_secs(10);

// --- decoder threads ----------------------------------------------------------------------------

#[derive(Default)]
struct DecodeFlags {
    /// The decoder failed on a frame: the next keyframe is needed.
    error: AtomicBool,
    /// FFmpeg could not be loaded (why).
    unavailable: Mutex<Option<String>>,
}

fn decode_thread(
    queue: Arc<JobQueue>,
    video: VideoSlot,
    waker: Waker,
    counters: Arc<Counters>,
    flags: Arc<DecodeFlags>,
) {
    let mut decoder = match HevcDecoder::new() {
        Ok(d) => Some(d),
        Err(e) => {
            if let Ok(mut u) = flags.unavailable.lock() {
                *u = Some(e.to_string());
            }
            None
        }
    };
    let mut serial = 0u64;
    loop {
        match queue.pop() {
            Job::Stop => return,
            Job::Reset => {}
            Job::Sets(sets) => {
                if let Some(d) = decoder.as_mut() {
                    if d.set_parameter_sets(&sets).is_err() {
                        flags.error.store(true, Ordering::Relaxed);
                    }
                }
            }
            Job::Frame { avcc, keyframe } => {
                let Some(d) = decoder.as_mut() else { continue };
                let started = Instant::now();
                let result = d.decode(&avcc, keyframe);
                let micros = started.elapsed().as_micros() as u64;
                counters.decode_micros.fetch_add(micros, Ordering::Relaxed);
                counters
                    .decode_max_micros
                    .fetch_max(micros, Ordering::Relaxed);
                match result {
                    Ok(Some(f)) => {
                        serial += 1;
                        counters.frames_decoded.fetch_add(1, Ordering::Relaxed);
                        if let Ok(mut slot) = video.lock() {
                            *slot = Some(VideoFrame {
                                width: f.width,
                                height: f.height,
                                stride: f.stride,
                                bgra: f.bgra,
                                serial,
                            });
                        }
                        waker();
                    }
                    Ok(None) => {}
                    Err(_) => flags.error.store(true, Ordering::Relaxed),
                }
            }
        }
    }
}

enum AudioJob {
    Format(AudioFormat),
    Packets {
        timestamp: f64,
        segment_start: bool,
        packets: Vec<Vec<u8>>,
    },
    Mute(bool),
    LagMs(f64),
    /// The session runs through the remote door (away from home).
    Away(bool),
    /// A session ended: drop the decoder and the output.
    Reset,
    Stop,
}

fn audio_thread(rx: Receiver<AudioJob>, muted: bool) {
    let mut muted = muted;
    let mut decoder: Option<AacEldDecoder> = None;
    let mut playout: Option<Playout> = None;
    let mut shape = (48_000u32, 2u16, 480i64);
    let mut warned = false;
    let mut away = false;
    let mut lag_ms: Option<f64> = None;
    for job in rx {
        match job {
            AudioJob::Stop => {
                if let Some(p) = playout.as_mut() {
                    p.stop();
                }
                return;
            }
            AudioJob::Mute(m) => {
                muted = m;
                if let Some(p) = playout.as_mut() {
                    p.set_muted(m);
                }
            }
            AudioJob::Away(a) => {
                away = a;
                if let Some(p) = playout.as_mut() {
                    p.set_away(a);
                }
            }
            AudioJob::Reset => {
                if let Some(p) = playout.as_mut() {
                    p.stop();
                }
                playout = None;
                decoder = None;
            }
            AudioJob::LagMs(ms) => {
                lag_ms = Some(ms);
                if let Some(p) = playout.as_mut() {
                    p.set_picture_lag_ms(ms);
                }
            }
            AudioJob::Format(f) => {
                let rate = f.sample_rate.unwrap_or(48_000).clamp(8_000, 192_000) as u32;
                let channels = f.channels.unwrap_or(2).clamp(1, 8) as u16;
                shape = (rate, channels, f.frames_per_packet.unwrap_or(480).max(1));
                match AacEldDecoder::new(&f) {
                    Ok(d) => decoder = Some(d),
                    Err(e) => {
                        decoder = None;
                        if !warned {
                            warned = true;
                            eprintln!("audio: {e}");
                        }
                    }
                }
                if decoder.is_some() {
                    if let Some(p) = playout.as_mut() {
                        p.stop();
                    }
                    let mut p = Playout::new(rate, channels);
                    p.set_muted(muted);
                    p.set_away(away);
                    if let Some(ms) = lag_ms {
                        p.set_picture_lag_ms(ms);
                    }
                    playout = Some(p);
                }
            }
            AudioJob::Packets {
                timestamp,
                segment_start,
                packets,
            } => {
                let (Some(d), Some(p)) = (decoder.as_mut(), playout.as_mut()) else {
                    continue;
                };
                if segment_start {
                    d.reset();
                }
                let per_packet = shape.2 as f64 / f64::from(shape.0);
                for (i, packet) in packets.iter().enumerate() {
                    if let Ok(samples) = d.decode(packet) {
                        if !samples.is_empty() {
                            p.push(
                                &samples,
                                timestamp + per_packet * i as f64,
                                segment_start && i == 0,
                            );
                        }
                    }
                }
            }
        }
    }
}

fn image_thread(rx: Receiver<ImageJob>, tx: Sender<Msg>) {
    for job in rx {
        let Ok(img) = image::load_from_memory(&job.bytes) else {
            continue;
        };
        let rgba = img.to_rgba8();
        let picture = Picture {
            width: rgba.width(),
            height: rgba.height(),
            rgba: Arc::new(rgba.into_raw()),
            generation: PICTURE_GENERATION.fetch_add(1, Ordering::Relaxed),
        };
        if tx
            .send(Msg::Image(ImageDone {
                kind: job.kind,
                picture,
            }))
            .is_err()
        {
            return;
        }
    }
}

// --- the controller -----------------------------------------------------------------------------

struct Live {
    session: Session,
    mac: SavedMac,
    feed: FeedPolicy,
    got_list: bool,
}

struct Reconnect {
    mac: SavedMac,
    remote_allowed: bool,
    failed: u32,
    last_dial: Option<Instant>,
    next_at: Instant,
    dialing: bool,
    /// The source to ask for again when the new session's first list says nothing streams.
    resume: Option<StreamSource>,
}

struct PendingAsk {
    target: String,
    fp: [u8; 32],
}

struct Controller {
    shared: Shared,
    video: VideoSlot,
    waker: Waker,
    counters: Arc<Counters>,
    msg_tx: Sender<Msg>,
    store: Option<Store>,
    identity: Option<DeviceIdentity>,
    macs: Vec<SavedMac>,
    prefs: Preferences,
    device_name: String,
    discovery: Discovery,
    gen: u64,
    dialing: bool,
    busy_row: Option<String>,
    status_override: Option<String>,
    pending_ask: Option<PendingAsk>,
    live: Option<Live>,
    reconnect: Option<Reconnect>,
    ledger: Ledger,
    vp: ViewportTracker,
    wanted_vp: Option<(f64, f64, Option<f64>)>,
    user_chose: bool,
    last_auto_desktop: Option<Instant>,
    last_source: Option<StreamSource>,
    clip: clipboard::Sync,
    host_clipboard: Option<i64>,
    shade: Option<words::Shade>,
    unlocking_since: Option<Instant>,
    session_paired: Option<bool>,
    menu_tokens: HashMap<i64, String>,
    next_menu_token: i64,
    menu_version: Option<i64>,
    goodbye: Option<Goodbye>,
    worst_rtt_ms: f64,
    last_rtt_ms: f64,
    queue: Arc<JobQueue>,
    flags: Arc<DecodeFlags>,
    audio_tx: Sender<AudioJob>,
    image_tx: Sender<ImageJob>,
    last_rows: Vec<MacRow>,
    decoded_at_last_second: u64,
    decode_micros_at_last_second: u64,
    last_second: Instant,
    resume_source: Option<StreamSource>,
    quitting: bool,
}

/// "‹computer name› (Windows)" (plan D8).
pub fn device_name() -> String {
    let mut name = None;
    for var in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                name = Some(v.trim().to_string());
                break;
            }
        }
    }
    let name = name
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "PC".into());
    format!("{name} (Windows)")
}

impl Controller {
    fn new(
        shared: Shared,
        video: VideoSlot,
        waker: Waker,
        counters: Arc<Counters>,
        msg_tx: Sender<Msg>,
    ) -> Self {
        let store = match Store::open() {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("store: {e}");
                None
            }
        };
        let macs = store.as_ref().map(|s| s.load_macs()).unwrap_or_default();
        let prefs = store.as_ref().map(|s| s.load_prefs()).unwrap_or_default();
        let identity = match store.as_ref().map(|s| s.identity()) {
            Some(Ok(i)) => Some(i),
            Some(Err(e)) => {
                eprintln!("identity: {e}");
                None
            }
            None => None,
        };
        let queue = Arc::new(JobQueue::default());
        queue.set_capacity(feed::QUEUE_START);
        let flags = Arc::new(DecodeFlags::default());
        {
            let (q, v, w, c, f) = (
                queue.clone(),
                video.clone(),
                waker.clone(),
                counters.clone(),
                flags.clone(),
            );
            let _ = std::thread::Builder::new()
                .name("sill-decode".into())
                .spawn(move || decode_thread(q, v, w, c, f));
        }
        let (audio_tx, audio_rx) = mpsc::channel();
        let muted = prefs.mute;
        let _ = std::thread::Builder::new()
            .name("sill-audio".into())
            .spawn(move || audio_thread(audio_rx, muted));
        let (image_tx, image_rx) = mpsc::channel();
        let mtx = msg_tx.clone();
        let _ = std::thread::Builder::new()
            .name("sill-images".into())
            .spawn(move || image_thread(image_rx, mtx));
        let discovery = Discovery::start(waker.clone());
        let device_name = device_name();
        let c = Controller {
            shared,
            video,
            waker,
            counters,
            msg_tx,
            store,
            identity,
            macs,
            prefs,
            device_name,
            discovery,
            gen: 0,
            dialing: false,
            busy_row: None,
            status_override: None,
            pending_ask: None,
            live: None,
            reconnect: None,
            ledger: Ledger::new(),
            vp: ViewportTracker::default(),
            wanted_vp: None,
            user_chose: false,
            last_auto_desktop: None,
            last_source: None,
            clip: clipboard::Sync::default(),
            host_clipboard: None,
            shade: None,
            unlocking_since: None,
            session_paired: None,
            menu_tokens: HashMap::new(),
            next_menu_token: 1,
            menu_version: None,
            goodbye: None,
            worst_rtt_ms: 0.0,
            last_rtt_ms: 0.0,
            queue,
            flags,
            audio_tx,
            image_tx,
            last_rows: Vec::new(),
            decoded_at_last_second: 0,
            decode_micros_at_last_second: 0,
            last_second: Instant::now(),
            resume_source: None,
            quitting: false,
        };
        c.update(|m| {
            m.device_name = c.device_name.clone();
            m.prefs = c.prefs.clone();
            m.connect.status = IDLE_STATUS.into();
        });
        c
    }

    fn update(&self, f: impl FnOnce(&mut AppModel)) {
        if let Ok(mut m) = self.shared.lock() {
            f(&mut m);
        }
        (self.waker)();
    }

    // --- the loop ---------------------------------------------------------------------------

    fn run(mut self, rx: Receiver<Msg>) {
        let mut next_tick = Instant::now();
        while !self.quitting {
            let wait = if self.live.is_some() {
                Duration::from_millis(2)
            } else {
                Duration::from_millis(25)
            };
            match rx.recv_timeout(wait) {
                Ok(m) => self.handle(m),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            while !self.quitting {
                match rx.try_recv() {
                    Ok(m) => self.handle(m),
                    Err(_) => break,
                }
            }
            self.pump_session();
            let now = Instant::now();
            if now >= next_tick {
                self.tick(now);
                next_tick = now + Duration::from_millis(250);
            }
        }
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(live) = self.live.take() {
            live.session.close();
        }
        self.gen += 1;
        let _ = self.audio_tx.send(AudioJob::Stop);
        self.queue.push_control(Job::Stop);
    }

    fn handle(&mut self, m: Msg) {
        match m {
            Msg::Cmd(c) => self.command(c),
            Msg::Dialed(d) => self.dialed(*d),
            Msg::Paired(p) => self.paired(p),
            Msg::Asked(a) => self.asked(a),
            Msg::Image(i) => self.image(i),
        }
    }

    // --- commands ---------------------------------------------------------------------------

    fn command(&mut self, c: Command) {
        match c {
            Command::Connect(row) => self.connect_row(&row, false),
            Command::ConnectRemotely(row) => self.connect_row(&row, true),
            Command::PairWithCode { addr, code } => self.pair_code(&addr, &code),
            Command::PairWithLink(link) => self.pair_link(&link),
            Command::Forget(id) => self.forget(&id),
            Command::Disconnect => self.disconnect(),
            Command::Pick(source) => {
                self.user_chose = true;
                if let Some(l) = &self.live {
                    l.session.select_source(source);
                }
            }
            Command::Launch(bundle_id) => {
                self.user_chose = true;
                self.send(
                    MessageKind::LaunchApp,
                    wire::encode(&LaunchApp { bundle_id }),
                );
            }
            Command::WindowCommand { window_id, action } => {
                self.user_chose = true;
                self.send(
                    MessageKind::WindowCommand,
                    wire::encode(&WindowCommand {
                        id: window_id,
                        action,
                    }),
                );
            }
            Command::Input(ev) => {
                self.send(MessageKind::Input, wire::encode(&ev));
            }
            Command::Viewport {
                width,
                height,
                scale,
            } => {
                self.wanted_vp = Some((width, height, scale));
                self.send_viewport();
            }
            Command::ChangeSettings(change) => {
                if self.live.is_some() {
                    if let Some(c) = self.ledger.pick(change, Instant::now()) {
                        self.send(MessageKind::ChangeSettings, wire::encode(&c));
                    }
                    self.publish_ledger();
                }
            }
            Command::FetchMenu { id, title } => {
                let token = self.menu_token(&id);
                let m = FetchMenu {
                    version: self.menu_version,
                    id: Some(id),
                    title: Some(title),
                    token: Some(token),
                };
                self.send(MessageKind::FetchMenu, wire::encode(&m));
            }
            Command::PressMenu { id, title } => {
                let token = self.menu_token(&id);
                let m = PressMenuItem {
                    version: self.menu_version,
                    id: Some(id),
                    title: Some(title),
                    token: Some(token),
                };
                self.send(MessageKind::PressMenuItem, wire::encode(&m));
            }
            Command::Gesture(name) => {
                let known = TrackpadGesture::NAMES.contains(&name.as_str());
                let takes = self
                    .shared
                    .lock()
                    .map(|m| m.stream.mac_gestures)
                    .unwrap_or(false);
                if known && takes {
                    let g = TrackpadGesture {
                        gesture: name,
                        fingers: Some(3),
                    };
                    self.send(MessageKind::Gesture, wire::encode(&g));
                }
            }
            Command::SetPreference(p) => self.set_preference(p),
            Command::Quit => self.quitting = true,
        }
    }

    fn send(&self, kind: MessageKind, payload: Vec<u8>) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| l.session.send(kind, payload))
    }

    fn menu_token(&mut self, id: &str) -> i64 {
        let t = self.next_menu_token;
        self.next_menu_token += 1;
        if self.menu_tokens.len() > 256 {
            self.menu_tokens.clear();
        }
        self.menu_tokens.insert(t, id.to_string());
        t
    }

    fn send_viewport(&mut self) {
        let (Some((w, h, s)), Some(l)) = (self.wanted_vp, self.live.as_ref()) else {
            return;
        };
        if let Some(v) = self.vp.next(w, h, s) {
            l.session.send_viewport(&v);
        }
    }

    fn set_preference(&mut self, p: Preference) {
        match p {
            Preference::Mute(m) => {
                self.prefs.mute = m;
                let _ = self.audio_tx.send(AudioJob::Mute(m));
            }
            Preference::ShareClipboard(on) => {
                self.prefs.share_clipboard = on;
                if let Some(c) =
                    self.clip
                        .sync_subscription(self.host_clipboard, on, Instant::now())
                {
                    self.send(MessageKind::Clipboard, c);
                }
            }
            Preference::ReverseScrolling(v) => self.prefs.reverse_scrolling = v,
            Preference::WindowsKeyIsCommand(v) => self.prefs.windows_key_is_command = v,
        }
        if let Some(s) = &self.store {
            let _ = s.save_prefs(&self.prefs);
        }
        let prefs = self.prefs.clone();
        self.update(|m| m.prefs = prefs);
    }

    // --- rows, dialing, pairing -------------------------------------------------------------

    fn set_status(&mut self, s: Option<String>) {
        self.status_override = s;
        let text = self
            .status_override
            .clone()
            .unwrap_or_else(|| IDLE_STATUS.into());
        self.update(|m| m.connect.status = text);
    }

    fn need_identity(&mut self) -> Option<DeviceIdentity> {
        if self.identity.is_none() {
            self.identity = self.store.as_ref().and_then(|s| s.identity().ok());
        }
        if self.identity.is_none() {
            self.set_status(Some(
                "Sill couldn’t open this PC’s key. Nothing was connected.".into(),
            ));
        }
        self.identity.clone()
    }

    fn connect_row(&mut self, row: &str, remote_only: bool) {
        if self.live.is_some() {
            return;
        }
        if let Some(r) = self.reconnect.as_mut() {
            r.next_at = Instant::now(); // a tap while it reconnects: try now
            return;
        }
        if self.dialing {
            return;
        }
        match targets::plan(row, &self.macs, &self.discovery.found(), remote_only) {
            Plan::Dial { mac, targets } => {
                self.busy_row = Some(row.to_string());
                let words = if remote_only {
                    format!("Connecting to {} remotely…", mac.name)
                } else {
                    format!("Connecting to {}…", mac.name)
                };
                self.set_status(Some(words));
                self.start_dial(mac, targets, false);
            }
            Plan::Pair { name, target } => self.start_ask(name, target),
            Plan::Plain(name) => {
                self.set_status(Some(format!(
                    "{name} runs an older Sill. Update Sill on the Mac."
                )));
            }
            Plan::NotFound => self.set_status(Some("That Mac isn’t listed any more.".into())),
        }
    }

    fn start_dial(&mut self, mac: SavedMac, targets: Vec<String>, auto: bool) {
        let Some(identity) = self.need_identity() else {
            self.busy_row = None;
            return;
        };
        let Some(pin) = mac.pin() else {
            self.busy_row = None;
            self.set_status(Some(format!(
                "{}’s saved key is damaged. Pair it again.",
                mac.name
            )));
            return;
        };
        self.gen += 1;
        let gen = self.gen;
        self.dialing = true;
        let tx = self.msg_tx.clone();
        let device_name = self.device_name.clone();
        let _ = std::thread::Builder::new()
            .name("sill-dial".into())
            .spawn(move || {
                let mut result = Err(DialError::Resolve("no address to dial".into()));
                for t in targets {
                    let cfg = SessionConfig {
                        target: t.clone(),
                        pin,
                        device_name: device_name.clone(),
                        audio: true,
                        connect_timeout: DIAL_TIMEOUT,
                    };
                    match Session::open(&cfg, &identity) {
                        Ok(s) => {
                            result = Ok((s, t));
                            break;
                        }
                        Err(DialError::PinMismatch) => {
                            result = Err(DialError::PinMismatch);
                            break;
                        }
                        Err(e) => result = Err(e),
                    }
                }
                let _ = tx.send(Msg::Dialed(Box::new(DialDone {
                    gen,
                    mac,
                    auto,
                    result,
                })));
            });
    }

    fn dialed(&mut self, d: DialDone) {
        if d.gen != self.gen {
            return; // cancelled: a session that arrives late is dropped, which closes it
        }
        self.dialing = false;
        match d.result {
            Ok((session, target)) => {
                let found = self.discovery.found();
                let home = targets::reconnect_targets(&d.mac, &found, false);
                let _ = self.audio_tx.send(AudioJob::Away(!home.contains(&target)));
                self.begin_session(d.mac, session)
            }
            Err(e) => {
                let words = words::dial_failure(&e, &d.mac.name);
                if d.auto {
                    if matches!(e, DialError::PinMismatch) {
                        self.reconnect = None;
                        self.back_to_connect(Some(words), None);
                    } else {
                        self.reconnect_failed(Instant::now());
                    }
                } else {
                    self.busy_row = None;
                    self.set_status(Some(words));
                }
            }
        }
    }

    fn begin_session(&mut self, mac: SavedMac, session: Session) {
        let now = Instant::now();
        let resumed = self.reconnect.take();
        self.busy_row = None;
        self.set_status(None);
        self.goodbye = None;
        self.vp.forget();
        self.ledger.reset();
        self.queue.reset();
        self.worst_rtt_ms = 0.0;
        self.menu_tokens.clear();
        self.menu_version = None;
        let (name, resume) = (mac.name.clone(), resumed.as_ref().and_then(|r| r.resume));
        if resumed.is_none() {
            self.shade = None;
            self.session_paired = None;
            self.host_clipboard = None;
            self.user_chose = false;
            self.last_source = None;
            self.last_auto_desktop = None;
            self.update(|m| {
                m.stream = StreamModel {
                    mac_name: name.clone(),
                    ..Default::default()
                };
                m.screen = Screen::Stream;
                m.connect.notice = None;
                m.connect.pairing = PairingState::Idle;
            });
        } else {
            self.update(|m| {
                m.stream.reconnecting = None;
                m.stream.settings = None;
                m.stream.settings_shown = None;
                m.stream.link_line = None;
                m.screen = Screen::Stream;
            });
        }
        self.live = Some(Live {
            session,
            mac,
            feed: FeedPolicy::new(now),
            got_list: false,
        });
        self.resume_source = resume;
        self.send_viewport();
    }

    fn back_to_connect(&mut self, status: Option<String>, notice: Option<String>) {
        if let Ok(mut v) = self.video.lock() {
            *v = None;
        }
        self.busy_row = None;
        self.update(|m| {
            m.screen = Screen::Connect;
            m.stream = StreamModel::default();
            if notice.is_some() {
                m.connect.notice = notice;
            }
        });
        self.set_status(status);
    }

    fn disconnect(&mut self) {
        self.gen += 1; // a dial under way is forgotten
        self.dialing = false;
        self.reconnect = None;
        if let Some(live) = self.live.take() {
            live.session.close();
        }
        self.after_session_end();
        self.back_to_connect(None, None);
    }

    fn after_session_end(&mut self) {
        self.clip.reset();
        self.queue.reset();
        self.ledger.reset();
        let _ = self.audio_tx.send(AudioJob::Reset);
    }

    fn forget(&mut self, mac_id: &str) {
        if self.live.as_ref().is_some_and(|l| l.mac.mac_id == mac_id)
            || self
                .reconnect
                .as_ref()
                .is_some_and(|r| r.mac.mac_id == mac_id)
        {
            self.disconnect();
        }
        self.macs.retain(|m| m.mac_id != mac_id);
        if let Some(s) = &self.store {
            let _ = s.save_macs(&self.macs);
        }
    }

    fn start_ask(&mut self, name: String, target: String) {
        let Some(identity) = self.need_identity() else {
            return;
        };
        self.gen += 1;
        let gen = self.gen;
        self.update(|m| {
            m.connect.pairing = PairingState::Working(format!("Asking {name} to show its code…"))
        });
        self.busy_row = Some(format!("bonjour:{name}"));
        let tx = self.msg_tx.clone();
        let device = self.device_name.clone();
        let _ = std::thread::Builder::new()
            .name("sill-ask".into())
            .spawn(move || {
                let result = pair_client::ask(&target, &identity, &device, None, DIAL_TIMEOUT);
                let _ = tx.send(Msg::Asked(AskDone {
                    gen,
                    name,
                    target,
                    result,
                }));
            });
    }

    fn asked(&mut self, a: AskDone) {
        if a.gen != self.gen {
            return;
        }
        self.busy_row = None;
        let name = a.name;
        match a.result {
            Ok(AskAnswer::Shown { mac_fingerprint } | AskAnswer::OpenOnMac { mac_fingerprint }) => {
                self.pending_ask = Some(PendingAsk {
                    target: a.target,
                    fp: mac_fingerprint,
                });
                self.update(|m| m.connect.pairing = PairingState::CodeShown { mac: name });
            }
            Ok(AskAnswer::Locked { .. }) => {
                let w = format!("{name} is locked. Unlock it, then try again.");
                self.update(|m| m.connect.pairing = PairingState::Failed(w));
            }
            Err(e) => {
                let w = words::pair_failure(&e, &name);
                self.update(|m| m.connect.pairing = PairingState::Failed(w));
            }
        }
    }

    fn pair_code(&mut self, addr: &str, code: &str) {
        let Some(identity) = self.need_identity() else {
            return;
        };
        let parsed = match parse_address(addr) {
            Ok(p) => p,
            Err(_) => {
                self.update(|m| {
                    m.connect.pairing = PairingState::Failed(
                        "That address doesn’t look right. Check it on the Mac.".into(),
                    )
                });
                return;
            }
        };
        let (target, pin) = self.pairing_target(&parsed);
        self.gen += 1;
        let gen = self.gen;
        let name = parsed.host.clone();
        self.update(|m| m.connect.pairing = PairingState::Working(format!("Pairing with {name}…")));
        let tx = self.msg_tx.clone();
        let device = self.device_name.clone();
        let code = code.to_string();
        let _ = std::thread::Builder::new()
            .name("sill-pair".into())
            .spawn(move || {
                let result = pair_client::pair_with_code(
                    &target,
                    &code,
                    pin,
                    &identity,
                    &device,
                    None,
                    PAIR_TIMEOUT,
                );
                let _ = tx.send(Msg::Paired(PairDone { gen, result, name }));
            });
    }

    /// The target for a typed address: the one an ask answered (its port, its key) when the host
    /// is the same, else the typed one on the remote door's port.
    fn pairing_target(&self, p: &ParsedAddress) -> (String, Option<[u8; 32]>) {
        let host_of = |t: &str| -> String {
            t.rsplit_once(':')
                .map(|(h, _)| h.trim_matches(|c| c == '[' || c == ']').to_string())
                .unwrap_or_default()
        };
        if let Some(ask) = &self.pending_ask {
            if p.port.is_none() && host_of(&ask.target) == p.host {
                return (ask.target.clone(), Some(ask.fp));
            }
            if ask.target == p.text() {
                return (ask.target.clone(), Some(ask.fp));
            }
        }
        (
            ParsedAddress {
                port: Some(p.port.unwrap_or(7455)),
                ..p.clone()
            }
            .text(),
            None,
        )
    }

    fn pair_link(&mut self, link: &str) {
        let Some(identity) = self.need_identity() else {
            return;
        };
        self.gen += 1;
        let gen = self.gen;
        self.update(|m| m.connect.pairing = PairingState::Working("Pairing…".into()));
        let tx = self.msg_tx.clone();
        let device = self.device_name.clone();
        let link = link.to_string();
        let _ = std::thread::Builder::new()
            .name("sill-pair".into())
            .spawn(move || {
                let result =
                    pair_client::pair_with_link(&link, &identity, &device, None, PAIR_TIMEOUT);
                let _ = tx.send(Msg::Paired(PairDone {
                    gen,
                    result,
                    name: "the Mac".into(),
                }));
            });
    }

    fn paired(&mut self, p: PairDone) {
        if p.gen != self.gen {
            return;
        }
        match p.result {
            Ok(saved) => {
                match self.macs.iter_mut().find(|m| m.mac_id == saved.mac_id) {
                    Some(old) => *old = saved.clone(),
                    None => self.macs.push(saved.clone()),
                }
                if let Some(s) = &self.store {
                    let _ = s.save_macs(&self.macs);
                }
                self.pending_ask = None;
                let name = saved.name.clone();
                self.update(|m| m.connect.pairing = PairingState::Paired { mac: name.clone() });
                self.connect_row(&format!("saved:{}", saved.mac_id), false);
            }
            Err(e) => {
                let w = words::pair_failure(&e, &p.name);
                self.update(|m| m.connect.pairing = PairingState::Failed(w));
            }
        }
    }

    // --- the session ------------------------------------------------------------------------

    fn pump_session(&mut self) {
        let mut events = Vec::new();
        if let Some(l) = &self.live {
            while events.len() < 512 {
                match l.session.events.try_recv() {
                    Ok(e) => events.push(e),
                    Err(_) => break,
                }
            }
        }
        for e in events {
            self.on_event(e);
            if self.live.is_none() {
                break;
            }
        }
    }

    fn on_event(&mut self, e: SessionEvent) {
        match e {
            SessionEvent::WindowList(l) => self.on_window_list(l),
            SessionEvent::HostSettings(s) => {
                let mac = self
                    .live
                    .as_ref()
                    .map(|l| l.mac.name.clone())
                    .unwrap_or_default();
                let line = s.link.as_ref().and_then(words::link_line);
                if s.session_paired != self.session_paired {
                    self.session_paired = s.session_paired;
                    self.publish_cover();
                }
                self.ledger.on_state(s.clone(), &mac);
                self.update(|m| {
                    m.stream.settings = Some(s);
                    m.stream.link_line = line;
                });
                self.publish_ledger();
            }
            SessionEvent::MacInfo { verified, info, .. } => {
                if let (true, Some(info)) = (verified, info) {
                    self.learn_addresses(&info);
                }
            }
            SessionEvent::Goodbye(g) => self.goodbye = Some(g),
            SessionEvent::Pong { rtt_ms } => {
                self.last_rtt_ms = rtt_ms;
                self.worst_rtt_ms = self.worst_rtt_ms.max(rtt_ms);
            }
            SessionEvent::Message(m) => self.on_message(m),
            SessionEvent::Closed(reason) => self.on_closed(reason),
        }
    }

    fn on_window_list(&mut self, l: WindowList) {
        self.counters.window_lists.fetch_add(1, Ordering::Relaxed);
        let now = Instant::now();
        let first = self
            .live
            .as_mut()
            .map(|x| !std::mem::replace(&mut x.got_list, true))
            .unwrap_or(false);
        let why = l.locked_why();
        let locked = why.map(|w| words::line(&l.mac_name, w));
        let before = self.shade;
        self.shade = words::shade_after_list(self.shade, why);
        if self.shade == Some(words::Shade::Unlocking) && before != self.shade {
            self.unlocking_since = Some(now);
        }
        let cover = self.cover_model(&l.mac_name);
        let mac_clipboard = l.clipboard.unwrap_or(0) >= 1;
        if let Some(s) = l.active.ne(&StreamSource::None).then_some(l.active) {
            self.last_source = Some(s);
        }
        // subscriptions at the first list: the Mac's menus, and the clipboard if shared
        if first {
            self.send(MessageKind::FetchMenu, wire::encode(&FetchMenu::default()));
        }
        self.host_clipboard = l.clipboard;
        if let Some(c) = self
            .clip
            .sync_subscription(l.clipboard, self.prefs.share_clipboard, now)
        {
            self.send(MessageKind::Clipboard, c);
        }
        // what streams: the resumed source, else the Desktop (at most every 10 s) until a person chooses
        if l.active == StreamSource::None && why != Some(sill_proto::switcher::LockedWhy::OtherUser)
        {
            let pick = match self.resume_source.take() {
                Some(s) => Some(s),
                None if !self.user_chose
                    && self
                        .last_auto_desktop
                        .is_none_or(|t| now.duration_since(t) >= AUTO_DESKTOP_EVERY) =>
                {
                    self.last_auto_desktop = Some(now);
                    Some(StreamSource::Desktop)
                }
                None => None,
            };
            if let (Some(s), Some(live)) = (pick, &self.live) {
                live.session.select_source(s);
            }
        } else {
            self.resume_source = None;
        }
        let window_ids: Vec<u32> = l.windows.iter().map(|w| w.id).collect();
        self.update(|m| {
            let s = &mut m.stream;
            s.mac_name = l.mac_name.clone();
            s.host_version = l.host_version.clone();
            s.windows = l.windows.clone();
            s.active = Some(l.active);
            s.locked = locked;
            s.cover = cover;
            s.mac_clipboard = mac_clipboard;
            s.mac_key_repeat = l.key_repeat.unwrap_or(0) >= 1;
            s.mac_gestures = l.gestures.unwrap_or(0) >= 1;
            s.thumbnails.retain(|id, _| window_ids.contains(id));
        });
    }

    fn learn_addresses(&mut self, info: &MacInfo) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if !info.remote_access {
            return;
        }
        let mut changed = false;
        for a in &info.addresses {
            let port = a
                .port
                .and_then(|p| u16::try_from(p).ok())
                .unwrap_or(info.remote_port.try_into().unwrap_or(7455));
            let raw = if a.port.is_some() {
                format!("{}:{}", a.host, port)
            } else {
                a.host.clone()
            };
            let Ok(p) = parse_address(&raw) else { continue };
            let text = ParsedAddress { port: p.port, ..p }.text();
            if !live.mac.addresses.contains(&text) && !live.mac.addresses.contains(&a.host) {
                live.mac.addresses.push(text);
                changed = true;
            }
        }
        if changed {
            let mac = live.mac.clone();
            if let Some(m) = self.macs.iter_mut().find(|m| m.mac_id == mac.mac_id) {
                m.addresses = mac.addresses;
                if let Some(s) = &self.store {
                    let _ = s.save_macs(&self.macs);
                }
            }
        }
    }

    fn cover_model(&self, mac: &str) -> Option<Cover> {
        self.shade.map(|sh| Cover {
            title: words::title(mac, sh),
            detail: words::detail(sh, self.session_paired),
        })
    }

    fn publish_cover(&self) {
        let mac = self
            .live
            .as_ref()
            .map(|l| l.mac.name.clone())
            .unwrap_or_default();
        let cover = self.cover_model(&mac);
        self.update(|m| m.stream.cover = cover);
    }

    fn publish_ledger(&self) {
        let (shown, problem) = (
            self.ledger.shown(),
            self.ledger.problem().map(str::to_string),
        );
        self.update(|m| {
            m.stream.settings_shown = shown;
            m.stream.settings_problem = problem;
        });
    }

    fn on_message(&mut self, m: Message) {
        let now = Instant::now();
        match m.kind {
            MessageKind::ParameterSets => {
                self.counters.sets_in.fetch_add(1, Ordering::Relaxed);
                if feed_trace() {
                    eprintln!("feed {:.3} sets {}B", trace_clock(), m.payload.len());
                }
                if let Some(sets) = ParameterSets::decode(&m.payload) {
                    self.queue.push_control(Job::Sets(sets));
                    if let Some(l) = self.live.as_mut() {
                        l.feed.need_keyframe(now);
                    }
                }
            }
            MessageKind::Frame => {
                self.counters.frames_in.fetch_add(1, Ordering::Relaxed);
                self.counters
                    .bytes_in
                    .fetch_add(m.payload.len() as u64, Ordering::Relaxed);
                if m.is_keyframe {
                    self.counters.keyframes_in.fetch_add(1, Ordering::Relaxed);
                }
                let shade = words::shade_after_frame(self.shade, m.is_keyframe);
                if shade != self.shade {
                    self.shade = shade;
                    self.publish_cover();
                }
                let Some(live) = self.live.as_mut() else {
                    return;
                };
                let room = self.queue.has_room();
                let act = live.feed.frame(m.is_keyframe, room, now);
                if feed_trace() {
                    eprintln!(
                        "feed {:.3} key={} {}B room={room} -> send={} ask={}",
                        trace_clock(),
                        m.is_keyframe,
                        m.payload.len(),
                        act.send,
                        act.request_keyframe
                    );
                }
                if act.send {
                    if !self.queue.push_frame(m.payload, m.is_keyframe) {
                        self.counters.frames_dropped.fetch_add(1, Ordering::Relaxed);
                        self.counters.dropped_full.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    self.counters.frames_dropped.fetch_add(1, Ordering::Relaxed);
                    if self.queue.has_room() {
                        self.counters
                            .dropped_waiting
                            .fetch_add(1, Ordering::Relaxed);
                    } else {
                        self.counters.dropped_full.fetch_add(1, Ordering::Relaxed);
                    }
                }
                if act.request_keyframe {
                    self.counters
                        .keyframe_requests
                        .fetch_add(1, Ordering::Relaxed);
                    live.session.request_keyframe();
                }
            }
            MessageKind::Thumbnail => {
                if let Some((id, jpeg)) = blobs::decode_thumbnail(&m.payload) {
                    let _ = self.image_tx.send(ImageJob {
                        kind: ImageKind::Thumb(id),
                        bytes: jpeg.to_vec(),
                    });
                }
            }
            MessageKind::AppIcon => {
                if let Some((id, png)) = blobs::decode_icon(&m.payload) {
                    let _ = self.image_tx.send(ImageJob {
                        kind: ImageKind::Icon(id),
                        bytes: png.to_vec(),
                    });
                }
            }
            MessageKind::AppList => {
                if let Some(apps) = wire::decode::<Vec<AppInfo>>(&m.payload) {
                    self.update(|md| md.stream.apps = apps);
                }
            }
            MessageKind::CursorShape => {
                if let Some(c) = blobs::decode_cursor_shape(&m.payload) {
                    let kind = ImageKind::Cursor {
                        hotspot: c.hotspot,
                        size: c.size,
                    };
                    let _ = self.image_tx.send(ImageJob {
                        kind,
                        bytes: c.png.to_vec(),
                    });
                }
            }
            MessageKind::MacPointer => {
                if let Some(p) = wire::decode_object::<MacPointer>(&m.payload) {
                    let pos = p.position();
                    self.update(|md| md.stream.cursor.mac_position = pos);
                }
            }
            MessageKind::MacMenu => {
                if let Some(menu) = wire::decode_object::<MacMenu>(&m.payload) {
                    self.on_menu(menu);
                }
            }
            MessageKind::Clipboard => {
                if let Some(c) = wire::decode_object::<Clipboard>(&m.payload) {
                    self.on_clipboard(c);
                }
            }
            MessageKind::Audio => {
                self.counters.audio_packets.fetch_add(1, Ordering::Relaxed);
                match AudioMessage::parse(&m.payload) {
                    Some(AudioMessage::Format(f)) => {
                        let _ = self.audio_tx.send(AudioJob::Format(f));
                    }
                    Some(AudioMessage::Packets(p)) => {
                        let _ = self.audio_tx.send(AudioJob::Packets {
                            timestamp: m.timestamp,
                            segment_start: p.segment_start,
                            packets: p.packets,
                        });
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    fn on_menu(&mut self, menu: MacMenu) {
        if menu.menus.is_some() {
            let changed = menu.version != self.menu_version;
            self.menu_version = menu.version;
            self.update(|m| {
                if changed {
                    m.stream.menu_items.clear();
                }
                m.stream.menus = Some(menu);
            });
        } else if let Some(token) = menu.answering {
            if let Some(id) = self.menu_tokens.remove(&token) {
                if menu.items.is_some() {
                    self.update(|m| {
                        m.stream.menu_items.insert(id, menu);
                    });
                }
            }
        }
    }

    fn on_clipboard(&mut self, c: Clipboard) {
        if self
            .clip
            .write_from_mac(&c, self.prefs.share_clipboard, self.host_clipboard)
        {
            self.counters.clip_received.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn image(&mut self, i: ImageDone) {
        let ImageDone { kind, picture } = i;
        self.update(|m| match kind {
            ImageKind::Thumb(id) => {
                m.stream.thumbnails.insert(id, picture);
            }
            ImageKind::Icon(id) => {
                m.stream.icons.insert(id, picture);
            }
            ImageKind::Cursor { hotspot, size } => {
                m.stream.cursor.shape = Some(picture);
                m.stream.cursor.hotspot = hotspot;
                m.stream.cursor.size = size;
            }
        });
    }

    fn on_closed(&mut self, reason: CloseReason) {
        let Some(live) = self.live.take() else { return };
        eprintln!("session: {} closed: {reason:?}", live.mac.name);
        self.after_session_end();
        if reason == CloseReason::Requested {
            return;
        }
        let saved = self.macs.iter().any(|m| m.mac_id == live.mac.mac_id);
        let goodbye = self.goodbye.take();
        let out = words::outcome_for_close(&reason, goodbye.as_ref(), &live.mac.name, saved);
        if out.reconnect {
            let now = Instant::now();
            let resume = self.last_source;
            self.reconnect = Some(Reconnect {
                mac: live.mac.clone(),
                remote_allowed: out.remote_allowed,
                failed: 0,
                last_dial: None,
                next_at: now,
                dialing: false,
                resume,
            });
            let line = reconnect::line(&live.mac.name, 0);
            self.update(|m| m.stream.reconnecting = Some(line));
        } else {
            let notice = out.notice.then(|| out.text.clone());
            self.back_to_connect(Some(out.text), notice);
        }
    }

    // --- timers -----------------------------------------------------------------------------

    fn tick(&mut self, now: Instant) {
        self.refresh_rows(now);
        // the settings ledger
        if self.live.is_some() {
            let name = self
                .live
                .as_ref()
                .map(|l| l.mac.name.clone())
                .unwrap_or_default();
            if self.ledger.expire(now, self.worst_rtt_ms, &name) {
                self.publish_ledger();
            }
        }
        // the decoder's own trouble
        if self.flags.error.swap(false, Ordering::Relaxed) {
            if let Some(l) = self.live.as_mut() {
                l.feed.need_keyframe(now);
            }
        }
        if let Some(l) = self.live.as_mut() {
            if l.feed.tick(now) {
                self.counters
                    .keyframe_requests
                    .fetch_add(1, Ordering::Relaxed);
                l.session.request_keyframe();
            }
        }
        // once a second: fps and round trip
        if now.duration_since(self.last_second) >= Duration::from_secs(1) {
            let secs = now.duration_since(self.last_second).as_secs_f64();
            self.last_second = now;
            let decoded = self.counters.frames_decoded.load(Ordering::Relaxed);
            let fps = (decoded - self.decoded_at_last_second) as f64 / secs;
            let micros = self.counters.decode_micros.load(Ordering::Relaxed);
            let decode_ms = if decoded > self.decoded_at_last_second {
                (micros - self.decode_micros_at_last_second) as f64
                    / (decoded - self.decoded_at_last_second) as f64
                    / 1000.0
            } else {
                0.0
            };
            self.decode_micros_at_last_second = micros;
            if decoded > self.decoded_at_last_second {
                self.queue.set_capacity(feed::capacity(Some(decode_ms)));
            }
            self.decoded_at_last_second = decoded;
            if self.live.is_some() {
                let (rtt, unavailable) = (
                    self.last_rtt_ms,
                    self.flags.unavailable.lock().ok().and_then(|u| u.clone()),
                );
                let _ = self.audio_tx.send(AudioJob::LagMs(rtt / 2.0 + decode_ms));
                self.worst_rtt_ms = 0.0;
                self.update(|m| {
                    m.stream.fps = fps;
                    m.stream.rtt_ms = rtt;
                    if let Some(why) = unavailable {
                        m.stream.link_line = Some(format!("The picture can’t be decoded: {why}"));
                    }
                });
            }
        }
        if self.shade == Some(words::Shade::Unlocking)
            && self
                .unlocking_since
                .is_some_and(|t| now.duration_since(t) >= words::UNLOCKING_TIMEOUT)
        {
            self.shade = None;
            self.publish_cover();
        }
        if self.live.is_some() {
            let on = self.prefs.share_clipboard;
            if let Some(c) = self.clip.look(on, self.host_clipboard, now) {
                self.counters.clip_sent.fetch_add(1, Ordering::Relaxed);
                self.send(MessageKind::Clipboard, c);
            }
        }
        self.reconnect_tick(now);
    }

    fn refresh_rows(&mut self, now: Instant) {
        let found = self.discovery.found();
        let mut rows = crate::discovery::rows(&found, &self.macs);
        for r in &mut rows {
            r.busy = self.busy_row.as_deref() == Some(r.id.as_str());
        }
        let nothing = found.is_empty()
            && now.duration_since(self.discovery.started_at()) >= Duration::from_secs(3);
        if rows != self.last_rows
            || self
                .shared
                .lock()
                .map(|m| m.connect.nothing_found != nothing)
                .unwrap_or(false)
        {
            self.last_rows = rows.clone();
            self.update(|m| {
                m.connect.rows = rows;
                m.connect.nothing_found = nothing;
            });
        }
    }

    fn reconnect_tick(&mut self, now: Instant) {
        let found = self.discovery.found();
        let Some(r) = self.reconnect.as_mut() else {
            return;
        };
        if r.dialing || self.dialing || now < r.next_at {
            return;
        }
        let targets = targets::reconnect_targets(&r.mac, &found, r.remote_allowed);
        r.last_dial = Some(now);
        if targets.is_empty() {
            self.reconnect_failed(now);
            return;
        }
        r.dialing = true;
        let mac = r.mac.clone();
        self.start_dial(mac, targets, true);
    }

    fn reconnect_failed(&mut self, now: Instant) {
        let Some(r) = self.reconnect.as_mut() else {
            return;
        };
        r.dialing = false;
        r.failed += 1;
        let name = r.mac.name.clone();
        match reconnect::next_delay(r.failed) {
            Some(delay) => {
                r.next_at = r.last_dial.unwrap_or(now) + delay;
                let line = reconnect::line(&name, r.failed);
                self.update(|m| m.stream.reconnecting = Some(line));
            }
            None => {
                self.reconnect = None;
                self.back_to_connect(Some(reconnect::line(&name, reconnect::DIALS)), None);
            }
        }
    }
}
