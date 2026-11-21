mod common;
use common::*;
use sill_media::video::{HevcDecoder, MediaError};
use sill_proto::framing::ParameterSets;
use std::process::Command;

fn load_ffmpeg() -> Option<HevcDecoder> {
    match HevcDecoder::new() {
        Ok(d) => Some(d),
        Err(MediaError::NotLoaded(why)) => {
            eprintln!("FFmpeg libraries not available here, skipping: {why}");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

fn run(path: &str) -> Option<(Vec<sill_media::video::DecodedFrame>, Vec<common::Msg>)> {
    let msgs = read_sill(&fixture(path));
    let mut dec = load_ffmpeg()?;
    let mut out = Vec::new();
    for m in &msgs {
        match m.kind {
            0 => dec
                .set_parameter_sets(&ParameterSets::decode(&m.payload).unwrap())
                .unwrap(),
            1 => {
                if let Some(f) = dec.decode(&m.payload, m.key).unwrap() {
                    out.push(f);
                }
            }
            _ => {}
        }
    }
    Some((out, msgs))
}

#[test]
fn synthetic_every_frame_decodes_and_matches_ffmpeg() {
    let Some((frames, msgs)) = run("synthetic-hevc.sill") else {
        return;
    };
    assert_eq!(frames.len(), 30);
    for f in &frames {
        assert_eq!((f.width, f.height, f.stride), (320, 240, 1280));
        assert_eq!(f.bgra.len(), 320 * 240 * 4);
    }
    assert!(
        frames[0].bgra.iter().any(|b| *b != frames[0].bgra[0]),
        "a picture, not a flat fill"
    );
    if !have_ffmpeg_cli() {
        eprintln!("no ffmpeg CLI: pixel comparison skipped");
        return;
    }
    // Annex B again from the Sill framing, decoded by ffmpeg itself.
    let sets = ParameterSets::decode(&msgs[0].payload).unwrap();
    let mut annexb = Vec::new();
    for s in &sets.sets {
        annexb.extend_from_slice(&[0, 0, 0, 1]);
        annexb.extend_from_slice(s);
    }
    for m in msgs.iter().filter(|m| m.kind == 1) {
        let mut i = 0;
        while i < m.payload.len() {
            let n = u32::from_be_bytes(m.payload[i..i + 4].try_into().unwrap()) as usize;
            annexb.extend_from_slice(&[0, 0, 0, 1]);
            annexb.extend_from_slice(&m.payload[i + 4..i + 4 + n]);
            i += 4 + n;
        }
    }
    let src = scratch("synthetic.hevc");
    std::fs::write(&src, annexb).unwrap();
    let r = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&src)
        .args(["-f", "rawvideo", "-pix_fmt", "bgra", "-"])
        .output()
        .unwrap();
    assert!(r.status.success());
    assert_eq!(r.stdout.len(), 30 * 320 * 240 * 4);
    let mut worst = 0f64;
    for (k, f) in frames.iter().enumerate() {
        let refr = &r.stdout[k * 307_200..(k + 1) * 307_200];
        let sum: u64 = f
            .bgra
            .iter()
            .zip(refr)
            .map(|(a, b)| a.abs_diff(*b) as u64)
            .sum();
        worst = worst.max(sum as f64 / 307_200.0);
    }
    eprintln!("worst mean abs diff vs ffmpeg CLI: {worst:.3}");
    assert!(worst < 2.0, "mean abs diff {worst}");
}

#[test]
fn deltas_before_a_keyframe_are_dropped_and_new_sets_restart() {
    let msgs = read_sill(&fixture("synthetic-hevc.sill"));
    let Some(mut dec) = load_ffmpeg() else { return };
    let sets = ParameterSets::decode(&msgs[0].payload).unwrap();
    let frames: Vec<_> = msgs.iter().filter(|m| m.kind == 1).collect();
    // No sets yet: nothing.
    assert!(dec.decode(&frames[1].payload, false).unwrap().is_none());
    dec.set_parameter_sets(&sets).unwrap();
    // A delta right after the sets waits for a keyframe.
    assert!(dec.decode(&frames[1].payload, false).unwrap().is_none());
    assert!(dec.decode(&frames[0].payload, true).unwrap().is_some());
    // The same sets again do not restart: a delta goes on decoding.
    dec.set_parameter_sets(&sets).unwrap();
    assert!(dec.decode(&frames[1].payload, false).unwrap().is_some());
    // Different sets (a stand-in change) restart: deltas wait again.
    let mut other = sets.clone();
    other.sets.reverse();
    dec.set_parameter_sets(&other).unwrap();
    assert!(dec.decode(&frames[2].payload, false).unwrap().is_none());
    assert!(dec
        .set_parameter_sets(&ParameterSets {
            nal_unit_header_length: 4,
            sets: vec![]
        })
        .is_err());
    // Garbage framing is an error, not a crash.
    assert!(dec.decode(&[0, 0, 0, 9, 1], true).is_err());
}

#[test]
fn real_capture_all_frames_decode() {
    let path = fixture("desktop-10s.sill");
    if !path.exists() {
        eprintln!("desktop-10s.sill is a local-only recording (gitignored): skipped");
        return;
    }
    let Some((frames, msgs)) = run("desktop-10s.sill") else {
        return;
    };
    let sent = msgs.iter().filter(|m| m.kind == 1).count();
    assert_eq!(sent, 458);
    assert_eq!(frames.len(), 458, "every frame decodes");
    assert!(frames.iter().all(|f| (f.width, f.height) == (1512, 982)));

    // Decode speed, per frame (meaningful in --release).
    let mut dec = HevcDecoder::new().unwrap();
    let t = std::time::Instant::now();
    let mut n = 0;
    for m in &msgs {
        match m.kind {
            0 => dec
                .set_parameter_sets(&ParameterSets::decode(&m.payload).unwrap())
                .unwrap(),
            1 => {
                n += dec.decode(&m.payload, m.key).unwrap().is_some() as u32;
            }
            _ => {}
        }
    }
    eprintln!(
        "1512x982: {:.2} ms per frame (decode + BGRA), {n} frames",
        t.elapsed().as_secs_f64() * 1000.0 / n as f64
    );
}
