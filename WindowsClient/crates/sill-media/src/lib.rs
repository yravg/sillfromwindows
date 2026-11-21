//! Sill's media. FFmpeg (LGPL, shared libraries) is loaded at run time, never linked: avcodec,
//! avutil and swscale from the folder beside Sill.exe on Windows, the system's on Linux.
pub mod audio; // owner: worker M1 (AAC-ELD decode through ffmpeg.rs)
pub mod ffmpeg; // owner: worker M1 (the loader and the FFI subset)
pub mod playout; // owner: worker M2 (AudioPlayout port and the cpal output)
pub mod video; // owner: worker M1 (HEVC decode to BGRA)
