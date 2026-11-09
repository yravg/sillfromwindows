//! Shared by the media tests: Sill's framing (14-byte header: kind u8, f64 BE ts, keyframe u8,
//! u32 BE len), the fixtures' paths, and the ffmpeg CLI as a reference when it is installed.
#![allow(dead_code)]
use std::path::PathBuf;
use std::process::Command;

pub struct Msg {
    pub kind: u8,
    pub key: bool,
    pub payload: Vec<u8>,
}

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

pub fn read_sill(path: &PathBuf) -> Vec<Msg> {
    let d = std::fs::read(path).expect("fixture readable");
    let mut o = 0;
    let mut v = Vec::new();
    while o + 14 <= d.len() {
        let kind = d[o];
        let key = d[o + 9] != 0;
        let len = u32::from_be_bytes(d[o + 10..o + 14].try_into().unwrap()) as usize;
        v.push(Msg {
            kind,
            key,
            payload: d[o + 14..o + 14 + len].to_vec(),
        });
        o += 14 + len;
    }
    assert_eq!(o, d.len(), "fixture framing");
    v
}

pub fn have_ffmpeg_cli() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn scratch(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}
