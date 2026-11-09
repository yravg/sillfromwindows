//! OWNER: worker M1. HEVC (length-prefixed NALs + out-of-band parameter sets, as Sill for Mac
//! sends them: protocol-spec §1) decoded by FFmpeg to BGRA. The API below is the contract; keep it.
use sill_proto::framing::ParameterSets;

#[derive(Debug, Clone, PartialEq)]
pub enum MediaError {
    /// FFmpeg's libraries could not be loaded (which file, why).
    NotLoaded(String),
    /// A call failed (which, the error code).
    Ffmpeg(String),
    /// The input is not what Sill sends.
    BadInput(String),
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaError::NotLoaded(s) => write!(f, "FFmpeg isn't available: {s}"),
            MediaError::Ffmpeg(s) => write!(f, "FFmpeg: {s}"),
            MediaError::BadInput(s) => write!(f, "bad input: {s}"),
        }
    }
}

/// One decoded picture, BGRA8.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bgra: Vec<u8>,
}

pub struct HevcDecoder {}

impl HevcDecoder {
    pub fn new() -> Result<Self, MediaError> {
        Err(MediaError::NotLoaded("not built yet".into()))
    }
    /// Kind 0. A change of sets (a new size) starts the decoder afresh.
    pub fn set_parameter_sets(&mut self, _sets: &ParameterSets) -> Result<(), MediaError> {
        Ok(())
    }
    /// Kind 1's payload. None while waiting for a keyframe after the sets, or when the decoder
    /// holds the picture back.
    pub fn decode(&mut self, _avcc: &[u8], _keyframe: bool) -> Result<Option<DecodedFrame>, MediaError> {
        Ok(None)
    }
}
