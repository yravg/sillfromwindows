//! OWNER: worker M1. AAC-ELD (kind 43's format and packets) decoded by FFmpeg to f32. Keep the API.
use sill_proto::audio::AudioFormat;

use crate::video::MediaError;

pub struct AacEldDecoder {}

impl AacEldDecoder {
    pub fn new(_format: &AudioFormat) -> Result<Self, MediaError> {
        Err(MediaError::NotLoaded("not built yet".into()))
    }
    /// One AAC-ELD access unit to interleaved f32 samples at the format's rate and channels.
    pub fn decode(&mut self, _packet: &[u8]) -> Result<Vec<f32>, MediaError> {
        Ok(Vec::new())
    }
    /// A segment starts: drop the decoder's state (and the priming frames that follow).
    pub fn reset(&mut self) {}
}
