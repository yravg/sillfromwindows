mod common;
use common::*;
use sill_media::audio::AacEldDecoder;
use sill_media::video::MediaError;
use sill_proto::audio::{AudioFormat, AudioMessage};
use std::process::Command;

fn make(f: &AudioFormat) -> Option<AacEldDecoder> {
    match AacEldDecoder::new(f) {
        Ok(d) => Some(d),
        Err(MediaError::NotLoaded(why)) => {
            eprintln!("FFmpeg libraries not available here, skipping: {why}");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn real_capture_audio_decodes() {
    let path = fixture("desktop-10s.sill");
    if !path.exists() {
        eprintln!("desktop-10s.sill is a local-only recording (gitignored): skipped");
        return;
    }
    let mut dec: Option<AacEldDecoder> = None;
    let mut fmt: Option<AudioFormat> = None;
    let (mut packets, mut samples, mut peak) = (0usize, 0usize, 0f32);
    for m in read_sill(&path).iter().filter(|m| m.kind == 43) {
        match AudioMessage::parse(&m.payload).expect("kind 43 parses") {
            AudioMessage::Format(f) => {
                if dec.is_none() {
                    dec = make(&f);
                    if dec.is_none() {
                        return;
                    }
                }
                fmt = Some(f);
            }
            AudioMessage::Packets(p) => {
                let f = fmt.as_ref().unwrap();
                let (per, prim) = (
                    f.frames_per_packet.unwrap() as usize,
                    f.priming_frames.unwrap() as usize,
                );
                for (i, pk) in p.packets.iter().enumerate() {
                    let out = dec.as_mut().unwrap().decode(pk).expect("packet decodes");
                    assert_eq!(out.len() % 2, 0);
                    let frames = out.len() / 2;
                    if packets == 0 && i == 0 {
                        assert_eq!(
                            frames,
                            per - prim,
                            "the first packet loses the priming frames"
                        );
                    } else {
                        assert_eq!(frames, per, "480 or 512 frames a packet");
                    }
                    peak = out.iter().fold(peak, |a, s| a.max(s.abs()));
                    packets += 1;
                    samples += frames;
                }
            }
        }
    }
    eprintln!("real capture audio: {packets} packets, {samples} frames, peak {peak}");
    assert!(packets > 0);
    assert!(peak <= 1.5);
}

/// ffmpeg's own `aac` encoder makes AAC-LC only (ELD needs libfdk, absent here), so this checks
/// the decode path (bare-ASC cookie, planar float to interleaved, 1024-sample frames), not ELD.
#[test]
fn synthetic_aac_lc_through_the_same_path() {
    if !have_ffmpeg_cli() {
        eprintln!("no ffmpeg CLI: skipped");
        return;
    }
    let r = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=0.5",
            "-ac",
            "2",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-f",
            "adts",
            "-",
        ])
        .output()
        .unwrap();
    assert!(r.status.success());
    // ADTS: 7-byte headers (protection absent), 13-bit frame length at bytes 3..6.
    let d = r.stdout;
    let mut o = 0;
    let fmt = AudioFormat {
        codec: Some("aac-eld".into()),
        sample_rate: Some(48000),
        channels: Some(2),
        frames_per_packet: Some(1024),
        priming_frames: Some(0),
        // AAC-LC, 48 kHz, stereo: 00010 0011 0010 000
        cookie: Some(vec![0x11, 0x90]),
        ..Default::default()
    };
    let Some(mut dec) = make(&fmt) else { return };
    let mut total = 0;
    let mut energy = 0f64;
    while o + 7 <= d.len() {
        let len = (((d[o + 3] & 3) as usize) << 11)
            | ((d[o + 4] as usize) << 3)
            | (d[o + 5] >> 5) as usize;
        let out = dec.decode(&d[o + 7..o + len]).unwrap();
        assert_eq!(out.len() % 2, 0);
        // Left and right carry the same sine.
        for p in out.chunks(2) {
            assert!((p[0] - p[1]).abs() < 1e-3);
            energy += (p[0] * p[0]) as f64;
        }
        total += out.len() / 2;
        o += len;
    }
    assert!(total >= 24000, "{total} frames");
    assert!(energy / total as f64 > 0.003, "a tone, not silence");
}

#[test]
fn bare_asc_and_missing_cookie_both_open() {
    // The ELD ASC bare, and none at all (built from the format), against the real cookie's ASC.
    let base = AudioFormat {
        codec: Some("aac-eld".into()),
        sample_rate: Some(48000),
        channels: Some(2),
        frames_per_packet: Some(480),
        priming_frames: Some(240),
        ..Default::default()
    };
    let bare = AudioFormat {
        cookie: Some(vec![0xf8, 0xe6, 0x50, 0x00]),
        ..base.clone()
    };
    let Some(_a) = make(&bare) else { return };
    let _b = make(&base).unwrap();
    assert!(AacEldDecoder::new(&AudioFormat {
        codec: Some("opus".into()),
        ..base
    })
    .is_err());
}
