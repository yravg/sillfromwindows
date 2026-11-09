//! JSON payload tests. Expected strings are the ones Swift's encoder/decoder produce or accept in
//! Tests/checks/compatibility and Tests/checks/protocol (cited per test), compared as JSON values
//! (Swift's key order is unspecified).

use serde_json::{json, Value};
use sill_proto::audio::*;
use sill_proto::blobs::*;
use sill_proto::compat::*;
use sill_proto::input::*;
use sill_proto::menu::*;
use sill_proto::remote::*;
use sill_proto::settings::*;
use sill_proto::small::*;
use sill_proto::switcher::*;
use sill_proto::viewport::*;
use sill_proto::wire;

fn val<T: serde::Serialize>(t: &T) -> Value {
    serde_json::to_value(t).unwrap()
}
fn dec<T: serde::de::DeserializeOwned>(s: &str) -> Option<T> {
    wire::decode_object(s.as_bytes())
}
/// Encode, decode, compare: every type round-trips through its own JSON.
fn round<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(t: &T) {
    let back: T = wire::decode(&wire::encode(t)).expect("decodes");
    assert_eq!(&back, t);
}

fn vp(width: f64, height: f64, scale: Option<f64>) -> Viewport {
    Viewport {
        width,
        height,
        scale,
        local_cursor: None,
        fps: None,
    }
}

// ---- StreamSource, WindowList -------------------------------------------------------------------

#[test]
fn stream_source_shapes() {
    // sillclient.py:190-195.
    assert_eq!(val(&StreamSource::None), json!({"none": {}}));
    assert_eq!(val(&StreamSource::Desktop), json!({"desktop": {}}));
    assert_eq!(
        val(&StreamSource::Window(123)),
        json!({"window": {"_0": 123}})
    );
    assert_eq!(
        dec::<StreamSource>(r#"{"desktop":{}}"#),
        Some(StreamSource::Desktop)
    );
    assert_eq!(
        dec::<StreamSource>(r#"{"window":{"_0":4294967295}}"#),
        Some(StreamSource::Window(u32::MAX))
    );
    assert_eq!(
        dec::<StreamSource>(r#"{"desktop":{"later":1}}"#),
        Some(StreamSource::Desktop)
    );
    assert_eq!(dec::<StreamSource>(r#"{"window":{"_0":-1}}"#), None);
    assert_eq!(dec::<StreamSource>(r#"{"window":{}}"#), None);
    assert_eq!(dec::<StreamSource>(r#"{"camera":{}}"#), None);
    assert_eq!(dec::<StreamSource>("[]"), None);
    for s in [
        StreamSource::None,
        StreamSource::Desktop,
        StreamSource::Window(0),
        StreamSource::Window(7),
    ] {
        round(&s);
    }
}

#[test]
fn window_list_old_host_decodes_with_nil_fields() {
    // Tests/checks/compatibility:139 `oldList`.
    let l: WindowList =
        dec(r#"{"macName":"Mac mini","windows":[],"active":{"desktop":{}},"launchID":"L"}"#)
            .unwrap();
    assert_eq!(l.mac_name, "Mac mini");
    assert_eq!(l.active, StreamSource::Desktop);
    assert_eq!(l.launch_id.as_deref(), Some("L"));
    assert_eq!(l.host_version, None);
    assert_eq!(l.protocol, None);
    assert_eq!(l.gestures, None);
    assert_eq!(l.clipboard, None);
    assert_eq!(l.key_repeat, None);
    assert_eq!(l.locked_why(), None);
}

#[test]
fn window_list_full_shape_and_nil_omission() {
    let l = WindowList {
        mac_name: "Mac mini".into(),
        windows: vec![WindowInfo {
            id: 104,
            title: "Doc".into(),
            app_name: "TextEdit".into(),
            bundle_id: "com.apple.TextEdit".into(),
            width: 800,
            height: 600,
        }],
        active: StreamSource::Window(104),
        launch_id: Some("L".into()),
        host_version: Some("0.4.0".into()),
        protocol: Some(1),
        gestures: Some(1),
        clipboard: Some(1),
        key_repeat: Some(1),
        locked: None,
        off_console: None,
    };
    assert_eq!(
        val(&l),
        json!({
            "macName": "Mac mini",
            "windows": [{"id": 104, "title": "Doc", "appName": "TextEdit", "bundleID": "com.apple.TextEdit", "width": 800, "height": 600}],
            "active": {"window": {"_0": 104}},
            "launchID": "L", "hostVersion": "0.4.0", "protocol": 1, "gestures": 1, "clipboard": 1, "keyRepeat": 1
        })
    );
    round(&l);
    // Minimal list: only the three required keys.
    let min = WindowList {
        mac_name: "M".into(),
        windows: vec![],
        active: StreamSource::None,
        launch_id: None,
        host_version: None,
        protocol: None,
        gestures: None,
        clipboard: None,
        key_repeat: None,
        locked: None,
        off_console: None,
    };
    assert_eq!(
        val(&min),
        json!({"macName": "M", "windows": [], "active": {"none": {}}})
    );
}

#[test]
fn window_list_tolerates_unknown_fields_and_null_optionals() {
    let l: WindowList = dec(r#"{"macName":"M","windows":[{"id":1,"title":"","appName":"","bundleID":"","width":1,"height":2,"later":true}],"active":{"none":{}},"launchID":null,"future":{"a":[1]}}"#).unwrap();
    assert_eq!(l.launch_id, None);
    assert_eq!(l.windows[0].height, 2);
}

#[test]
fn window_list_missing_required_or_wrong_types_do_not_decode() {
    assert!(dec::<WindowList>(r#"{"windows":[],"active":{"none":{}}}"#).is_none());
    assert!(dec::<WindowList>(r#"{"macName":"M","windows":[]}"#).is_none());
    assert!(dec::<WindowList>(r#"{"macName":3,"windows":[],"active":{"none":{}}}"#).is_none());
    assert!(dec::<WindowList>("[]").is_none());
    assert!(dec::<WindowList>("not json").is_none());
}

#[test]
fn window_list_locked_reasons() {
    // docs/locked-mac-plan.md; LockedMac.why (Switcher.swift:101).
    let base = r#"{"macName":"M","windows":[],"active":{"none":{}}"#;
    let l: WindowList = dec(&format!(r#"{base},"locked":true}}"#)).unwrap();
    assert_eq!(l.locked_why(), Some(LockedWhy::ScreenLocked));
    let l: WindowList = dec(&format!(r#"{base},"locked":true,"offConsole":true}}"#)).unwrap();
    assert_eq!(l.locked_why(), Some(LockedWhy::OtherUser));
    let l: WindowList = dec(&format!(r#"{base},"offConsole":true}}"#)).unwrap();
    assert_eq!(l.locked_why(), None);
}

#[test]
fn app_list_launch_and_window_command() {
    assert_eq!(
        val(&vec![AppInfo {
            name: "Safari".into(),
            bundle_id: "com.apple.Safari".into()
        }]),
        json!([{"name": "Safari", "bundleID": "com.apple.Safari"}])
    );
    assert_eq!(
        val(&LaunchApp {
            bundle_id: "com.apple.TextEdit".into()
        }),
        json!({"bundleID": "com.apple.TextEdit"})
    );
    // Switcher.swift:172-177: `{"id":N,"action":"close"|"minimize"|"fullScreen"}`.
    assert_eq!(
        val(&WindowCommand {
            id: 5,
            action: WindowAction::FullScreen
        }),
        json!({"id": 5, "action": "fullScreen"})
    );
    assert_eq!(
        val(&WindowCommand {
            id: 5,
            action: WindowAction::Minimize
        }),
        json!({"id": 5, "action": "minimize"})
    );
    assert_eq!(
        dec::<WindowCommand>(r#"{"id":9,"action":"close"}"#),
        Some(WindowCommand {
            id: 9,
            action: WindowAction::Close
        })
    );
    assert_eq!(dec::<WindowCommand>(r#"{"id":9,"action":"zoom"}"#), None);
}

// ---- Hello, Goodbye ----------------------------------------------------------------------------

#[test]
fn hello_keys_and_tolerance() {
    // Tests/checks/compatibility:110-116, 185-194.
    let h = Hello {
        app_version: Some("1.0".into()),
        build: Some("42".into()),
        protocol: Some(1),
        device: Some("iPad (iPad14,1)".into()),
        audio: None,
    };
    assert_eq!(
        val(&h),
        json!({"appVersion": "1.0", "build": "42", "device": "iPad (iPad14,1)", "protocol": 1})
    );
    assert_eq!(dec::<Hello>("{}"), Some(Hello::default()));
    assert_eq!(
        dec::<Hello>(r#"{"appVersion":"1.2","protocol":1,"extra":[1,2]}"#),
        Some(Hello {
            app_version: Some("1.2".into()),
            protocol: Some(1),
            ..Hello::default()
        })
    );
    assert_eq!(
        dec::<Hello>(r#"{"appVersion":12}"#),
        None,
        "a number as the version does not decode"
    );
    assert_eq!(dec::<Hello>("[]"), None);
    let sound = Hello {
        app_version: Some("0.6".into()),
        build: Some("7".into()),
        protocol: Some(1),
        device: Some("iPad (iPad14,1)".into()),
        audio: Some(vec!["aac-eld".into()]),
    };
    assert_eq!(
        val(&sound),
        json!({"appVersion": "0.6", "audio": ["aac-eld"], "build": "7", "device": "iPad (iPad14,1)", "protocol": 1})
    );
    round(&sound);
    assert_eq!(
        val(&Hello {
            app_version: Some("1.0".into()),
            ..Hello::default()
        }),
        json!({"appVersion": "1.0"}),
        "no audio key"
    );
    assert_eq!(
        dec::<Hello>(r#"{"audio":["opus","aac-eld"],"later":1}"#)
            .unwrap()
            .audio,
        Some(vec!["opus".to_string(), "aac-eld".to_string()])
    );
    assert_eq!(
        dec::<Hello>(r#"{"audio":"aac-eld"}"#),
        None,
        "codecs as one string do not decode"
    );
    assert_eq!(PROTOCOL_CURRENT, 1);
}

#[test]
fn goodbye_shapes() {
    // Tests/checks/compatibility:119-131.
    let update = Goodbye {
        reason: "update".into(),
        message: Some(
            "Update Sill on your iPad to keep using Mac mini. It needs version 1.2 or later."
                .into(),
        ),
        minimum_version: Some("1.2".into()),
        reconnect: Some(false),
    };
    assert_eq!(
        val(&update),
        json!({
            "message": "Update Sill on your iPad to keep using Mac mini. It needs version 1.2 or later.",
            "minimumVersion": "1.2", "reason": "update", "reconnect": false })
    );
    assert_eq!(
        dec::<Goodbye>(r#"{"reason":"quit"}"#),
        Some(Goodbye {
            reason: "quit".into(),
            message: None,
            minimum_version: None,
            reconnect: None
        })
    );
    assert_eq!(
        dec::<Goodbye>(r#"{"message":"x"}"#),
        None,
        "no reason: no goodbye"
    );
    let later = r#"{"reason":"pairAgain","message":"Pair again.","reconnect":false}"#;
    assert_eq!(dec::<Goodbye>(later).unwrap().reason, "pairAgain");
    // Tests/checks/protocol "kind 22 decodes an unknown reason as a string".
    assert_eq!(
        dec::<Goodbye>(r#"{"reason":"later","x":1}"#)
            .unwrap()
            .reason,
        "later"
    );
    assert_eq!(
        val(&Goodbye {
            reason: "removed".into(),
            message: None,
            minimum_version: None,
            reconnect: None
        }),
        json!({"reason": "removed"})
    );
    for r in [
        Goodbye::REMOVED,
        Goodbye::REMOTE_OFF,
        Goodbye::INTERNET_OFF,
        Goodbye::QUIT,
        Goodbye::BUSY,
        Goodbye::PAIRING_REQUIRED,
        Goodbye::UPDATE,
    ] {
        round(&Goodbye {
            reason: r.into(),
            message: None,
            minimum_version: None,
            reconnect: None,
        });
    }
}

#[test]
fn sill_version_cases() {
    let v = |s: &str| SillVersion::parse(s);
    assert!(v("0.10").unwrap() > v("0.9").unwrap());
    assert_eq!(v("v1.2.0"), v("1.2"));
    assert_eq!(v("V0.6.0 (beta)").unwrap().components(), &[0, 6]);
    assert_eq!(v(" 1.2 ").unwrap().to_string(), "1.2");
    assert_eq!(v("1").unwrap().to_string(), "1.0");
    assert_eq!(v("0").unwrap(), SillVersion::zero());
    assert_eq!(v("1.2.").unwrap(), v("1.2").unwrap());
    assert!(v("1.0.1").unwrap() > v("1.0").unwrap());
    assert!(v("1.0").unwrap() < v("1.0.0.1").unwrap());
    assert!(v("").is_none() && v("abc").is_none() && v("nightly").is_none() && v(".5").is_none());
    assert!(v("1..2").is_none());
    assert!(v("1234567890.1").is_none(), "a part over 9 digits");
    assert!(
        v("1.2.3.4.5.6.7.8").is_some() && v("1.2.3.4.5.6.7.8.9").is_none(),
        "at most 8 parts"
    );
    assert!(v("0.5.5").unwrap() < v("0.6.0").unwrap());
}

// ---- Viewport, ClientStats ---------------------------------------------------------------------

#[test]
fn viewport_shapes() {
    // sillclient.py:714-716: scale null decodes as none; fps present.
    let v: Viewport = dec(r#"{"width":1117,"height":642,"scale":null,"fps":60}"#).unwrap();
    assert_eq!(
        v,
        Viewport {
            width: 1117.0,
            height: 642.0,
            scale: None,
            local_cursor: None,
            fps: Some(60)
        }
    );
    assert_eq!(
        val(&v),
        json!({"width": 1117.0, "height": 642.0, "fps": 60})
    );
    let w = Viewport {
        width: 800.5,
        height: 600.0,
        scale: Some(1.5),
        local_cursor: Some(false),
        fps: None,
    };
    assert_eq!(
        val(&w),
        json!({"width": 800.5, "height": 600.0, "scale": 1.5, "localCursor": false})
    );
    round(&w);
    assert!(dec::<Viewport>(r#"{"height":1}"#).is_none());
}

#[test]
fn viewport_bounds_rule() {
    // Viewport.swift:45-96; Tests/checks/viewport-bounds (security scan finding 4).
    assert_eq!(ViewportBounds::refusal(&vp(1.0, 16384.0, Some(0.25))), None);
    assert_eq!(ViewportBounds::refusal(&vp(16384.0, 1.0, Some(4.0))), None);
    assert_eq!(
        ViewportBounds::refusal(&vp(1e300, 1e300, None)).unwrap(),
        "width 1e+300 (1 to 16384 points), height 1e+300 (1 to 16384 points)"
    );
    assert_eq!(
        ViewportBounds::refusal(&vp(100.0, 100.0, Some(1e-300))).unwrap(),
        "scale 1e-300 (0.25 to 4)"
    );
    let nan = vp(f64::NAN, 100.0, None);
    assert!(ViewportBounds::refusal(&nan).is_some());
    assert_eq!(
        ViewportBounds::refusal(&vp(-1.0, 0.5, Some(0.2))).unwrap(),
        "width -1 (1 to 16384 points), height 0.5 (1 to 16384 points), scale 0.2 (0.25 to 4)"
    );
    // held: clamps what a device sends; unusable sides give none.
    let held = ViewportBounds::held(&Viewport {
        width: 0.5,
        height: 99999.0,
        scale: Some(9.0),
        local_cursor: None,
        fps: Some(60),
    })
    .unwrap();
    assert_eq!(
        (held.width, held.height, held.scale, held.fps),
        (1.0, 16384.0, Some(4.0), Some(60))
    );
    assert!(ViewportBounds::held(&nan).is_none());
    assert!(ViewportBounds::held(&vp(0.0, 1.0, None)).is_none());
    assert!(ViewportBounds::held(&vp(f64::INFINITY, 1.0, None)).is_none());
    assert_eq!(
        ViewportBounds::held(&vp(5.0, 5.0, Some(f64::NAN)))
            .unwrap()
            .scale,
        None
    );
    assert_eq!(
        ViewportBounds::held(&vp(5.0, 5.0, Some(-1.0)))
            .unwrap()
            .scale,
        None
    );
    // Anything held is accepted by the host's rule.
    for (w, h, sc) in [(0.01, 1e9, 1e-9), (1e5, 0.4, 1e9), (3.3, 7.7, 2.0)] {
        let held = ViewportBounds::held(&vp(w, h, Some(sc))).unwrap();
        assert_eq!(ViewportBounds::refusal(&held), None);
    }
    assert_eq!(ViewportBounds::whole(1117.9), "1117");
    assert_eq!(ViewportBounds::whole(-3.9), "-3");
    assert_eq!(ViewportBounds::whole(1e300), "1e+300");
}

#[test]
fn client_stats_shapes() {
    // Tests/checks/compatibility:267-273; sillclient.py:851.
    let q = ClientStats {
        fps: 0,
        frame_age_ms: 0,
        rtt_ms: 0,
        device: "Windows PC".into(),
        frame_age_max_ms: None,
        rtt_max_ms: None,
        audio_behind_ms: None,
        audio_late: None,
    };
    assert_eq!(
        val(&q),
        json!({"fps": 0, "frameAgeMs": 0, "rttMs": 0, "device": "Windows PC"})
    );
    let heard = ClientStats {
        fps: 60,
        frame_age_ms: 9,
        rtt_ms: 7,
        device: "iPad14,1".into(),
        frame_age_max_ms: Some(24),
        rtt_max_ms: Some(80),
        audio_behind_ms: Some(42),
        audio_late: Some(3),
    };
    let j = val(&heard);
    assert_eq!(j["audioBehindMs"], 42);
    assert_eq!(j["audioLate"], 3);
    assert_eq!(j["frameAgeMaxMs"], 24);
    round(&heard);
    assert_eq!(ClientStats::late_since(5, Some(2)), 7);
    assert_eq!(ClientStats::late_since(i64::MAX, Some(1)), i64::MAX);
    assert_eq!(ClientStats::late_since(3, Some(-9)), 3);
    assert_eq!(ClientStats::late_since(3, None), 3);
}

// ---- Input -------------------------------------------------------------------------------------

#[test]
fn input_event_every_case() {
    // sillclient.py:710-714 and docs/windows-client/protocol-spec.md section 6.
    let cases: Vec<(InputEvent, Value)> = vec![
        (
            InputEvent::Pointer {
                action: PointerAction::Move,
                x: 0.5,
                y: 0.25,
            },
            json!({"pointer": {"_0": "move", "x": 0.5, "y": 0.25}}),
        ),
        (
            InputEvent::Pointer {
                action: PointerAction::LeftDown,
                x: 0.0,
                y: 1.0,
            },
            json!({"pointer": {"_0": "leftDown", "x": 0.0, "y": 1.0}}),
        ),
        (
            InputEvent::Pointer {
                action: PointerAction::LeftUp,
                x: 0.0,
                y: 1.0,
            },
            json!({"pointer": {"_0": "leftUp", "x": 0.0, "y": 1.0}}),
        ),
        (
            InputEvent::Pointer {
                action: PointerAction::RightDown,
                x: 0.1,
                y: 0.2,
            },
            json!({"pointer": {"_0": "rightDown", "x": 0.1, "y": 0.2}}),
        ),
        (
            InputEvent::Pointer {
                action: PointerAction::RightUp,
                x: 0.1,
                y: 0.2,
            },
            json!({"pointer": {"_0": "rightUp", "x": 0.1, "y": 0.2}}),
        ),
        (
            InputEvent::Scroll {
                x: 0.5,
                y: 0.5,
                dx: 0.0,
                dy: -0.125,
            },
            json!({"scroll": {"x": 0.5, "y": 0.5, "dx": 0.0, "dy": -0.125}}),
        ),
        (
            InputEvent::Text {
                text: "héllo\n".into(),
            },
            json!({"text": {"_0": "héllo\n"}}),
        ),
        (
            InputEvent::ScrollGesture {
                phase: ScrollPhase::Began,
                x: 0.5,
                y: 0.5,
            },
            json!({"scrollGesture": {"_0": "began", "x": 0.5, "y": 0.5}}),
        ),
        (
            InputEvent::ScrollGesture {
                phase: ScrollPhase::Ended,
                x: 0.5,
                y: 0.5,
            },
            json!({"scrollGesture": {"_0": "ended", "x": 0.5, "y": 0.5}}),
        ),
        (
            InputEvent::ScrollGesture {
                phase: ScrollPhase::MomentumBegan,
                x: 0.5,
                y: 0.5,
            },
            json!({"scrollGesture": {"_0": "momentumBegan", "x": 0.5, "y": 0.5}}),
        ),
        (
            InputEvent::ScrollGesture {
                phase: ScrollPhase::MomentumEnded,
                x: 0.5,
                y: 0.5,
            },
            json!({"scrollGesture": {"_0": "momentumEnded", "x": 0.5, "y": 0.5}}),
        ),
        (
            InputEvent::Key {
                hid_usage: 44,
                down: true,
                modifiers: 0,
            },
            json!({"key": {"hidUsage": 44, "down": true, "modifiers": 0}}),
        ),
        (
            InputEvent::Key {
                hid_usage: 0xE3,
                down: false,
                modifiers: 0x100000,
            },
            json!({"key": {"hidUsage": 227, "down": false, "modifiers": 1048576}}),
        ),
        (
            InputEvent::Key {
                hid_usage: 65535,
                down: true,
                modifiers: u64::MAX,
            },
            json!({"key": {"hidUsage": 65535, "down": true, "modifiers": u64::MAX}}),
        ),
    ];
    for (e, j) in &cases {
        assert_eq!(&val(e), j);
        let back: InputEvent = serde_json::from_value(j.clone()).unwrap();
        assert_eq!(&back, e);
        round(e);
    }
    // sillclient's key order (down first) decodes.
    assert_eq!(
        dec::<InputEvent>(r#"{"key":{"down":true,"hidUsage":44,"modifiers":0}}"#),
        Some(InputEvent::Key {
            hid_usage: 44,
            down: true,
            modifiers: 0
        })
    );
    assert_eq!(
        dec::<InputEvent>(r#"{"key":{"hidUsage":70000,"down":true,"modifiers":0}}"#),
        None,
        "UInt16 overflow"
    );
    assert_eq!(
        dec::<InputEvent>(r#"{"pointer":{"_0":"drag","x":0,"y":0}}"#),
        None
    );
    assert_eq!(
        dec::<InputEvent>(r#"{"pointer":{"_0":"move","x":0}}"#),
        None,
        "missing y"
    );
    assert_eq!(dec::<InputEvent>(r#"{"pen":{}}"#), None);
}

// ---- Settings ----------------------------------------------------------------------------------

fn base_settings() -> StreamSettings {
    StreamSettings {
        max_fps: 60,
        bitrate: 8_000_000,
        capture_scale: 1.0,
        prioritize_speed: true,
        virtual_display: false,
        direct_wireless: Some(true),
        send_audio: Some(true),
    }
}

#[test]
fn stream_settings_and_old_hosts() {
    // Tests/checks/compatibility:221-229.
    let s = base_settings();
    let j = val(&s);
    assert_eq!(j["sendAudio"], true);
    assert_eq!(j["maxFPS"], 60);
    round(&s);
    let old: StreamSettings = dec(r#"{"maxFPS":60,"bitrate":8000000,"captureScale":1,"prioritizeSpeed":true,"virtualDisplay":false,"directWireless":true}"#).unwrap();
    assert_eq!(old.send_audio, None);
    assert_eq!(old.capture_scale, 1.0);
    let nothing = StreamSettings {
        direct_wireless: None,
        send_audio: None,
        ..s
    };
    assert_eq!(
        val(&nothing),
        json!({"maxFPS": 60, "bitrate": 8000000, "captureScale": 1.0, "prioritizeSpeed": true, "virtualDisplay": false})
    );
}

#[test]
fn host_settings_state_full_and_minimal() {
    let text = r#"{"settings":{"maxFPS":60,"bitrate":8000000,"captureScale":1,"prioritizeSpeed":true,"virtualDisplay":false,"directWireless":true},"persistent":true,"virtualDisplayAvailable":true,"softwareEncoder":false}"#;
    let st: HostSettingsState = dec(text).unwrap();
    assert!(st.persistent && st.virtual_display_available && !st.software_encoder);
    assert!(
        st.stream.is_none() && st.answering.is_none() && st.away.is_none() && st.link.is_none()
    );
    assert!(st.audio_note.is_none() && st.session_paired.is_none());
    round(&st);
    let full = HostSettingsState {
        settings: st.settings.clone(),
        persistent: false,
        virtual_display_available: false,
        virtual_display_note: Some("n".into()),
        software_encoder: true,
        stream: Some(RunningStream {
            width: 3024,
            height: 1964,
            fps: 60,
            mbps: 40,
            on_virtual_display: false,
        }),
        answering: Some(4),
        away: Some(AwayQuality {
            home_bitrate: 40_000_000,
            home_capture_scale: 2.0,
            away_bitrate: 4_000_000,
            away_capture_scale: 1.0,
            this_connection_away: true,
            away_running: true,
        }),
        link: Some(LinkReport {
            state: "behind".into(),
            withheld_per_second: 52,
            bitrate: 40_000_000,
            carried_kbps: Some(6400),
            suggested_bitrate: Some(4_000_000),
            suggested_capture_scale: Some(1.0),
        }),
        audio_note: Some(
            "Couldn’t capture the sound of Safari: it did not start within 2 s.".into(),
        ),
        session_paired: Some(false),
    };
    let j = val(&full);
    assert_eq!(j["answering"], 4);
    assert_eq!(j["sessionPaired"], false);
    assert_eq!(j["stream"]["onVirtualDisplay"], false);
    assert_eq!(j["away"]["thisConnectionAway"], true);
    assert_eq!(j["link"]["withheldPerSecond"], 52);
    assert_eq!(j["link"]["carriedKbps"], 6400);
    assert_eq!(j["virtualDisplayNote"], "n");
    round(&full);
    assert!(full.link.as_ref().unwrap().is_behind());
    // An unknown link state decodes (a device reads it as keeping up).
    let l: LinkReport = dec(r#"{"state":"later","withheldPerSecond":0,"bitrate":1}"#).unwrap();
    assert!(!l.is_behind());
    assert!(dec::<HostSettingsState>(r#"{"persistent":true}"#).is_none());
}

#[test]
fn host_settings_change_rules() {
    // Tests/checks/compatibility:237.
    let c = HostSettingsChange {
        token: Some(4),
        send_audio: Some(true),
        ..Default::default()
    };
    assert_eq!(val(&c), json!({"sendAudio": true, "token": 4}));
    assert!(!c.is_empty());
    assert!(
        HostSettingsChange {
            token: Some(4),
            ..Default::default()
        }
        .is_empty(),
        "the token alone changes nothing"
    );
    assert!(!HostSettingsChange {
        max_fps: Some(120),
        ..Default::default()
    }
    .is_empty());
    assert_eq!(val(&HostSettingsChange::default()), json!({}));
    let st = StreamSettings {
        max_fps: 60,
        bitrate: 8_000_000,
        capture_scale: 1.0,
        prioritize_speed: false,
        virtual_display: false,
        direct_wireless: None,
        send_audio: None,
    };
    let after = HostSettingsChange {
        token: Some(1),
        max_fps: Some(120),
        bitrate: Some(40_000_000),
        capture_scale: Some(2.0),
        prioritize_speed: Some(true),
        virtual_display: Some(true),
        direct_wireless: Some(true),
        send_audio: Some(false),
    }
    .applied(&st);
    assert_eq!(
        (
            after.max_fps,
            after.bitrate,
            after.capture_scale,
            after.prioritize_speed,
            after.virtual_display,
            after.direct_wireless,
            after.send_audio
        ),
        (120, 40_000_000, 2.0, true, true, Some(true), Some(false))
    );
    assert_eq!(
        HostSettingsChange {
            token: Some(1),
            ..Default::default()
        }
        .applied(&st),
        st
    );
    round(&c);
}

#[test]
fn quality_presets_and_choices() {
    // HostSettings.swift:297-362.
    assert_eq!(
        SettingsChoices::bitrate(),
        vec![
            4_000_000,
            8_000_000,
            15_000_000,
            25_000_000,
            40_000_000,
            80_000_000,
            150_000_000
        ]
    );
    assert_eq!(SettingsChoices::MAX_FPS, [60, 120]);
    assert_eq!(SettingsChoices::CAPTURE_SCALE, [2.0, 1.0]);
    assert_eq!(QualityPreset::Pro.title(), "Pro — 40 Mbps");
    assert_eq!(
        QualityPreset::title_for_bitrate(40_000_000),
        "Pro — 40 Mbps"
    );
    assert_eq!(
        QualityPreset::title_for_bitrate(200_000_000),
        "Custom — 200 Mbps"
    );
    assert_eq!(
        QualityPreset::title_for_bitrate(12_500_000),
        "Custom — 12.5 Mbps"
    );
    assert_eq!(QualityPreset::name_for_bitrate(4_000_000), "Low");
    assert_eq!(QualityPreset::name_for_bitrate(9_000_000), "9 Mbps");
    assert_eq!(QualityPreset::short_title(4_000_000, 1.0), "Low · Standard");
    assert_eq!(QualityPreset::short_title(40_000_000, 2.0), "Pro · Retina");
    assert_eq!(QualityPreset::short_title(40_000_000, 1.5), "Pro · Retina");
    assert_eq!(
        QualityPreset::from_bitrate(150_000_000),
        Some(QualityPreset::Extreme)
    );
    assert_eq!(QualityPreset::from_bitrate(1), None);
}

// ---- Pairing and remote ------------------------------------------------------------------------

#[test]
fn pair_request_shape() {
    // Tests/checks/protocol "kind 19 decodes the plan's example".
    let r: PairRequest = dec(r#"{"v":1,"method":"qr","proof":"AyZAJqNTU5nSKEaHx6RZOf0K5dOeygR_8JSrCV_gNOo","name":"iPad","model":"iPad14,1"}"#).unwrap();
    assert_eq!(r.model.as_deref(), Some("iPad14,1"));
    assert_eq!(r.cable, None);
    assert_eq!(
        val(&r),
        json!({"v": 1, "method": "qr", "proof": "AyZAJqNTU5nSKEaHx6RZOf0K5dOeygR_8JSrCV_gNOo", "name": "iPad", "model": "iPad14,1"})
    );
    let ask = PairRequest::new(PairRequest::ASK, "", "Windows PC", None);
    assert_eq!(
        val(&ask),
        json!({"v": 1, "method": "ask", "proof": "", "name": "Windows PC"})
    );
    let cable = PairRequest {
        cable: Some(true),
        ..ask.clone()
    };
    assert_eq!(val(&cable)["cable"], true);
    round(&cable);
    assert!(dec::<PairRequest>(r#"{"v":1,"method":"qr"}"#).is_none());
}

#[test]
fn pair_result_shapes() {
    // Tests/checks/protocol "kind 20 JSON leaves out nil fields".
    let r = PairResult {
        ok: false,
        reason: Some("code".into()),
        tries_left: Some(4),
        ..Default::default()
    };
    assert_eq!(
        val(&r),
        json!({"ok": false, "reason": "code", "triesLeft": 4})
    );
    let ok: PairResult = dec(r#"{"ok":true,"proof":"abc","macID":"ABCDEFGHJKMNPQRS","name":"Mac","recognitionKey":"k","method":"cable","later":1}"#).unwrap();
    assert!(ok.ok);
    assert_eq!(ok.mac_id.as_deref(), Some("ABCDEFGHJKMNPQRS"));
    assert_eq!(ok.recognition_key.as_deref(), Some("k"));
    assert_eq!(ok.method.as_deref(), Some("cable"));
    let busy: PairResult = dec(r#"{"ok":false,"reason":"busy","retryAfter":2.5}"#).unwrap();
    assert_eq!(busy.retry_after, Some(2.5));
    assert!(!busy.has_unknown_reason());
    assert!(!ok.has_unknown_reason());
    let strange: PairResult =
        dec(r#"{"ok":false,"reason":"tryLater","message":"Not now."}"#).unwrap();
    assert!(strange.has_unknown_reason());
    assert!(PairResult {
        ok: false,
        ..Default::default()
    }
    .has_unknown_reason());
    for k in PairResult::KNOWN_REASONS {
        assert!(!PairResult {
            ok: false,
            reason: Some(k.into()),
            ..Default::default()
        }
        .has_unknown_reason());
    }
    round(&r);
}

#[test]
fn mac_info_and_signed_envelope() {
    // Tests/checks/protocol "kind 18 signing": the example addresses; port only where set.
    let info = MacInfo {
        v: 1,
        mac_id: "ABCDEFGHJKMNPQRS".into(),
        name: "Mac mini".into(),
        issued_at: 1790265600.5,
        remote_access: true,
        remote_port: 7455,
        internet: false,
        addresses: vec![
            MacAddress {
                host: "100.101.102.103".into(),
                port: None,
                kind: "vpn".into(),
                via: "Tailscale".into(),
            },
            MacAddress {
                host: "home.example.net".into(),
                port: Some(17455),
                kind: "internet".into(),
                via: "Address name".into(),
            },
        ],
    };
    let j = val(&info);
    assert!(j["addresses"][0].get("port").is_none());
    assert_eq!(j["addresses"][1]["port"], 17455);
    assert_eq!(j["remotePort"], 7455);
    assert_eq!(j["issuedAt"], 1790265600.5);
    round(&info);
    let bytes = wire::encode(&info);
    let signed = SignedMacInfo {
        info: wire::b64_encode(&bytes),
        key: wire::b64_encode(&[4u8; 65]),
        sig: wire::b64_encode(&[0x30, 0x06, 1, 2, 3, 4, 5, 6]),
    };
    assert_eq!(signed.info_bytes().unwrap(), bytes);
    assert_eq!(signed.unverified_info().unwrap(), info);
    assert_eq!(signed.key_point().unwrap().len(), 65);
    assert_eq!(signed.sig_der().unwrap()[0], 0x30);
    round(&signed);
    // key must be 65 bytes; a non-base64 info is None.
    assert!(SignedMacInfo {
        key: wire::b64_encode(&[4u8; 64]),
        ..signed.clone()
    }
    .key_point()
    .is_none());
    assert!(SignedMacInfo {
        info: "not base64!".into(),
        ..signed.clone()
    }
    .unverified_info()
    .is_none());
    let keys: std::collections::BTreeSet<String> =
        val(&signed).as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        keys,
        ["info", "key", "sig"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );
}

// ---- Menus, pointer, gesture, clipboard --------------------------------------------------------

#[test]
fn menu_messages() {
    // docs/windows-client/protocol-spec.md 7.4; MacMenu.swift:33-124.
    let sub = FetchMenu {
        token: Some(1),
        ..Default::default()
    };
    assert_eq!(val(&sub), json!({"token": 1}));
    let fetch = FetchMenu {
        version: Some(3),
        id: Some("2".into()),
        title: Some("File".into()),
        token: Some(9),
    };
    assert_eq!(
        val(&fetch),
        json!({"version": 3, "id": "2", "title": "File", "token": 9})
    );
    let press = PressMenuItem {
        version: Some(3),
        id: Some("2.9".into()),
        title: Some("Save".into()),
        token: Some(10),
    };
    assert_eq!(
        val(&press),
        json!({"version": 3, "id": "2.9", "title": "Save", "token": 10})
    );
    let top: MacMenu = dec(r#"{"version":2,"app":"TextEdit","bundleID":"com.apple.TextEdit","menus":[{"id":"1","title":"File","submenu":true},{"id":"2","separator":true}],"future":1}"#).unwrap();
    assert_eq!(top.bundle_id.as_deref(), Some("com.apple.TextEdit"));
    assert_eq!(top.menus.as_ref().unwrap().len(), 2);
    assert_eq!(top.menus.as_ref().unwrap()[0].submenu, Some(true));
    assert_eq!(top.menus.as_ref().unwrap()[1].separator, Some(true));
    let answer: MacMenu = dec(r#"{"answering":9,"menu":"File","items":[{"id":"2.0","title":"Save","enabled":false,"mark":"✓","key":"⌘S"}],"more":300,"stale":true,"note":"n"}"#).unwrap();
    assert_eq!(answer.answering, Some(9));
    assert_eq!(answer.items.as_ref().unwrap()[0].key.as_deref(), Some("⌘S"));
    assert_eq!(
        (answer.more, answer.stale, answer.note.as_deref()),
        (Some(300), Some(true), Some("n"))
    );
    let pressed: MacMenu = dec(r#"{"answering":10,"pressed":true}"#).unwrap();
    assert_eq!(pressed.pressed, Some(true));
    assert_eq!(
        dec::<MacMenu>("{}"),
        Some(MacMenu::default()),
        "all fields optional"
    );
    assert_eq!(val(&MacMenu::default()), json!({}));
    round(&top);
    round(&answer);
}

#[test]
fn pointer_gesture_clipboard() {
    // docs/windows-client/protocol-spec.md 7.5.
    let p: MacPointer = dec(r#"{"x":0.4213,"y":0.1873,"inside":true,"seen":12}"#).unwrap();
    assert_eq!(p.position(), Some((0.4213, 0.1873)));
    assert_eq!(p.seen, Some(12));
    assert_eq!(
        dec::<MacPointer>(r#"{"inside":false,"seen":12}"#)
            .unwrap()
            .position(),
        None
    );
    assert_eq!(
        dec::<MacPointer>(r#"{"inside":true,"x":0.5}"#)
            .unwrap()
            .position(),
        None
    );
    assert_eq!(MacPointer::rounded(0.123456), 0.1235);
    assert_eq!(
        val(&MacPointer {
            inside: Some(false),
            seen: Some(3),
            ..Default::default()
        }),
        json!({"inside": false, "seen": 3})
    );
    // Tests/checks/compatibility:153-160.
    assert_eq!(
        val(&TrackpadGesture {
            gesture: "swipeUp".into(),
            fingers: Some(3)
        }),
        json!({"fingers": 3, "gesture": "swipeUp"})
    );
    assert_eq!(
        val(&TrackpadGesture {
            gesture: TrackpadGesture::PINCH.into(),
            fingers: None
        }),
        json!({"gesture": "pinch"})
    );
    assert_eq!(
        dec::<TrackpadGesture>(r#"{"gesture":"spread","fingers":4,"phase":"began"}"#),
        Some(TrackpadGesture {
            gesture: "spread".into(),
            fingers: Some(4)
        })
    );
    assert_eq!(
        dec::<TrackpadGesture>(r#"{"gesture":"rotate"}"#)
            .unwrap()
            .gesture,
        "rotate"
    );
    assert_eq!(dec::<TrackpadGesture>(r#"{"fingers":3}"#), None);
    assert_eq!(dec::<TrackpadGesture>(r#"{"gesture":3}"#), None);
    assert_eq!(
        dec::<TrackpadGesture>(r#"{"gesture":"swipeUp","fingers":"3"}"#),
        None
    );
    assert_eq!(TrackpadGesture::NAMES.len(), 6);
    // Tests/checks/compatibility:291-292.
    assert_eq!(
        val(&Clipboard {
            text: Some("Hello".into()),
            ..Default::default()
        }),
        json!({"text": "Hello"})
    );
    assert_eq!(val(&Clipboard::default()), json!({}));
    assert_eq!(
        val(&Clipboard {
            stop: Some(true),
            ..Default::default()
        }),
        json!({"stop": true})
    );
    assert_eq!(
        dec::<Clipboard>(r#"{"text":"a","image":"AQID","later":1}"#),
        Some(Clipboard {
            text: Some("a".into()),
            image: Some(vec![1, 2, 3]),
            stop: None
        })
    );
    assert_eq!(
        val(&Clipboard {
            image: Some(vec![1, 2, 3]),
            ..Default::default()
        }),
        json!({"image": "AQID"})
    );
    assert_eq!(Clipboard::MAX_TEXT_BYTES, 262_144);
}

// ---- Audio -------------------------------------------------------------------------------------

#[test]
fn audio_format_json() {
    // Tests/checks/compatibility:210-216.
    let f = AudioFormat {
        codec: Some("aac-eld".into()),
        sample_rate: Some(48000),
        channels: Some(2),
        frames_per_packet: Some(480),
        priming_frames: Some(240),
        bitrate: Some(128000),
        epoch: Some(3),
        cookie: Some(vec![0xF8, 0xF0, 0x21, 0x2C]),
        source: Some("app".into()),
        app: Some("Safari".into()),
    };
    assert_eq!(
        val(&f),
        json!({"app": "Safari", "bitrate": 128000, "channels": 2, "codec": "aac-eld", "cookie": "+PAhLA==",
               "epoch": 3, "framesPerPacket": 480, "primingFrames": 240, "sampleRate": 48000, "source": "app"})
    );
    round(&f);
    assert_eq!(dec::<AudioFormat>("{}"), Some(AudioFormat::default()));
    assert_eq!(val(&AudioFormat::default()), json!({}));
    assert_eq!(
        dec::<AudioFormat>(r#"{"codec":"opus","v":2,"channels":6}"#),
        Some(AudioFormat {
            codec: Some("opus".into()),
            channels: Some(6),
            ..Default::default()
        })
    );
    assert_eq!(
        dec::<AudioFormat>(r#"{"cookie":"not base64!"}"#),
        None,
        "a cookie that is not base64 does not decode"
    );
    assert_eq!(
        dec::<AudioFormat>(r#"{"cookie":"+PAhLA"}"#).unwrap().cookie,
        Some(vec![0xF8, 0xF0, 0x21, 0x2C]),
        "padding-tolerant"
    );
}

fn eld() -> Vec<u8> {
    (0..162u32)
        .map(|i| (i.wrapping_mul(7).wrapping_add(1)) as u8)
        .collect()
}
fn packets(epoch: u16, seq: u32, start: bool, p: Vec<Vec<u8>>) -> Vec<u8> {
    AudioMessage::Packets(AudioPackets {
        epoch,
        seq,
        segment_start: start,
        packets: p,
    })
    .serialize()
}

#[test]
fn audio_packets_vectors() {
    // Tests/checks/protocol "kind 43 type 2".
    let ex = packets(3, 0, true, vec![eld()]);
    assert_eq!(
        &ex[..11],
        &[0x02, 0x00, 0x03, 0, 0, 0, 0, 0x01, 0x01, 0x00, 0xA2]
    );
    assert_eq!(ex.len(), 173);
    assert_eq!(&ex[11..], eld().as_slice());
    assert_eq!(
        AudioMessage::parse(&ex),
        Some(AudioMessage::Packets(AudioPackets {
            epoch: 3,
            seq: 0,
            segment_start: true,
            packets: vec![eld()]
        }))
    );
    let many: Vec<Vec<u8>> = (0..255u32)
        .map(|i| vec![i as u8; (i % 40) as usize])
        .collect();
    let bytes = packets(65535, 4_000_000_000, false, many.clone());
    assert_eq!(
        AudioMessage::parse(&bytes),
        Some(AudioMessage::Packets(AudioPackets {
            epoch: 65535,
            seq: 4_000_000_000,
            segment_start: false,
            packets: many
        }))
    );
    assert_eq!(
        &packets(0x0102, 0x03040506, false, vec![vec![9]])[..9],
        &[2, 1, 2, 3, 4, 5, 6, 0, 1]
    );
    // Flags: bit 0 only.
    let mut f = ex.clone();
    f[7] = 0x03;
    assert!(matches!(AudioMessage::parse(&f), Some(AudioMessage::Packets(p)) if p.segment_start));
    f[7] = 0xFE;
    assert!(matches!(AudioMessage::parse(&f), Some(AudioMessage::Packets(p)) if !p.segment_start));
    // At most 255 packets are written.
    match AudioMessage::parse(&packets(1, 1, false, vec![vec![1]; 300])) {
        Some(AudioMessage::Packets(p)) => assert_eq!(p.packets.len(), 255),
        o => panic!("{o:?}"),
    }
}

#[test]
fn audio_refusals_and_caps() {
    // Tests/checks/protocol "kind 43 refused".
    let ex = packets(3, 0, true, vec![eld()]);
    let refused = |d: &[u8]| assert_eq!(AudioMessage::parse(d), None);
    refused(&[]);
    refused(&[3, 0, 3, 0, 0, 0, 0, 1, 1]);
    refused(&[0]);
    refused(&[1]);
    refused(b"\x01aac");
    refused(b"\x01[]");
    let mut over = vec![1u8];
    over.extend(br#"{"app":""#);
    over.extend(vec![b'a'; 4100]);
    over.extend(br#""}"#);
    refused(&over);
    let mut at = vec![1u8];
    at.extend(br#"{"app":""#);
    at.extend(vec![b'a'; 4085]);
    at.extend(br#""}"#);
    assert_eq!(at.len(), 4096);
    assert!(AudioMessage::parse(&at).is_some(), "4,096 bytes are taken");
    refused(&[2]);
    refused(&ex[..8]);
    refused(&[2, 0, 3, 0, 0, 0, 0, 1, 0]);
    refused(&[2, 0, 3, 0, 0, 0, 0, 1, 0, 0, 1, 7]);
    refused(&ex[..ex.len() - 1]);
    let mut extra = ex.clone();
    extra.push(0);
    refused(&extra);
    let mut count2 = ex.clone();
    count2[8] = 2;
    refused(&count2);
    let big = vec![1u8; 16384 - 9 - 2];
    assert!(
        AudioMessage::parse(&packets(1, 1, false, vec![big.clone()])).is_some(),
        "16,384 bytes are taken"
    );
    let mut bigger = big;
    bigger.push(1);
    refused(&packets(1, 1, false, vec![bigger]));
    // Type 1 round trips and {} is a format.
    let fmt = AudioFormat {
        codec: Some("aac-eld".into()),
        epoch: Some(3),
        cookie: Some(vec![1, 2]),
        ..Default::default()
    };
    let b = AudioMessage::Format(fmt.clone()).serialize();
    assert_eq!(b[0], 1);
    assert_eq!(AudioMessage::parse(&b), Some(AudioMessage::Format(fmt)));
    assert_eq!(
        AudioMessage::parse(b"\x01{}"),
        Some(AudioMessage::Format(AudioFormat::default()))
    );
}

#[test]
fn audio_random_and_damaged_payloads_never_panic() {
    // Tests/checks/protocol: 2,000 random and damaged payloads, here from a deterministic LCG.
    let mut s: u64 = 0x1234_5678_9abc_def0;
    let mut next = move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as u32
    };
    let (mut parsed, mut bad_round) = (0, 0);
    for i in 0..2000 {
        let mut d: Vec<u8>;
        if i < 1000 {
            d = (0..next() % 41).map(|_| next() as u8).collect();
            if i % 3 == 0 && !d.is_empty() {
                d[0] = 2
            }
            if i % 3 == 1 && !d.is_empty() {
                d[0] = 1
            }
        } else {
            let p: Vec<Vec<u8>> = (0..1 + next() % 4)
                .map(|_| (0..next() % 21).map(|_| next() as u8).collect())
                .collect();
            d = packets(next() as u16, next(), next() % 2 == 0, p);
            match next() % 4 {
                0 => {
                    let n = (next() as usize) % (d.len() + 1);
                    d.resize(n, 0);
                    d.shrink_to_fit();
                }
                1 => {
                    let k = (next() as usize) % d.len();
                    d[k] ^= 1 + (next() % 255) as u8
                }
                2 => d.push(next() as u8),
                _ => {}
            }
        }
        if let Some(AudioMessage::Packets(p)) = AudioMessage::parse(&d) {
            parsed += 1;
            let mut again = p.serialize();
            again[7] = d[7];
            if again != d {
                bad_round += 1
            }
        }
    }
    assert!(parsed > 0);
    assert_eq!(bad_round, 0);
}

#[test]
fn audio_codec_choice() {
    let offered = vec!["opus".to_string(), "aac-eld".to_string()];
    assert_eq!(choose_codec(Some(&offered), &["aac-eld"]), Some("aac-eld"));
    assert_eq!(choose_codec(Some(&offered), &[]), None);
    assert_eq!(choose_codec(None, &["aac-eld"]), None);
}

// ---- Blobs -------------------------------------------------------------------------------------

#[test]
fn image_blobs() {
    // Switcher.swift:190-218. Hand-computed: window 7, JPEG ff d8.
    let t = encode_thumbnail(7, &[0xff, 0xd8]);
    assert_eq!(t, [0, 0, 0, 7, 0xff, 0xd8]);
    assert_eq!(decode_thumbnail(&t), Some((7, &[0xff, 0xd8][..])));
    assert!(
        decode_thumbnail(&[0, 0, 0, 7]).is_none(),
        "needs more than 4 bytes"
    );
    let i = encode_icon("a.b", &[0x89, b'P']).unwrap();
    assert_eq!(i, [0, 3, b'a', b'.', b'b', 0x89, b'P']);
    assert_eq!(
        decode_icon(&i),
        Some(("a.b".to_string(), &[0x89, b'P'][..]))
    );
    assert!(
        decode_icon(&[0, 3, b'a', b'.', b'b']).is_none(),
        "empty PNG refused"
    );
    assert!(decode_icon(&[0, 9, 1, 2]).is_none());
    assert!(
        decode_icon(&[0, 2, 0xff, 0xfe, 5]).is_none(),
        "id not UTF-8"
    );
    assert!(decode_icon(&[0]).is_none());
}

#[test]
fn cursor_shape_blob() {
    // Viewport.swift:141-158: four Float32 BE then PNG; 1.0 = 3F800000, 2.0 = 40000000.
    let b = encode_cursor_shape((1.0, 2.0), (16.0, 24.0), &[0x89, b'P']);
    assert_eq!(&b[..8], &[0x3f, 0x80, 0, 0, 0x40, 0, 0, 0]);
    assert_eq!(&b[8..16], &[0x41, 0x80, 0, 0, 0x41, 0xc0, 0, 0]);
    let c = decode_cursor_shape(&b).unwrap();
    assert_eq!(
        (c.hotspot, c.size, c.png),
        ((1.0, 2.0), (16.0, 24.0), &[0x89, b'P'][..])
    );
    assert!(
        decode_cursor_shape(&b[..16]).is_none(),
        "needs more than 16 bytes"
    );
}
