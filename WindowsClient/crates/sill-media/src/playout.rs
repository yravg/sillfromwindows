//! OWNER: worker M2. When each decoded sample plays (iOSClient/AudioPlayout.swift's rules) and the
//! output (cpal/WASAPI on Windows; none on Linux, where the timing is tested alone). Keep the API.

pub struct Playout {}

impl Playout {
    /// Opens the output device on Windows; on Linux a silent sink.
    pub fn new(_sample_rate: u32, _channels: u16) -> Self {
        Playout {}
    }
    /// Decoded samples of one packet, interleaved; `wall_time` is the header stamp of its first
    /// sample (seconds since 1970, the Mac's clock), `segment_start` the packet's flag.
    pub fn push(&mut self, _samples: &[f32], _wall_time: f64, _segment_start: bool) {}
    pub fn set_muted(&mut self, _muted: bool) {}
    /// The picture's lag in ms, for the guard (AudioPlayout's).
    pub fn set_picture_lag_ms(&mut self, _ms: f64) {}
    pub fn stop(&mut self) {}
}
