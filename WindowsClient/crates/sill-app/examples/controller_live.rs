//! Headless live run of the controller against the owner's paired Mac (never pairs, no code).
//!
//!   cargo run --release -p sill-app --example controller_live [-- MACID [SECONDS]]
//!
//! Without SILL_HOME it makes a temporary one and copies the development identity and saved Mac
//! from WindowsClient/target (sill-dev-identity/key.pk8, sill-dev-macs.json) into it.
#![allow(dead_code)]

#[path = "../src/clipboard.rs"]
mod clipboard;
#[path = "../src/controller.rs"]
mod controller;
#[path = "../src/discovery.rs"]
mod discovery;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/store.rs"]
mod store;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use model::{Command, Screen};
use sill_proto::switcher::StreamSource;

fn main() {
    let mut args = std::env::args().skip(1);
    let mac_id = args.next().unwrap_or_else(|| "9WMQ8KK6CN9J7K99".into());
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    if std::env::var_os("SILL_HOME").is_none() {
        let dir = std::env::temp_dir().join(format!("sill-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(
            target.join("sill-dev-identity/key.pk8"),
            dir.join("identity.key"),
        )
        .unwrap();
        std::fs::copy(target.join("sill-dev-macs.json"), dir.join("macs.json")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                dir.join("identity.key"),
                std::fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        std::env::set_var("SILL_HOME", &dir);
        println!("SILL_HOME {}", dir.display());
    }
    let waker: model::Waker = Arc::new(|| {});
    let (h, counters) = controller::start_with_counters(waker);
    let t0 = Instant::now();
    let summary = |label: &str| {
        let m = h.shared.lock().unwrap().clone();
        let c = &counters;
        let s = m.stream.settings.as_ref().map(|s| {
            format!(
                "{} fps limit, {} bps, scale {}",
                s.settings.max_fps, s.settings.bitrate, s.settings.capture_scale
            )
        });
        println!(
            "[{:5.1}s] {label}: screen {:?}, mac {:?}, windows {}, apps {}, thumbs {}, icons {}, active {:?}, locked {:?}, cover {:?}, settings {:?}, \
             clipboard {} keyRepeat {} gestures {}, rtt {:.0} ms, decoded fps {:.1}; received frames {} (key {}), sets {}, \
             dropped {} (full {}, awaiting keyframe {}), decode avg {} us max {} us, kf requests {}, {} KB, audio msgs {}, lists {}, cursor {}, reconnecting {:?}, line {:?}, status {:?}",
            t0.elapsed().as_secs_f64(),
            m.screen,
            m.stream.mac_name,
            m.stream.windows.len(),
            m.stream.apps.len(),
            m.stream.thumbnails.len(),
            m.stream.icons.len(),
            m.stream.active,
            m.stream.locked,
            m.stream.cover,
            s,
            m.stream.mac_clipboard,
            m.stream.mac_key_repeat,
            m.stream.mac_gestures,
            m.stream.rtt_ms,
            m.stream.fps,
            c.frames_in.load(Ordering::Relaxed),
            c.keyframes_in.load(Ordering::Relaxed),
            c.sets_in.load(Ordering::Relaxed),
            c.frames_dropped.load(Ordering::Relaxed),
            c.dropped_full.load(Ordering::Relaxed),
            c.dropped_waiting.load(Ordering::Relaxed),
            c.decode_micros.load(Ordering::Relaxed) / c.frames_decoded.load(Ordering::Relaxed).max(1),
            c.decode_max_micros.load(Ordering::Relaxed),
            c.keyframe_requests.load(Ordering::Relaxed),
            c.bytes_in.load(Ordering::Relaxed) / 1024,
            c.audio_packets.load(Ordering::Relaxed),
            c.window_lists.load(Ordering::Relaxed),
            m.stream.cursor.shape.is_some(),
            m.stream.reconnecting,
            m.stream.link_line,
            m.connect.status,
        );
    };
    summary("start");
    let row = format!("saved:{mac_id}");
    h.commands.send(Command::Connect(row)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let m = h.shared.lock().unwrap().clone();
        if m.screen == Screen::Stream && counters.window_lists.load(Ordering::Relaxed) > 0 {
            break;
        }
        if Instant::now() > deadline {
            summary("TIMEOUT waiting for the stream screen and a window list");
            let _ = h.commands.send(Command::Quit);
            std::process::exit(1);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    summary("connected");
    h.commands
        .send(Command::Viewport {
            width: 1280.0,
            height: 800.0,
            scale: None,
        })
        .unwrap();
    h.commands
        .send(Command::Pick(StreamSource::Desktop))
        .unwrap();
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_secs(1));
        summary("running");
    }
    if std::env::var_os("TWICE").is_some() {
        // the same again in one process: a second session over the first one's decoder and queue
        h.commands.send(Command::Disconnect).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let lists = counters.window_lists.load(Ordering::Relaxed);
        h.commands
            .send(Command::Connect(format!("saved:{mac_id}")))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(25);
        while counters.window_lists.load(Ordering::Relaxed) == lists && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        summary("second session");
        for _ in 0..5 {
            std::thread::sleep(Duration::from_secs(1));
            summary("second running");
        }
    }
    let before = h
        .video
        .lock()
        .unwrap()
        .as_ref()
        .map(|f| (f.width, f.height, f.serial));
    println!("video slot: {before:?}");
    h.commands.send(Command::Disconnect).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    summary("disconnected");
    h.commands.send(Command::Quit).unwrap();
    std::thread::sleep(Duration::from_millis(300));
}
