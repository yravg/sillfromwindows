//! Cross-checks against Scripts/sillclient.py, the Python reference client that works against the
//! real Mac host. `tests/oracle/sillclient_oracle.py` extracts the script's message builders with
//! `ast` (the script's top level connects to hosts and is never run) and prints what they build;
//! `tests/oracle/parse_stdin.py` parses Rust-built messages the way the script's read loop does.
//! Neither opens a socket. Needs `python3`; the tests fail (not skip) when it is missing, because a
//! silent skip would hide a broken oracle.

use serde_json::{json, Value};
use sill_proto::input::*;
use sill_proto::settings::HostSettingsChange;
use sill_proto::small::TrackpadGesture;
use sill_proto::switcher::StreamSource;
use sill_proto::viewport::{ClientStats, Viewport};
use sill_proto::{wire, Header, Message, MessageKind, MessageReader};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn python(args: &[&str], stdin: Option<&str>) -> String {
    let mut child = Command::new("python3")
        .arg("-I")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python3 is needed for the sillclient cross-checks");
    if let Some(s) = stdin {
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(s.as_bytes())
            .unwrap();
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "python failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn oracle() -> Value {
    let script = root().join("Scripts/sillclient.py");
    let oracle =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/sillclient_oracle.py");
    serde_json::from_str(&python(
        &[oracle.to_str().unwrap(), script.to_str().unwrap()],
        None,
    ))
    .unwrap()
}

fn hex_to_bytes(h: &str) -> Vec<u8> {
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
        .collect()
}
fn bytes_to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// All numbers as f64, so 1 and 1.0 compare equal (Python writes 0.5 and 1117, Rust 1117.0).
fn norm(v: &Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), norm(v))).collect()),
        other => other.clone(),
    }
}

fn built(o: &Value, name: &str) -> Vec<u8> {
    hex_to_bytes(
        o["built"][name]
            .as_str()
            .unwrap_or_else(|| panic!("oracle has no {name}")),
    )
}

/// Python's message: parse it with the Rust reader, return (kind, ts, key, payload).
fn read_one(bytes: &[u8]) -> Message {
    let mut r = MessageReader::new(sill_proto::Caps::HOST);
    let mut got = r.feed(bytes).unwrap();
    assert_eq!(got.len(), 1);
    assert!(!r.mid_message());
    got.remove(0)
}

const TS: f64 = 1700000000.5; // the oracle's fixed clock

#[test]
fn python_headers_parse_and_rust_headers_match_byte_for_byte() {
    let o = oracle();
    // For every message the script builds, rebuilding it from what Rust parsed gives the same bytes.
    let names: Vec<String> = o["built"].as_object().unwrap().keys().cloned().collect();
    assert!(names.len() >= 15);
    for n in names {
        let bytes = built(&o, &n);
        let m = read_one(&bytes);
        assert_eq!(m.timestamp, TS, "{n}");
        assert_eq!(
            m.serialize().unwrap(),
            bytes,
            "{n}: Rust re-serializes Python's bytes exactly"
        );
        // And the Python parse of the same bytes agrees with the Rust header.
        let p = &o["parsed"][&n];
        assert_eq!(p["kind"].as_u64().unwrap() as u8, m.kind.as_u8(), "{n}");
        assert_eq!(p["len"].as_u64().unwrap() as usize, m.payload.len(), "{n}");
        assert_eq!(p["key"].as_u64().unwrap() == 1, m.is_keyframe, "{n}");
    }
}

#[test]
fn python_built_kinds_and_unknown_kinds() {
    let o = oracle();
    assert_eq!(read_one(&built(&o, "ping8")).kind, MessageKind::Ping);
    assert_eq!(
        read_one(&built(&o, "ping8")).payload,
        12.25f64.to_be_bytes(),
        "iOS sends a Double BE"
    );
    assert_eq!(
        read_one(&built(&o, "empty_unknown_201")).kind,
        MessageKind::Unknown(201)
    );
    let u = read_one(&built(&o, "unknown_200_hello"));
    assert_eq!(
        (u.kind, u.payload.as_slice()),
        (MessageKind::Unknown(200), &b"hello"[..])
    );
    let k = read_one(&built(&o, "keyframe_flag"));
    assert!(k.is_keyframe);
    assert_eq!(k.kind, MessageKind::Frame);
    assert_eq!(k.payload, [0, 1, 2]);
    // sillclient.py:708 sends 200 then 201 then a pick back to back: the reader skips by length.
    let mut stream = built(&o, "unknown_200_hello");
    stream.extend(built(&o, "empty_unknown_201"));
    stream.extend(built(&o, "pick_desktop"));
    let got = MessageReader::new(sill_proto::Caps::HOST)
        .feed(&stream)
        .unwrap();
    assert_eq!(
        got.iter().map(|m| m.kind).collect::<Vec<_>>(),
        vec![
            MessageKind::Unknown(200),
            MessageKind::Unknown(201),
            MessageKind::SelectSource
        ]
    );
}

#[test]
fn python_payloads_decode_to_the_typed_values_and_rust_json_is_the_same_value() {
    let o = oracle();
    // (name, expected kind, Rust-built JSON of the typed value)
    let cases: Vec<(&str, MessageKind, Value)> = vec![
        (
            "pick_desktop",
            MessageKind::SelectSource,
            serde_json::to_value(StreamSource::Desktop).unwrap(),
        ),
        (
            "pick_none",
            MessageKind::SelectSource,
            serde_json::to_value(StreamSource::None).unwrap(),
        ),
        (
            "pick_window",
            MessageKind::SelectSource,
            serde_json::to_value(StreamSource::Window(123)).unwrap(),
        ),
        (
            "pointer_move",
            MessageKind::Input,
            serde_json::to_value(InputEvent::Pointer {
                action: PointerAction::Move,
                x: 0.5,
                y: 0.25,
            })
            .unwrap(),
        ),
        (
            "pointer_left_down",
            MessageKind::Input,
            serde_json::to_value(InputEvent::Pointer {
                action: PointerAction::LeftDown,
                x: 0.5,
                y: 0.25,
            })
            .unwrap(),
        ),
        (
            "key_down",
            MessageKind::Input,
            serde_json::to_value(InputEvent::Key {
                hid_usage: 44,
                down: true,
                modifiers: 0,
            })
            .unwrap(),
        ),
        (
            "key_up",
            MessageKind::Input,
            serde_json::to_value(InputEvent::Key {
                hid_usage: 44,
                down: false,
                modifiers: 0,
            })
            .unwrap(),
        ),
        (
            "viewport_60",
            MessageKind::Viewport,
            serde_json::to_value(Viewport {
                width: 1117.0,
                height: 642.0,
                scale: None,
                local_cursor: None,
                fps: Some(60),
            })
            .unwrap(),
        ),
        (
            "gesture_swipe_up",
            MessageKind::Gesture,
            serde_json::to_value(TrackpadGesture {
                gesture: "swipeUp".into(),
                fingers: Some(3),
            })
            .unwrap(),
        ),
        (
            "gesture_pinch_4",
            MessageKind::Gesture,
            serde_json::to_value(TrackpadGesture {
                gesture: "pinch".into(),
                fingers: Some(4),
            })
            .unwrap(),
        ),
        (
            "hello",
            MessageKind::Hello,
            serde_json::to_value(sill_proto::compat::Hello {
                app_version: Some("1.0".into()),
                build: Some("1".into()),
                protocol: Some(1),
                device: Some("Windows PC".into()),
                audio: Some(vec!["aac-eld".into()]),
            })
            .unwrap(),
        ),
        (
            "stats",
            MessageKind::ClientStats,
            serde_json::to_value(ClientStats {
                fps: 0,
                frame_age_ms: 0,
                rtt_ms: 0,
                device: "Windows PC".into(),
                frame_age_max_ms: None,
                rtt_max_ms: None,
                audio_behind_ms: None,
                audio_late: None,
            })
            .unwrap(),
        ),
    ];
    for (name, kind, rust_json) in cases {
        let m = read_one(&built(&o, name));
        assert_eq!(m.kind, kind, "{name}");
        let py_json: Value = serde_json::from_slice(&m.payload).unwrap();
        // Python's JSON (with its ", " separators and "scale": null) is the same value as Rust's,
        // except where Python writes an explicit null, which Swift reads as nil.
        let mut py = norm(&py_json);
        if let Some(obj) = py.as_object_mut() {
            obj.retain(|_, v| !v.is_null());
        }
        assert_eq!(py, norm(&rust_json), "{name}");
    }
    // And Rust's typed decode accepts every Python payload, including the explicit null.
    let vp: Viewport = wire::decode_object(&read_one(&built(&o, "viewport_60")).payload).unwrap();
    assert_eq!(vp.fps, Some(60));
    assert_eq!(vp.scale, None);
    let src: StreamSource =
        wire::decode_object(&read_one(&built(&o, "pick_window")).payload).unwrap();
    assert_eq!(src, StreamSource::Window(123));
    let key: InputEvent = wire::decode_object(&read_one(&built(&o, "key_down")).payload).unwrap();
    assert_eq!(
        key,
        InputEvent::Key {
            hid_usage: 44,
            down: true,
            modifiers: 0
        }
    );
    let g: TrackpadGesture =
        wire::decode_object(&read_one(&built(&o, "gesture_pinch_4")).payload).unwrap();
    assert_eq!(g.fingers, Some(4));
}

#[test]
fn rust_messages_parse_in_python_the_way_the_script_reads_them() {
    // Build with Rust, parse with the script's own struct/json calls.
    let msgs: Vec<Message> = vec![
        Message::new(
            MessageKind::Hello,
            TS,
            false,
            wire::encode(&sill_proto::compat::Hello {
                app_version: Some("0.1".into()),
                protocol: Some(1),
                device: Some("Windows PC".into()),
                audio: Some(vec!["aac-eld".into()]),
                build: None,
            }),
        ),
        Message::new(
            MessageKind::SelectSource,
            TS,
            false,
            wire::encode(&StreamSource::Window(42)),
        ),
        Message::new(
            MessageKind::Viewport,
            TS,
            false,
            wire::encode(&Viewport {
                width: 1117.0,
                height: 642.0,
                scale: Some(1.5),
                local_cursor: None,
                fps: Some(120),
            }),
        ),
        Message::new(
            MessageKind::Input,
            TS,
            false,
            wire::encode(&InputEvent::Text {
                text: "héllo".into(),
            }),
        ),
        Message::new(
            MessageKind::Input,
            TS,
            false,
            wire::encode(&InputEvent::Scroll {
                x: 0.5,
                y: 0.5,
                dx: 0.0,
                dy: -0.25,
            }),
        ),
        Message::new(
            MessageKind::ChangeSettings,
            TS,
            false,
            wire::encode(&HostSettingsChange {
                token: Some(1),
                bitrate: Some(25_000_000),
                ..Default::default()
            }),
        ),
        Message::new(MessageKind::Ping, TS, false, 3.5f64.to_be_bytes().to_vec()),
        Message::new(MessageKind::KeyframeRequest, TS, false, vec![]),
        Message::new(MessageKind::Frame, TS, true, vec![0, 0, 0, 1, 0x26]),
    ];
    let stdin: String = msgs
        .iter()
        .map(|m| bytes_to_hex(&m.serialize().unwrap()) + "\n")
        .collect();
    let parser = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/parse_stdin.py");
    let out: Value =
        serde_json::from_str(&python(&[parser.to_str().unwrap()], Some(&stdin))).unwrap();
    let out = out.as_array().unwrap();
    assert_eq!(out.len(), msgs.len());
    for (m, p) in msgs.iter().zip(out) {
        assert_eq!(p["kind"].as_u64().unwrap() as u8, m.kind.as_u8());
        assert_eq!(p["timestamp"].as_f64().unwrap(), m.timestamp);
        assert_eq!(p["key"].as_u64().unwrap() == 1, m.is_keyframe);
        assert_eq!(p["len"].as_u64().unwrap() as usize, m.payload.len());
    }
    assert_eq!(out[1]["json"], json!({"window": {"_0": 42}}));
    assert_eq!(out[3]["json"], json!({"text": {"_0": "héllo"}}));
    assert_eq!(out[5]["json"], json!({"token": 1, "bitrate": 25000000}));
    assert_eq!(out[0]["json"]["audio"], json!(["aac-eld"]));
    assert!(out[7].get("json").is_none());
}

#[test]
fn header_layout_equals_pythons_struct_pack() {
    // `struct.pack(">BdBI", kind, ts, key, len)` (sillclient.py:188) for a spread of values.
    let o = oracle();
    let b = built(&o, "keyframe_flag");
    let h = Header::parse(&b).unwrap();
    assert_eq!(h.timestamp, TS);
    assert_eq!(
        &b[..14],
        &Message::new(MessageKind::Frame, TS, true, vec![0, 1, 2])
            .serialize()
            .unwrap()[..14]
    );
}
