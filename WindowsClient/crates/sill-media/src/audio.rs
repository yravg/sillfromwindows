//! OWNER: worker M1. AAC-ELD (kind 43's format and packets) decoded by FFmpeg to f32. Keep the API.
//!
//! The format's `cookie` is what AudioToolbox returns for `kAudioConverterCompressionMagicCookie`
//! (AudioEncoder.swift:81-89): an MPEG-4 ES descriptor (tag 0x03 > DecoderConfigDescriptor 0x04 >
//! DecoderSpecificInfo 0x05 > AudioSpecificConfig). Real capture, 41 bytes:
//! `03 80808024 0000 00 | 04 80808016 40 14 001800 000001f4 000001f4 | 05 80808004 f8e65000 |
//! 06 80808001 02`, whose ASC `f8 e6 50 00` is AOT 39 (ELD, 5-bit escape 31 + 6 bits 7), 48 kHz
//! (index 3), 2 channels, frameLengthFlag 1 (480), no SBR. A bare ASC is handled too (it starts
//! with 0xF8 for ELD, never 0x03), and with no cookie the ASC is built from the format.
use std::ffi::c_void;

use sill_proto::audio::AudioFormat;

use crate::ffmpeg::{self, Ffmpeg, AVERROR_EAGAIN, AV_SAMPLE_FMT_FLT, AV_SAMPLE_FMT_FLTP};
use crate::video::MediaError;

/// A descriptor's size: 1 to 4 bytes of 7 bits, the top bit saying another follows.
fn desc_len(d: &[u8], at: &mut usize) -> Option<usize> {
    let mut n = 0usize;
    for _ in 0..4 {
        let b = *d.get(*at)?;
        *at += 1;
        n = (n << 7) | (b & 0x7f) as usize;
        if b & 0x80 == 0 {
            return Some(n);
        }
    }
    None
}

/// The AudioSpecificConfig inside an ES descriptor (tag 0x05's payload), or None when `cookie`
/// is not one.
pub fn asc_from_es_descriptor(cookie: &[u8]) -> Option<&[u8]> {
    fn walk(d: &[u8]) -> Option<&[u8]> {
        let mut i = 0;
        while i < d.len() {
            let tag = d[i];
            i += 1;
            let len = desc_len(d, &mut i)?;
            let body = d.get(i..i.checked_add(len)?)?;
            match tag {
                0x05 => return Some(body),
                // ES_Descriptor: ES_ID(2), flags(1) [+ dependsOn(2)] [+ URL] [+ OCR(2)], then more.
                0x03 => {
                    let flags = *body.get(2)?;
                    let mut j = 3;
                    if flags & 0x80 != 0 {
                        j += 2;
                    }
                    if flags & 0x40 != 0 {
                        j += 1 + *body.get(j)? as usize;
                    }
                    if flags & 0x20 != 0 {
                        j += 2;
                    }
                    if let Some(a) = walk(body.get(j..)?) {
                        return Some(a);
                    }
                }
                // DecoderConfigDescriptor: objectType(1) streamType(1) buffer(3) max(4) avg(4).
                0x04 => {
                    if let Some(a) = walk(body.get(13..)?) {
                        return Some(a);
                    }
                }
                _ => {}
            }
            i += len;
        }
        None
    }
    if cookie.first() == Some(&0x03) {
        walk(cookie)
    } else {
        None
    }
}

/// An ELD AudioSpecificConfig made from the format: AOT 39 (escape), rate index, channels,
/// frameLengthFlag (1 = 480), no resilience, no SBR, ELDEXT_TERM.
pub fn build_eld_asc(sample_rate: i64, channels: i64, frames_per_packet: i64) -> Option<Vec<u8>> {
    const RATES: [i64; 13] = [
        96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
    ];
    let idx = RATES.iter().position(|r| *r == sample_rate)? as u32;
    if !(1..=7).contains(&channels) {
        return None;
    }
    let flag = match frames_per_packet {
        480 => 1u32,
        512 => 0,
        _ => return None,
    };
    // 5 bits 31, 6 bits 39-32, 4 rate, 4 channels, 1 flag, 3 resilience, 1 sbr, 4 ext = 28 bits.
    let bits: u32 = (31 << 23) | (7 << 17) | (idx << 13) | ((channels as u32) << 9) | (flag << 8);
    Some(vec![
        (bits >> 20) as u8,
        (bits >> 12) as u8,
        (bits >> 4) as u8,
        ((bits & 0xf) << 4) as u8,
    ])
}

pub struct AacEldDecoder {
    ff: &'static Ffmpeg,
    ctx: *mut c_void,
    frame: *mut c_void,
    pkt: *mut c_void,
    channels: usize,
    priming: usize,
    /// Decoded frames still to drop: the codec's delay at the start of a segment.
    skip: usize,
}

// Owned by this value alone; FFmpeg contexts have no thread affinity.
unsafe impl Send for AacEldDecoder {}

impl AacEldDecoder {
    pub fn new(format: &AudioFormat) -> Result<Self, MediaError> {
        if let Some(c) = format.codec.as_deref() {
            if c != "aac-eld" {
                return Err(MediaError::BadInput(format!("audio codec {c}")));
            }
        }
        let rate = format.sample_rate.unwrap_or(48000);
        let channels = format.channels.unwrap_or(2);
        let frames = format.frames_per_packet.unwrap_or(480);
        let asc: Vec<u8> = match format.cookie.as_deref() {
            Some(c) if !c.is_empty() => match asc_from_es_descriptor(c) {
                Some(a) => a.to_vec(),
                None => c.to_vec(),
            },
            _ => build_eld_asc(rate, channels, frames).ok_or_else(|| {
                MediaError::BadInput("audio format without a cookie and not 48 kHz-like ELD".into())
            })?,
        };
        let ff = ffmpeg::ffmpeg()?;
        // FFmpeg's ELD path leaves the context's rate and layout unset (every frame is then
        // rejected as "invalid"), so both are given up front.
        let layout = match channels {
            1 => "mono".to_string(),
            2 => "stereo".to_string(),
            n => format!("{n}c"),
        };
        let rate_s = rate.to_string();
        let ctx = ff.open_decoder(
            "aac",
            &[("ar", &rate_s), ("ch_layout", &layout)],
            Some(&asc),
        )?;
        let frame = unsafe { (ff.av_frame_alloc)() };
        let pkt = unsafe { (ff.av_packet_alloc)() };
        if frame.is_null() || pkt.is_null() {
            return Err(MediaError::Ffmpeg(
                "av_frame_alloc / av_packet_alloc returned null".into(),
            ));
        }
        let priming = format.priming_frames.unwrap_or(0).max(0) as usize;
        Ok(AacEldDecoder {
            ff,
            ctx,
            frame,
            pkt,
            channels: channels.max(1) as usize,
            priming,
            skip: priming,
        })
    }

    /// One AAC-ELD access unit to interleaved f32 samples at the format's rate and channels. The
    /// first `primingFrames` frames of a segment are the codec's delay and are dropped, so the
    /// first packet of a segment returns `framesPerPacket - primingFrames` frames.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>, MediaError> {
        let ff = self.ff;
        ff.send(self.ctx, self.pkt, packet)?;
        let l = ff.layout;
        let mut out = Vec::new();
        loop {
            let r = unsafe { (ff.avcodec_receive_frame)(self.ctx, self.frame) };
            if r == AVERROR_EAGAIN || r == ffmpeg::AVERROR_EOF {
                break;
            }
            if r < 0 {
                return Err(ff.err("avcodec_receive_frame", r));
            }
            let n = ff.frame_i32(self.frame, l.frame_nb_samples).max(0) as usize;
            let fmt = ff.frame_i32(self.frame, l.frame_format);
            let skip = self.skip.min(n);
            self.skip -= skip;
            let ch = self.channels;
            out.reserve((n - skip) * ch);
            // Planes through extended_data: planar float has one per channel, packed float one.
            let planes = ff.frame_ptr(self.frame, l.frame_extended_data, 0) as *const *const f32;
            unsafe {
                match fmt {
                    AV_SAMPLE_FMT_FLTP => {
                        for i in skip..n {
                            for c in 0..ch {
                                out.push(*(*planes.add(c)).add(i));
                            }
                        }
                    }
                    AV_SAMPLE_FMT_FLT => {
                        let p = *planes;
                        for i in skip * ch..n * ch {
                            out.push(*p.add(i));
                        }
                    }
                    other => {
                        (ff.av_frame_unref)(self.frame);
                        return Err(MediaError::Ffmpeg(format!(
                            "aac decoded sample format {other}"
                        )));
                    }
                }
                (ff.av_frame_unref)(self.frame);
            }
        }
        Ok(out)
    }

    /// A segment starts: drop the decoder's state (and the priming frames that follow).
    pub fn reset(&mut self) {
        unsafe { (self.ff.avcodec_flush_buffers)(self.ctx) };
        self.skip = self.priming;
    }
}

impl Drop for AacEldDecoder {
    fn drop(&mut self) {
        unsafe {
            (self.ff.avcodec_free_context)(&mut self.ctx);
            (self.ff.av_frame_free)(&mut self.frame);
            (self.ff.av_packet_free)(&mut self.pkt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn es_descriptor_and_built_asc_agree() {
        let cookie = hex(
            "0380808024000000048080801640140018000001f4000001f4000580808004f8e65000068080800102",
        );
        assert_eq!(asc_from_es_descriptor(&cookie), Some(&hex("f8e65000")[..]));
        assert_eq!(build_eld_asc(48000, 2, 480), Some(hex("f8e65000")));
        assert_eq!(asc_from_es_descriptor(&hex("f8e65000")), None);
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }
}
