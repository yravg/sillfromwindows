//! FFmpeg's shared libraries, loaded at run time (never linked): `avutil`, `avcodec`, `swscale`.
//!
//! Windows: the DLLs sit beside Sill.exe (`avcodec-61.dll` ...). Linux: the system's sonames.
//! `SILL_FFMPEG_DIR` (a folder) is tried first on both, for tests and unusual installs.
//! FFmpeg 7 (61, 59, 8), FFmpeg 8 (62, 60, 9; what Windows ships) and FFmpeg 6 (60, 58, 7) are supported; the major
//! versions are read from the loaded libraries and anything else is refused by name.
//!
//! Struct fields are read through accessors where FFmpeg has them. The few it has not are read at
//! fixed offsets of the 64-bit layout (`Layout`, chosen by the loaded avcodec major), checked
//! when a context is made. AVFrame's are the same in 6, 7 and 8 (8.1: compiled offsetof against
//! the n8.1.3 headers). AVCodecContext's differ: FFmpeg 8 flags 64, extradata 72, size 80:
//!
//! * `AVFrame` (libavutil/frame.h): `uint8_t *data[8]` at 0, `int linesize[8]` at 64,
//!   `uint8_t **extended_data` at 96, `int width` at 104, `int height` at 108, `int nb_samples`
//!   at 112, `int format` at 116. Those lead the struct and were not touched between 6.0 and 7.1.
//! * `AVCodecContext` (libavcodec/avcodec.h), FFmpeg 6/7: `const AVCodec *codec` at 16, `int flags` at 76,
//!   `uint8_t *extradata` at 88, `int extradata_size` at 96 (av_class 8, log_level_offset 4,
//!   codec_type 4, codec 8, codec_id 4, codec_tag 4, priv_data 8, internal 8, opaque 8, bit_rate 8,
//!   bit_rate_tolerance 4, global_quality 4, compression_level 4, flags 4, flags2 4, pad 4).
//!   `codec` and `flags` are verified after `avcodec_alloc_context3` (the pointer given, and an
//!   `av_opt_set` of flags read back), so a layout that moved is an error, never a wild write.
//! * `AVPacket` is never touched: `av_packet_from_data` fills it.
#![allow(non_snake_case)]
// The raw pointers these helpers take are FFmpeg objects this crate made; they are not a public API.
#![allow(clippy::not_unsafe_ptr_arg_deref)]
use std::ffi::{c_char, c_int, c_void, CString};
use std::path::PathBuf;
use std::sync::OnceLock;

use libloading::Library;

use crate::video::MediaError;

pub const AV_PIX_FMT_BGRA: c_int = 28;
pub const AV_SAMPLE_FMT_FLT: c_int = 3;
pub const AV_SAMPLE_FMT_FLTP: c_int = 8;
pub const AVERROR_EAGAIN: c_int = -11;
pub const AVERROR_EOF: c_int = -541_478_725;
pub const PADDING: usize = 64;
pub const SWS_BILINEAR: c_int = 2;
/// `AV_CODEC_FLAG_LOW_DELAY` (1 << 19), read back from `AVCodecContext.flags` as the layout check.
pub const FLAG_LOW_DELAY: c_int = 1 << 19;

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub ctx_codec: usize,
    pub ctx_flags: usize,
    pub ctx_extradata: usize,
    pub ctx_extradata_size: usize,
    pub frame_extended_data: usize,
    pub frame_width: usize,
    pub frame_height: usize,
    pub frame_nb_samples: usize,
    pub frame_format: usize,
    pub frame_data: usize,
    pub frame_linesize: usize,
    /// `AVFrame.color_range` and `.colorspace`; None where the layout is not verified (FFmpeg 7).
    pub frame_color_range: Option<usize>,
    pub frame_colorspace: Option<usize>,
}

const FRAME: Layout = Layout {
    ctx_codec: 0,
    ctx_flags: 0,
    ctx_extradata: 0,
    ctx_extradata_size: 0,
    frame_data: 0,
    frame_linesize: 64,
    frame_extended_data: 96,
    frame_width: 104,
    frame_height: 108,
    frame_nb_samples: 112,
    frame_format: 116,
    frame_color_range: None,
    frame_colorspace: None,
};

/// Offsets for avcodec 60 and 61 (FFmpeg 6 and 7): identical, listed per major so a new major is a
/// deliberate edit.
fn layout_for(avcodec_major: u32) -> Option<Layout> {
    match avcodec_major {
        60 | 61 => Some(Layout {
            ctx_codec: 16,
            ctx_flags: 76,
            ctx_extradata: 88,
            ctx_extradata_size: 96,
            // Found on FFmpeg 6.1.1 by decoding a limited-range BT.709 stream and the full-range
            // untagged capture: (range, primaries, trc, colorspace) = (1,1,1,1) and (2,2,2,2) at
            // 320, 324, 328, 332. For 7.x the same offsets are not verified: pixel format only.
            frame_color_range: if avcodec_major == 60 { Some(320) } else { None },
            frame_colorspace: if avcodec_major == 60 { Some(332) } else { None },
            ..FRAME
        }),
        // FFmpeg 8.1 (n8.1.3 headers, offsetof compiled against them): the deprecated
        // bit_rate_tolerance, global_quality and compression_level are gone, so flags moves up.
        62 => Some(Layout {
            ctx_codec: 16,
            ctx_flags: 64,
            ctx_extradata: 72,
            ctx_extradata_size: 80,
            // n8.1.3 headers: color_range 280, color_primaries 284, color_trc 288, colorspace 292.
            frame_color_range: Some(280),
            frame_colorspace: Some(292),
            ..FRAME
        }),
        _ => None,
    }
}

macro_rules! api {
    ($($field:ident : $sym:literal => $ty:ty),* $(,)?) => {
        /// The loaded libraries and every function this crate calls.
        pub struct Ffmpeg {
            _libs: Vec<Library>,
            pub layout: Layout,
            pub avcodec_major: u32,
            $(pub $field: $ty,)*
        }
        impl Ffmpeg {
            fn resolve(libs: Vec<Library>, layout: Layout, avcodec_major: u32, names: &[String]) -> Result<Self, MediaError> {
                $(
                    let $field: $ty = unsafe {
                        let mut found = None;
                        for (i, lib) in libs.iter().enumerate() {
                            if let Ok(s) = lib.get::<$ty>(concat!($sym, "\0").as_bytes()) {
                                found = Some(*s);
                                let _ = i;
                                break;
                            }
                        }
                        found.ok_or_else(|| MediaError::NotLoaded(format!(
                            "{}: symbol {} not found ({})", names.join(", "), $sym, "FFmpeg of another version?")))?
                    };
                )*
                Ok(Ffmpeg { _libs: libs, layout, avcodec_major, $($field,)* })
            }
        }
    };
}

api! {
    avcodec_version: "avcodec_version" => unsafe extern "C" fn() -> u32,
    avutil_version: "avutil_version" => unsafe extern "C" fn() -> u32,
    swscale_version: "swscale_version" => unsafe extern "C" fn() -> u32,
    av_malloc: "av_malloc" => unsafe extern "C" fn(usize) -> *mut c_void,
    av_mallocz: "av_mallocz" => unsafe extern "C" fn(usize) -> *mut c_void,
    av_free: "av_free" => unsafe extern "C" fn(*mut c_void),
    av_strerror: "av_strerror" => unsafe extern "C" fn(c_int, *mut c_char, usize) -> c_int,
    av_frame_alloc: "av_frame_alloc" => unsafe extern "C" fn() -> *mut c_void,
    av_frame_free: "av_frame_free" => unsafe extern "C" fn(*mut *mut c_void),
    av_frame_unref: "av_frame_unref" => unsafe extern "C" fn(*mut c_void),
    av_opt_set: "av_opt_set" => unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, c_int) -> c_int,
    av_opt_set_int: "av_opt_set_int" => unsafe extern "C" fn(*mut c_void, *const c_char, i64, c_int) -> c_int,
    avcodec_find_decoder_by_name: "avcodec_find_decoder_by_name" => unsafe extern "C" fn(*const c_char) -> *const c_void,
    avcodec_alloc_context3: "avcodec_alloc_context3" => unsafe extern "C" fn(*const c_void) -> *mut c_void,
    avcodec_free_context: "avcodec_free_context" => unsafe extern "C" fn(*mut *mut c_void),
    avcodec_open2: "avcodec_open2" => unsafe extern "C" fn(*mut c_void, *const c_void, *mut *mut c_void) -> c_int,
    avcodec_send_packet: "avcodec_send_packet" => unsafe extern "C" fn(*mut c_void, *const c_void) -> c_int,
    avcodec_receive_frame: "avcodec_receive_frame" => unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int,
    avcodec_flush_buffers: "avcodec_flush_buffers" => unsafe extern "C" fn(*mut c_void),
    av_packet_alloc: "av_packet_alloc" => unsafe extern "C" fn() -> *mut c_void,
    av_packet_free: "av_packet_free" => unsafe extern "C" fn(*mut *mut c_void),
    av_packet_unref: "av_packet_unref" => unsafe extern "C" fn(*mut c_void),
    av_packet_from_data: "av_packet_from_data" => unsafe extern "C" fn(*mut c_void, *mut u8, c_int) -> c_int,
    sws_getCachedContext: "sws_getCachedContext" => unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_int, c_int, c_int, c_int, *mut c_void, *mut c_void, *const f64) -> *mut c_void,
    sws_freeContext: "sws_freeContext" => unsafe extern "C" fn(*mut c_void),
    sws_scale: "sws_scale" => unsafe extern "C" fn(*mut c_void, *const *const u8, *const c_int, c_int, c_int, *const *mut u8, *const c_int) -> c_int,
    sws_getCoefficients: "sws_getCoefficients" => unsafe extern "C" fn(c_int) -> *const c_int,
    sws_setColorspaceDetails: "sws_setColorspaceDetails" => unsafe extern "C" fn(*mut c_void, *const c_int, c_int, *const c_int, c_int, c_int, c_int, c_int) -> c_int,
}

#[cfg(windows)]
const SONAMES: [(&str, &[&str]); 3] = [
    (
        "avutil",
        &["avutil-60.dll", "avutil-59.dll", "avutil-58.dll"],
    ),
    (
        "avcodec",
        &["avcodec-62.dll", "avcodec-61.dll", "avcodec-60.dll"],
    ),
    (
        "swscale",
        &["swscale-9.dll", "swscale-8.dll", "swscale-7.dll"],
    ),
];
#[cfg(not(windows))]
const SONAMES: [(&str, &[&str]); 3] = [
    (
        "avutil",
        &["libavutil.so.60", "libavutil.so.59", "libavutil.so.58"],
    ),
    (
        "avcodec",
        &["libavcodec.so.62", "libavcodec.so.61", "libavcodec.so.60"],
    ),
    (
        "swscale",
        &["libswscale.so.9", "libswscale.so.8", "libswscale.so.7"],
    ),
];

fn search_dirs() -> Vec<Option<PathBuf>> {
    let mut dirs = Vec::new();
    if let Some(d) = std::env::var_os("SILL_FFMPEG_DIR") {
        dirs.push(Some(PathBuf::from(d)));
    }
    #[cfg(windows)]
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
    {
        dirs.push(Some(d));
    }
    // On Linux the dynamic loader's own search; on Windows a last resort (the DLL search path).
    dirs.push(None);
    dirs
}

fn open_one(lib: &str, names: &[&str]) -> Result<(Library, String), MediaError> {
    let mut tried = Vec::new();
    for dir in search_dirs() {
        for name in names {
            let target = match &dir {
                Some(d) => d.join(name),
                None => PathBuf::from(name),
            };
            #[cfg(windows)]
            let res = unsafe {
                // LOAD_WITH_ALTERED_SEARCH_PATH: avcodec's own dependencies come from its folder.
                libloading::os::windows::Library::load_with_flags(&target, 0x8).map(Library::from)
            };
            #[cfg(not(windows))]
            let res = unsafe { Library::new(&target) };
            match res {
                Ok(l) => return Ok((l, target.display().to_string())),
                Err(e) => tried.push(format!("{}: {e}", target.display())),
            }
        }
    }
    Err(MediaError::NotLoaded(format!(
        "{lib} not found (tried {})",
        tried.join("; ")
    )))
}

fn load() -> Result<Ffmpeg, MediaError> {
    let mut libs = Vec::new();
    let mut names = Vec::new();
    for (lib, sonames) in SONAMES {
        let (l, path) = open_one(lib, sonames)?;
        libs.push(l);
        names.push(path);
    }
    // Versions first, from their own libraries, so a mismatch is named before anything else.
    let ver = |lib: &Library, sym: &str, name: &str| -> Result<u32, MediaError> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn() -> u32> = lib
                .get(format!("{sym}\0").as_bytes())
                .map_err(|e| MediaError::NotLoaded(format!("{name}: {sym}: {e}")))?;
            Ok(f() >> 16)
        }
    };
    let avutil = ver(&libs[0], "avutil_version", &names[0])?;
    let avcodec = ver(&libs[1], "avcodec_version", &names[1])?;
    let swscale = ver(&libs[2], "swscale_version", &names[2])?;
    let layout = layout_for(avcodec).ok_or_else(|| {
        MediaError::NotLoaded(format!(
            "{}: avcodec {avcodec} (avutil {avutil}, swscale {swscale}) is not FFmpeg 6, 7 or 8",
            names[1]
        ))
    })?;
    Resolved::check(avutil, avcodec, swscale, &names)?;
    Ffmpeg::resolve(libs, layout, avcodec, &names)
}

struct Resolved;
impl Resolved {
    /// FFmpeg 6 = avutil 58 / avcodec 60 / swscale 7; 7 = 59 / 61 / 8; 8 = 60 / 62 / 9. A mixed set is refused.
    fn check(avutil: u32, avcodec: u32, swscale: u32, names: &[String]) -> Result<(), MediaError> {
        let ok = matches!(
            (avutil, avcodec, swscale),
            (58, 60, 7) | (59, 61, 8) | (60, 62, 9)
        );
        if ok {
            Ok(())
        } else {
            Err(MediaError::NotLoaded(format!(
                "{}: libraries from different FFmpeg versions (avutil {avutil}, avcodec {avcodec}, swscale {swscale})",
                names.join(", ")
            )))
        }
    }
}

static FFMPEG: OnceLock<Result<Ffmpeg, MediaError>> = OnceLock::new();

/// The loaded FFmpeg, loaded once for the process.
pub fn ffmpeg() -> Result<&'static Ffmpeg, MediaError> {
    FFMPEG.get_or_init(load).as_ref().map_err(Clone::clone)
}

impl Ffmpeg {
    /// FFmpeg's words for an error code.
    pub fn err(&self, what: &str, code: c_int) -> MediaError {
        let mut buf = [0 as c_char; 128];
        let msg = unsafe {
            if (self.av_strerror)(code, buf.as_mut_ptr(), buf.len()) == 0 {
                std::ffi::CStr::from_ptr(buf.as_ptr())
                    .to_string_lossy()
                    .into_owned()
            } else {
                "unknown error".into()
            }
        };
        MediaError::Ffmpeg(format!("{what} failed ({code}: {msg})"))
    }

    pub fn find_decoder(&self, name: &str) -> Result<*const c_void, MediaError> {
        let c = CString::new(name).expect("static name");
        let p = unsafe { (self.avcodec_find_decoder_by_name)(c.as_ptr()) };
        if p.is_null() {
            return Err(MediaError::Ffmpeg(format!(
                "avcodec: no decoder named {name} in this FFmpeg build"
            )));
        }
        Ok(p)
    }

    pub fn opt(&self, ctx: *mut c_void, name: &str, value: &str) -> Result<(), MediaError> {
        let (n, v) = (CString::new(name).unwrap(), CString::new(value).unwrap());
        let r = unsafe { (self.av_opt_set)(ctx, n.as_ptr(), v.as_ptr(), 0) };
        if r < 0 {
            return Err(self.err(&format!("av_opt_set({name}={value})"), r));
        }
        Ok(())
    }

    /// A decoder context with low-delay flags and `threads`/`thread_type` options set, the layout
    /// verified, `extradata` copied in (with padding), then opened.
    pub fn open_decoder(
        &self,
        codec_name: &str,
        options: &[(&str, &str)],
        extradata: Option<&[u8]>,
    ) -> Result<*mut c_void, MediaError> {
        let codec = self.find_decoder(codec_name)?;
        let ctx = unsafe { (self.avcodec_alloc_context3)(codec) };
        if ctx.is_null() {
            return Err(MediaError::Ffmpeg(
                "avcodec_alloc_context3 returned null".into(),
            ));
        }
        let mut guard = ctx;
        let fail = |this: &Self, mut c: *mut c_void, e: MediaError| {
            unsafe { (this.avcodec_free_context)(&mut c) };
            e
        };
        // Layout check: the codec pointer, and flags written through av_opt and read back.
        let l = self.layout;
        unsafe {
            let codec_at = *((ctx as *const u8).add(l.ctx_codec) as *const *const c_void);
            if codec_at != codec {
                return Err(fail(
                    self,
                    guard,
                    MediaError::Ffmpeg(
                        "AVCodecContext layout differs from the known offsets (codec)".into(),
                    ),
                ));
            }
        }
        if let Err(e) = self.opt(ctx, "flags", "+low_delay") {
            return Err(fail(self, guard, e));
        }
        for (k, v) in options {
            if let Err(e) = self.opt(ctx, k, v) {
                return Err(fail(self, guard, e));
            }
        }
        let flags = unsafe { *((ctx as *const u8).add(l.ctx_flags) as *const c_int) };
        if flags & FLAG_LOW_DELAY == 0 {
            return Err(fail(
                self,
                guard,
                MediaError::Ffmpeg(
                    "AVCodecContext layout differs from the known offsets (flags)".into(),
                ),
            ));
        }
        if let Some(ed) = extradata {
            unsafe {
                let buf = (self.av_mallocz)(ed.len() + PADDING) as *mut u8;
                if buf.is_null() {
                    return Err(fail(
                        self,
                        guard,
                        MediaError::Ffmpeg("av_mallocz failed".into()),
                    ));
                }
                std::ptr::copy_nonoverlapping(ed.as_ptr(), buf, ed.len());
                *((ctx as *mut u8).add(l.ctx_extradata) as *mut *mut u8) = buf;
                *((ctx as *mut u8).add(l.ctx_extradata_size) as *mut c_int) = ed.len() as c_int;
            }
        }
        let r = unsafe { (self.avcodec_open2)(ctx, codec, std::ptr::null_mut()) };
        if r < 0 {
            let e = self.err(&format!("avcodec_open2({codec_name})"), r);
            guard = ctx;
            return Err(fail(self, guard, e));
        }
        Ok(ctx)
    }

    /// Copy `data` into an FFmpeg-owned padded buffer, hand it to a packet and send it.
    pub fn send(&self, ctx: *mut c_void, pkt: *mut c_void, data: &[u8]) -> Result<(), MediaError> {
        unsafe {
            let buf = (self.av_malloc)(data.len() + PADDING) as *mut u8;
            if buf.is_null() {
                return Err(MediaError::Ffmpeg("av_malloc failed".into()));
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len());
            std::ptr::write_bytes(buf.add(data.len()), 0, PADDING);
            let r = (self.av_packet_from_data)(pkt, buf, data.len() as c_int);
            if r < 0 {
                (self.av_free)(buf as *mut c_void);
                return Err(self.err("av_packet_from_data", r));
            }
            let r = (self.avcodec_send_packet)(ctx, pkt);
            (self.av_packet_unref)(pkt);
            if r < 0 {
                return Err(self.err("avcodec_send_packet", r));
            }
        }
        Ok(())
    }

    pub fn frame_i32(&self, frame: *const c_void, offset: usize) -> i32 {
        unsafe { *((frame as *const u8).add(offset) as *const i32) }
    }
    pub fn frame_ptr(&self, frame: *const c_void, offset: usize, index: usize) -> *const u8 {
        unsafe { *((frame as *const u8).add(offset) as *const *const u8).add(index) }
    }
}
