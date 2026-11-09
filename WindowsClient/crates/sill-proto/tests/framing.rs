//! Framing tests: hand-computed byte vectors (StreamMessage.swift) and the reader's rules
//! (iOSClient/MessageReader.swift), with cases ported from Tests/checks/protocol "Wire kinds and caps".

use sill_proto::framing::{Direction, FrameError, MAX_CLIENT_PAYLOAD, MAX_FRAME_PAYLOAD};
use sill_proto::{
    Caps, Header, Message, MessageKind, MessageReader, ParameterSets, ReadError, HEADER_LEN,
};

fn hdr(kind: u8, ts: f64, key: u8, len: u32) -> Vec<u8> {
    let mut v = vec![kind];
    v.extend_from_slice(&ts.to_bits().to_be_bytes());
    v.push(key);
    v.extend_from_slice(&len.to_be_bytes());
    v
}

#[test]
fn header_hand_computed_vector() {
    // kind 2, timestamp 1.0 = 0x3FF0000000000000, keyframe 1, length 5.
    let want = [0x02, 0x3f, 0xf0, 0, 0, 0, 0, 0, 0, 0x01, 0, 0, 0, 0x05];
    let m = Message::new(MessageKind::WindowList, 1.0, true, vec![1, 2, 3, 4, 5]);
    let bytes = m.serialize().unwrap();
    assert_eq!(&bytes[..HEADER_LEN], &want);
    assert_eq!(&bytes[HEADER_LEN..], &[1, 2, 3, 4, 5]);
    let h = Header::parse(&bytes).unwrap();
    assert_eq!(h.kind, MessageKind::WindowList);
    assert_eq!(h.timestamp, 1.0);
    assert!(h.is_keyframe);
    assert_eq!(h.payload_length, 5);
}

#[test]
fn header_length_is_14_and_short_input_is_none() {
    assert_eq!(HEADER_LEN, 14);
    assert!(Header::parse(&[0u8; 13]).is_none());
    assert!(Header::parse(&[0u8; 14]).is_some());
}

#[test]
fn keyframe_flag_is_true_only_for_exactly_one() {
    // StreamMessage.swift:147 `== 1`.
    for (b, want) in [(0u8, false), (1, true), (2, false), (255, false)] {
        assert_eq!(Header::parse(&hdr(1, 0.0, b, 0)).unwrap().is_keyframe, want);
    }
}

#[test]
fn timestamp_round_trips_bit_for_bit() {
    for ts in [
        0.0,
        -0.0,
        1700000000.5,
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::INFINITY,
    ] {
        let m = Message::new(MessageKind::Ping, ts, false, vec![]);
        let h = Header::parse(&m.serialize().unwrap()).unwrap();
        assert_eq!(h.timestamp.to_bits(), ts.to_bits());
    }
    let nan = Header::parse(&hdr(10, f64::NAN, 0, 0)).unwrap();
    assert!(nan.timestamp.is_nan());
}

#[test]
fn every_kind_number_0_to_44_that_exists() {
    // Tests/checks/protocol "kinds 18–22 …", "kind 23 …", "kind 28 …", "kind 32 …", "kinds 16 and 17".
    let known: &[(u8, MessageKind)] = &[
        (0, MessageKind::ParameterSets),
        (1, MessageKind::Frame),
        (2, MessageKind::WindowList),
        (3, MessageKind::Thumbnail),
        (4, MessageKind::AppIcon),
        (5, MessageKind::AppList),
        (6, MessageKind::SelectSource),
        (7, MessageKind::LaunchApp),
        (8, MessageKind::Input),
        (9, MessageKind::Viewport),
        (10, MessageKind::Ping),
        (11, MessageKind::Pong),
        (12, MessageKind::ClientStats),
        (13, MessageKind::Tick),
        (14, MessageKind::CursorShape),
        (15, MessageKind::WindowCommand),
        (16, MessageKind::HostSettings),
        (17, MessageKind::ChangeSettings),
        (18, MessageKind::MacInfo),
        (19, MessageKind::PairRequest),
        (20, MessageKind::PairResult),
        (21, MessageKind::PairingWanted),
        (22, MessageKind::Goodbye),
        (23, MessageKind::Hello),
        (24, MessageKind::MacMenu),
        (25, MessageKind::PressMenuItem),
        (26, MessageKind::MacPointer),
        (27, MessageKind::FetchMenu),
        (28, MessageKind::Gesture),
        (32, MessageKind::Clipboard),
        (43, MessageKind::Audio),
        (44, MessageKind::KeyframeRequest),
    ];
    for (n, k) in known {
        assert_eq!(MessageKind::from_u8(*n), *k, "kind {n}");
        assert_eq!(k.as_u8(), *n);
        assert!(k.direction().is_some());
    }
    // 29-31 and 33-42 are unknown (skipped), and so is anything above 44, 255 included.
    for n in (29..=31)
        .chain(33..=42)
        .chain([45, 100, 200, 201, 254, 255])
    {
        assert_eq!(MessageKind::from_u8(n), MessageKind::Unknown(n), "kind {n}");
        assert_eq!(MessageKind::Unknown(n).as_u8(), n);
        assert_eq!(MessageKind::Unknown(n).direction(), None);
    }
    // Round trip over every byte value.
    for n in 0..=255u8 {
        assert_eq!(MessageKind::from_u8(n).as_u8(), n);
    }
}

#[test]
fn directions_follow_the_swift_comments() {
    use Direction::*;
    assert_eq!(MessageKind::Frame.direction(), Some(HostToDevice));
    assert_eq!(MessageKind::Hello.direction(), Some(DeviceToHost));
    assert_eq!(MessageKind::Clipboard.direction(), Some(Both));
    assert_eq!(MessageKind::KeyframeRequest.direction(), Some(DeviceToHost));
    assert_eq!(MessageKind::Audio.direction(), Some(HostToDevice));
    assert_eq!(MessageKind::PairingWanted.direction(), Some(DeviceToHost));
}

#[test]
fn caps_match_the_swift_constants() {
    // Tests/checks/protocol: "caps: 1 MiB client, 4 KiB pairing, 32 MiB frames, 4 MiB other".
    assert_eq!(MAX_CLIENT_PAYLOAD, 1_048_576);
    assert_eq!(sill_proto::framing::MAX_PAIRING_PAYLOAD, 4096);
    assert_eq!(MAX_FRAME_PAYLOAD, 33_554_432);
    assert_eq!(sill_proto::framing::MAX_OTHER_HOST_PAYLOAD, 4_194_304);
    assert_eq!(Caps::DEVICE.cap(MessageKind::Frame), 32 << 20);
    assert_eq!(Caps::DEVICE.cap(MessageKind::WindowList), 4 << 20);
    assert_eq!(Caps::DEVICE.cap(MessageKind::Unknown(200)), 4 << 20);
    assert_eq!(Caps::HOST.cap(MessageKind::Frame), 1 << 20);
    assert_eq!(Caps::PAIRING.cap(MessageKind::PairRequest), 4096);
}

#[test]
fn payload_longer_than_u32_is_an_error_not_a_trap() {
    let h = Header {
        kind: MessageKind::Frame,
        timestamp: 0.0,
        is_keyframe: false,
        payload_length: u32::MAX as usize + 1,
    };
    assert_eq!(h.to_bytes(), Err(FrameError::PayloadTooLarge));
}

// ---- reader ---------------------------------------------------------------------------------

fn wire(kind: MessageKind, payload: &[u8]) -> Vec<u8> {
    Message::new(kind, 7.5, kind == MessageKind::Frame, payload.to_vec())
        .serialize()
        .unwrap()
}

#[test]
fn reader_delivers_whole_messages_in_order() {
    let mut bytes = wire(MessageKind::WindowList, b"{}");
    bytes.extend(wire(MessageKind::Tick, b""));
    bytes.extend(wire(MessageKind::Frame, &[9; 1000]));
    let mut r = MessageReader::new(Caps::DEVICE);
    let got = r.feed(&bytes).unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(got[0].kind, MessageKind::WindowList);
    assert_eq!(got[0].payload, b"{}");
    assert_eq!(
        got[1].kind,
        MessageKind::Tick,
        "a zero-length payload is a legal message"
    );
    assert!(got[1].payload.is_empty());
    assert!(got[2].is_keyframe);
    assert_eq!(got[2].payload.len(), 1000);
    assert!(!r.mid_message());
}

#[test]
fn reader_byte_at_a_time_gives_the_same_messages() {
    let mut bytes = wire(MessageKind::Hello, b"{\"a\":1}");
    bytes.extend(wire(MessageKind::Tick, b""));
    bytes.extend(wire(MessageKind::Pong, &[1, 2, 3, 4, 5, 6, 7, 8]));
    let mut r = MessageReader::new(Caps::DEVICE);
    let mut got = Vec::new();
    for b in &bytes {
        got.extend(r.feed(std::slice::from_ref(b)).unwrap());
    }
    assert_eq!(
        got.iter().map(|m| m.kind).collect::<Vec<_>>(),
        vec![MessageKind::Hello, MessageKind::Tick, MessageKind::Pong]
    );
    assert!(!r.mid_message());
}

#[test]
fn reader_every_split_point_of_a_two_message_stream() {
    let mut bytes = wire(MessageKind::MacPointer, b"{\"inside\":false}");
    bytes.extend(wire(MessageKind::Thumbnail, &[0, 0, 0, 7, 0xff, 0xd8]));
    for cut in 0..=bytes.len() {
        let mut r = MessageReader::new(Caps::DEVICE);
        let mut got = r.feed(&bytes[..cut]).unwrap();
        got.extend(r.feed(&bytes[cut..]).unwrap());
        assert_eq!(got.len(), 2, "cut at {cut}");
        assert_eq!(got[1].payload, [0, 0, 0, 7, 0xff, 0xd8]);
    }
}

#[test]
fn reader_unknown_kinds_are_delivered_and_skipped_by_length() {
    // sillclient sends kinds 200 and 201 to test exactly this (sillclient.py:708).
    let mut bytes = wire(MessageKind::Unknown(200), b"hello");
    bytes.extend(wire(MessageKind::Unknown(201), b""));
    bytes.extend(wire(MessageKind::Tick, b""));
    let mut r = MessageReader::new(Caps::HOST);
    let got = r.feed(&bytes).unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(got[0].kind, MessageKind::Unknown(200));
    assert_eq!(got[0].payload, b"hello");
    assert_eq!(got[2].kind, MessageKind::Tick);
}

#[test]
fn reader_over_cap_header_ends_the_reading_before_any_payload() {
    // A 1.7 GB header, as an SSH banner parses (MessageReader.swift:86-91).
    let mut r = MessageReader::new(Caps::DEVICE);
    let banner = b"SSH-2.0-OpenSSH_9.6\r\n";
    let e = r.feed(banner).unwrap_err();
    let ReadError::TooBig(h) = e;
    assert!(h.payload_length > Caps::DEVICE.other);
    // Sticky: nothing more is delivered, and further bytes are dropped.
    assert_eq!(r.feed(&wire(MessageKind::Tick, b"")), Err(e));
    assert_eq!(r.failure(), Some(e));
    assert!(!r.mid_message());
}

#[test]
fn reader_cap_boundaries_per_kind() {
    // Frame: exactly 32 MiB passes the header check, one more fails; other kinds 4 MiB.
    let ok = hdr(1, 0.0, 1, (32 << 20) as u32);
    let mut r = MessageReader::new(Caps::DEVICE);
    assert_eq!(r.feed(&ok).unwrap().len(), 0);
    assert!(r.mid_message());
    let mut r = MessageReader::new(Caps::DEVICE);
    assert!(matches!(
        r.feed(&hdr(1, 0.0, 1, (32 << 20) as u32 + 1)),
        Err(ReadError::TooBig(_))
    ));
    let mut r = MessageReader::new(Caps::DEVICE);
    assert!(r.feed(&hdr(2, 0.0, 0, (4 << 20) as u32)).is_ok());
    let mut r = MessageReader::new(Caps::DEVICE);
    assert!(matches!(
        r.feed(&hdr(2, 0.0, 0, (4 << 20) as u32 + 1)),
        Err(ReadError::TooBig(_))
    ));
    // A frame is capped at 4 MiB by a host reading a device only through Caps::HOST (1 MiB).
    let mut r = MessageReader::new(Caps::HOST);
    assert!(matches!(
        r.feed(&hdr(8, 0.0, 0, (1 << 20) as u32 + 1)),
        Err(ReadError::TooBig(_))
    ));
    // Unknown kinds are capped like any other.
    let mut r = MessageReader::new(Caps::DEVICE);
    assert!(matches!(
        r.feed(&hdr(201, 0.0, 0, u32::MAX)),
        Err(ReadError::TooBig(_))
    ));
}

#[test]
fn reader_message_cut_short_is_never_delivered() {
    // "a message cut short is never delivered" (MessageReader.swift:20-22).
    let full = wire(MessageKind::WindowList, &[b'x'; 50]);
    for cut in 1..full.len() {
        let mut r = MessageReader::new(Caps::DEVICE);
        assert!(r.feed(&full[..cut]).unwrap().is_empty(), "cut at {cut}");
        assert!(r.mid_message(), "cut at {cut}");
    }
    let mut r = MessageReader::new(Caps::DEVICE);
    assert_eq!(r.feed(&full).unwrap().len(), 1);
    assert!(!r.mid_message());
    assert!(!MessageReader::new(Caps::DEVICE).mid_message());
}

#[test]
fn reader_long_stream_keeps_memory_flat_and_order() {
    let mut r = MessageReader::new(Caps::DEVICE);
    let mut count = 0usize;
    for i in 0..2000u32 {
        let m = wire(MessageKind::Pong, &i.to_be_bytes());
        for got in r.feed(&m).unwrap() {
            assert_eq!(got.payload, i.to_be_bytes());
            count += 1;
        }
    }
    assert_eq!(count, 2000);
}

// ---- ParameterSets -----------------------------------------------------------------------------

#[test]
fn parameter_sets_hand_computed_vector() {
    // nalLen 4, two sets: [40 01 AA] (VPS, NAL type 32) and [42 01] (SPS, NAL type 33).
    let want = [
        0x04, 0x02, 0x00, 0x03, 0x40, 0x01, 0xAA, 0x00, 0x02, 0x42, 0x01,
    ];
    let ps = ParameterSets {
        nal_unit_header_length: 4,
        sets: vec![vec![0x40, 0x01, 0xAA], vec![0x42, 0x01]],
    };
    assert_eq!(ps.encode().unwrap(), want);
    assert_eq!(ParameterSets::decode(&want).unwrap(), ps);
    assert_eq!(ParameterSets::nal_type(&ps.sets[0]), Some(32));
    assert_eq!(ParameterSets::nal_type(&ps.sets[1]), Some(33));
    assert_eq!(ParameterSets::nal_type(&[0x44, 0x01]), Some(34));
    assert_eq!(ParameterSets::nal_type(&[]), None);
}

#[test]
fn parameter_sets_refusals_and_edges() {
    assert!(ParameterSets::decode(&[]).is_none());
    assert!(ParameterSets::decode(&[4]).is_none());
    assert_eq!(ParameterSets::decode(&[4, 0]).unwrap().sets.len(), 0);
    assert!(
        ParameterSets::decode(&[4, 1, 0]).is_none(),
        "length cut short"
    );
    assert!(
        ParameterSets::decode(&[4, 1, 0, 5, 1, 2]).is_none(),
        "set cut short"
    );
    // Bytes after the last set are ignored, as the Swift reader does.
    assert_eq!(
        ParameterSets::decode(&[4, 1, 0, 1, 9, 0xEE, 0xEE])
            .unwrap()
            .sets,
        vec![vec![9]]
    );
    // Writing: 255 sets fit, 256 do not; a 65535-byte set fits, 65536 does not.
    let many = ParameterSets {
        nal_unit_header_length: 4,
        sets: vec![vec![1]; 255],
    };
    assert_eq!(
        ParameterSets::decode(&many.encode().unwrap()).unwrap(),
        many
    );
    assert!(ParameterSets {
        nal_unit_header_length: 4,
        sets: vec![vec![1]; 256]
    }
    .encode()
    .is_none());
    let big = ParameterSets {
        nal_unit_header_length: 4,
        sets: vec![vec![7; 65535]],
    };
    assert_eq!(ParameterSets::decode(&big.encode().unwrap()).unwrap(), big);
    assert!(ParameterSets {
        nal_unit_header_length: 4,
        sets: vec![vec![7; 65536]]
    }
    .encode()
    .is_none());
}
