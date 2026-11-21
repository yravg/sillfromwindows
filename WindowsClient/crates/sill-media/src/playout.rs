//! When each decoded sample plays (iOSClient/AudioPlayout.swift's rules, in `timing`) and the output
//! (cpal/WASAPI on Windows; a silent sink on Linux, where the timing is tested alone).
//!
//! What this API gives the rules less than the Swift has, and what was chosen:
//! * no seq or epoch: a seq is counted here, and a stamp that does not follow the packet before it
//!   (by over half a packet) counts as a skipped one (rule 8); a new packet size is a new epoch;
//! * no codec priming: the caller's samples are already decoded, so priming is 0 (a segment start
//!   still fades in);
//! * no home or away: the away cover (150 ms to start, 60-600 ms) is the default; `set_away(false)`
//!   gives home's (40 ms, 20-150 ms);
//! * the picture's lag comes from `set_picture_lag_ms` (the whole lag, as rule 3 would measure it);
//!   until it is set there is no picture guard, only the need;
//! * the output latency, IO buffer and the timeline's reading come from the device callback (cpal's
//!   playback timestamp); the silent sink assumes none.
//!
//! The offset between the Mac's wall clock and this machine's is never needed: the first stamp is
//! subtracted, and the floor (rule 1) absorbs the rest.

pub mod engine;
pub mod timing;

use engine::Engine;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The timeline's rate and the input's: AAC-ELD arrives at 48 kHz.
const RATE: u32 = 48_000;

struct Shared {
    engine: Mutex<Engine>,
    start: Instant,
}

pub struct Playout {
    shared: Arc<Shared>,
    input_rate: u32,
    channels: u16,
    /// No device (Linux, or none could be opened): the clock drains the timeline.
    silent: bool,
    #[cfg(windows)]
    device: Option<output::Handle>,
}

impl Playout {
    /// Opens the output device on Windows; on Linux a silent sink.
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        let shared = Arc::new(Shared {
            engine: Mutex::new(Engine::new()),
            start: Instant::now(),
        });
        #[cfg(windows)]
        let device = output::open(shared.clone());
        #[cfg(windows)]
        let silent = device.is_none();
        #[cfg(not(windows))]
        let silent = true;
        Playout {
            shared,
            input_rate: sample_rate.max(1),
            channels: channels.max(1),
            silent,
            #[cfg(windows)]
            device,
        }
    }

    fn now(&self) -> f64 {
        self.shared.start.elapsed().as_secs_f64()
    }

    /// Decoded samples of one packet, interleaved; `wall_time` is the header stamp of its first
    /// sample (seconds since 1970, the Mac's clock), `segment_start` the packet's flag.
    pub fn push(&mut self, samples: &[f32], wall_time: f64, segment_start: bool) {
        let now = self.now();
        let resampled;
        let samples = if self.input_rate != RATE {
            resampled = resample(samples, self.channels as usize, self.input_rate, RATE);
            &resampled[..]
        } else {
            samples
        };
        let mut e = self.shared.engine.lock().unwrap();
        if self.silent {
            e.advance_silent(now);
        }
        e.push(
            samples,
            self.channels as usize,
            wall_time,
            segment_start,
            now,
        );
    }

    /// Muted: silence out, packets still counted; nothing in hand plays on.
    pub fn set_muted(&mut self, muted: bool) {
        self.shared.engine.lock().unwrap().set_muted(muted);
    }

    /// The picture's lag in ms, for the guard (AudioPlayout's).
    pub fn set_picture_lag_ms(&mut self, ms: f64) {
        let lag = if ms.is_finite() {
            Some(ms / 1000.0)
        } else {
            None
        };
        self.shared
            .engine
            .lock()
            .unwrap()
            .timing
            .set_lag_override(lag);
    }

    /// Home (true: away) changes the cover's start and bounds. Away by default.
    pub fn set_away(&mut self, away: bool) {
        self.shared.engine.lock().unwrap().timing.set_away(away);
    }

    pub fn stop(&mut self) {
        #[cfg(windows)]
        {
            if let Some(d) = self.device.take() {
                d.stop();
            }
            self.silent = true;
        }
        self.shared.engine.lock().unwrap().reset_session();
    }
}

impl Drop for Playout {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Linear resampling of interleaved samples (an input that is not 48 kHz).
pub fn resample(samples: &[f32], channels: usize, from: u32, to: u32) -> Vec<f32> {
    let frames = samples.len() / channels.max(1);
    if frames == 0 || from == to {
        return samples.to_vec();
    }
    let out_frames = (frames as u64 * to as u64 / from as u64) as usize;
    let mut out = Vec::with_capacity(out_frames * channels);
    for i in 0..out_frames {
        let pos = i as f64 * from as f64 / to as f64;
        let a = (pos.floor() as usize).min(frames - 1);
        let b = (a + 1).min(frames - 1);
        let t = (pos - a as f64) as f32;
        for c in 0..channels {
            let x = samples[a * channels + c];
            let y = samples[b * channels + c];
            out.push(x + (y - x) * t);
        }
    }
    out
}

#[cfg(windows)]
mod output {
    //! cpal's default output device, f32; the engine fills the callback (stereo to the first two
    //! channels, mono the mean, any device rate by linear interpolation). The stream lives on a
    //! thread of its own (it is not Send everywhere) until `stop`.
    use super::Shared;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::thread::JoinHandle;

    pub struct Handle {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }
    impl Handle {
        pub fn stop(mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    pub fn open(shared: Arc<Shared>) -> Option<Handle> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let (tx, rx) = mpsc::channel::<bool>();
        let thread = std::thread::Builder::new()
            .name("sill-audio-out".into())
            .spawn(move || {
                let built = (|| {
                    let host = cpal::default_host();
                    let device = host.default_output_device()?;
                    let supported = device.default_output_config().ok()?;
                    let cfg = supported.config();
                    let channels = cfg.channels as usize;
                    let rate = cfg.sample_rate;
                    let sh = shared.clone();
                    let stream = device
                        .build_output_stream(
                            cfg,
                            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                                let ts = info.timestamp();
                                let heard_in = ts
                                    .playback
                                    .saturating_duration_since(ts.callback)
                                    .as_secs_f64();
                                let now = sh.start.elapsed().as_secs_f64();
                                match sh.engine.try_lock() {
                                    Ok(mut e) => e.render(data, channels, rate, now, heard_in),
                                    Err(_) => data.fill(0.0),
                                }
                            },
                            |err| eprintln!("audio output: {err}"),
                            None,
                        )
                        .ok()?;
                    stream.play().ok()?;
                    Some(stream)
                })();
                let ok = built.is_some();
                let _ = tx.send(ok);
                if let Some(stream) = built {
                    while !stop2.load(Ordering::SeqCst) {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    drop(stream);
                }
            })
            .ok()?;
        if rx.recv().unwrap_or(false) {
            Some(Handle {
                stop,
                thread: Some(thread),
            })
        } else {
            let _ = thread.join();
            None
        }
    }
}
