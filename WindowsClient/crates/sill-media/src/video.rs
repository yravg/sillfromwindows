//! OWNER: worker M1. HEVC (length-prefixed NALs + out-of-band parameter sets, as Sill for Mac
//! sends them: protocol-spec §1) decoded by FFmpeg to BGRA. The API below is the contract; keep it.
use std::ffi::c_void;

use sill_proto::framing::ParameterSets;

use crate::ffmpeg::{self, Ffmpeg, AVERROR_EAGAIN, AV_PIX_FMT_BGRA, SWS_BILINEAR};

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

const START: [u8; 4] = [0, 0, 0, 1];

/// HEVC to BGRA. Latency choices: `AV_CODEC_FLAG_LOW_DELAY` (set by the loader), and
/// `thread_type=slice` with automatic threads: frame threading holds one picture back per extra
/// thread, a frame of latency each at 60 fps, and Sill's streams have no B-frames to repay it.
pub struct HevcDecoder {
    ff: &'static Ffmpeg,
    ctx: *mut c_void,
    frame: *mut c_void,
    pkt: *mut c_void,
    sws: *mut c_void,
    sets: Option<ParameterSets>,
    /// The sets as Annex B, put before each keyframe.
    annexb_sets: Vec<u8>,
    nal_len: usize,
    waiting_key: bool,
    scratch: Vec<u8>,
}

// The raw pointers are owned by this value alone and FFmpeg contexts have no thread affinity.
unsafe impl Send for HevcDecoder {}

impl HevcDecoder {
    pub fn new() -> Result<Self, MediaError> {
        let ff = ffmpeg::ffmpeg()?;
        let frame = unsafe { (ff.av_frame_alloc)() };
        let pkt = unsafe { (ff.av_packet_alloc)() };
        if frame.is_null() || pkt.is_null() {
            return Err(MediaError::Ffmpeg(
                "av_frame_alloc / av_packet_alloc returned null".into(),
            ));
        }
        Ok(HevcDecoder {
            ff,
            ctx: std::ptr::null_mut(),
            frame,
            pkt,
            sws: std::ptr::null_mut(),
            sets: None,
            annexb_sets: Vec::new(),
            nal_len: 4,
            waiting_key: true,
            scratch: Vec::new(),
        })
    }

    fn close(&mut self) {
        if !self.ctx.is_null() {
            unsafe { (self.ff.avcodec_free_context)(&mut self.ctx) };
            self.ctx = std::ptr::null_mut();
        }
    }

    /// Kind 0. A change of sets (a new size) starts the decoder afresh.
    pub fn set_parameter_sets(&mut self, sets: &ParameterSets) -> Result<(), MediaError> {
        if self.sets.as_ref() == Some(sets) && !self.ctx.is_null() {
            return Ok(());
        }
        if !(1..=4).contains(&sets.nal_unit_header_length) || sets.sets.is_empty() {
            return Err(MediaError::BadInput(
                "parameter sets: no sets or a bad length-prefix size".into(),
            ));
        }
        self.close();
        self.sets = None;
        let mut ab = Vec::new();
        for s in &sets.sets {
            ab.extend_from_slice(&START);
            ab.extend_from_slice(s);
        }
        self.ctx = self.ff.open_decoder(
            "hevc",
            &[("threads", "auto"), ("thread_type", "slice")],
            None,
        )?;
        self.annexb_sets = ab;
        self.nal_len = sets.nal_unit_header_length as usize;
        self.sets = Some(sets.clone());
        self.waiting_key = true;
        Ok(())
    }

    /// Kind 1's payload. None while waiting for a keyframe after the sets, or when the decoder
    /// holds the picture back.
    pub fn decode(
        &mut self,
        avcc: &[u8],
        keyframe: bool,
    ) -> Result<Option<DecodedFrame>, MediaError> {
        if self.ctx.is_null() || (self.waiting_key && !keyframe) {
            return Ok(None);
        }
        let mut buf = std::mem::take(&mut self.scratch);
        buf.clear();
        if keyframe {
            buf.extend_from_slice(&self.annexb_sets);
        }
        let n = self.nal_len;
        let mut i = 0;
        while i < avcc.len() {
            if i + n > avcc.len() {
                self.scratch = buf;
                return Err(MediaError::BadInput("frame: cut length prefix".into()));
            }
            let len = avcc[i..i + n]
                .iter()
                .fold(0usize, |a, b| (a << 8) | *b as usize);
            i += n;
            if len > avcc.len() - i {
                self.scratch = buf;
                return Err(MediaError::BadInput(
                    "frame: NAL length past the end".into(),
                ));
            }
            buf.extend_from_slice(&START);
            buf.extend_from_slice(&avcc[i..i + len]);
            i += len;
        }
        let sent = self.ff.send(self.ctx, self.pkt, &buf);
        self.scratch = buf;
        if let Err(e) = sent {
            self.waiting_key = true;
            return Err(e);
        }
        if keyframe {
            self.waiting_key = false;
        }
        let mut out = None;
        loop {
            let r = unsafe { (self.ff.avcodec_receive_frame)(self.ctx, self.frame) };
            if r == AVERROR_EAGAIN || r == ffmpeg::AVERROR_EOF {
                break;
            }
            if r < 0 {
                self.waiting_key = true;
                return Err(self.ff.err("avcodec_receive_frame", r));
            }
            let converted = self.convert_bgra();
            unsafe { (self.ff.av_frame_unref)(self.frame) };
            out = Some(converted?);
        }
        Ok(out)
    }

    fn convert_bgra(&mut self) -> Result<DecodedFrame, MediaError> {
        let ff = self.ff;
        let l = ff.layout;
        let w = ff.frame_i32(self.frame, l.frame_width);
        let h = ff.frame_i32(self.frame, l.frame_height);
        let fmt = ff.frame_i32(self.frame, l.frame_format);
        if w <= 0 || h <= 0 || fmt < 0 {
            return Err(MediaError::Ffmpeg(format!(
                "decoded frame {w}x{h} format {fmt}"
            )));
        }
        // The deprecated full-range "yuvj" formats are the plain ones plus a range flag.
        let (fmt, yuvj) = match fmt {
            12 => (0, true),
            13 => (4, true),
            14 => (5, true),
            f => (f, false),
        };
        let range = l.frame_color_range.map(|o| ff.frame_i32(self.frame, o));
        let space = l.frame_colorspace.map(|o| ff.frame_i32(self.frame, o));
        let src_full = i32::from(yuvj || range == Some(2));
        // AVCOL_SPC_* to SWS_CS_*: 601 family 5/6 -> 5, 240M 7 -> 7, BT.2020 9 -> 9; BT.709, and
        // anything unspecified (Sill's streams are HD), -> ITU709 (1).
        let sws_cs = match space {
            Some(5) | Some(6) => 5,
            Some(7) => 7,
            Some(9) => 9,
            _ => 1,
        };
        unsafe {
            self.sws = (ff.sws_getCachedContext)(
                self.sws,
                w,
                h,
                fmt,
                w,
                h,
                AV_PIX_FMT_BGRA,
                SWS_BILINEAR,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if self.sws.is_null() {
                return Err(MediaError::Ffmpeg(format!(
                    "sws_getCachedContext: no conversion from format {fmt}"
                )));
            }
            let src_table = (ff.sws_getCoefficients)(sws_cs);
            let dst_table = (ff.sws_getCoefficients)(1);
            (ff.sws_setColorspaceDetails)(
                self.sws,
                src_table,
                src_full,
                dst_table,
                1,
                0,
                1 << 16,
                1 << 16,
            );
        }
        let stride = w as usize * 4;
        let size = stride * h as usize;
        // swscale's SIMD may write a few bytes past a row's end: slack, then trimmed.
        let mut bgra = vec![0u8; size + ffmpeg::PADDING];
        let src: [*const u8; 4] = std::array::from_fn(|i| {
            if i < 3 {
                ff.frame_ptr(self.frame, l.frame_data, i)
            } else {
                std::ptr::null()
            }
        });
        let sls: [i32; 4] = std::array::from_fn(|i| {
            if i < 3 {
                ff.frame_i32(self.frame, l.frame_linesize + 4 * i)
            } else {
                0
            }
        });
        let dst: [*mut u8; 4] = [
            bgra.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ];
        let dls: [i32; 4] = [stride as i32, 0, 0, 0];
        let r = unsafe {
            (ff.sws_scale)(
                self.sws,
                src.as_ptr(),
                sls.as_ptr(),
                0,
                h,
                dst.as_ptr(),
                dls.as_ptr(),
            )
        };
        if r != h {
            return Err(MediaError::Ffmpeg(format!(
                "sws_scale converted {r} of {h} rows"
            )));
        }
        bgra.truncate(size);
        Ok(DecodedFrame {
            width: w as u32,
            height: h as u32,
            stride: stride as u32,
            bgra,
        })
    }
}

impl Drop for HevcDecoder {
    fn drop(&mut self) {
        self.close();
        unsafe {
            if !self.sws.is_null() {
                (self.ff.sws_freeContext)(self.sws);
            }
            (self.ff.av_frame_free)(&mut self.frame);
            (self.ff.av_packet_free)(&mut self.pkt);
        }
    }
}
