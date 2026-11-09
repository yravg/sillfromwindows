//! The pure engine behind `Playout`: the timing rules (timing.rs), the kept packets, and the output
//! timeline (a stereo ring at 48 kHz that the device callback drains). Every time comes from the
//! caller, so it runs without a device and is tested on Linux. The cpal callback, or on Linux a silent
//! sink advanced by the clock, calls `render`.

use super::timing::{self, Output, Part, Plan, Reset, Schedule, Timing, RATE};
use std::collections::{HashMap, VecDeque};

/// The output timeline: stereo frames from absolute sample `base`. Writes go at an absolute sample;
/// frames already past are skipped, gaps are silence.
pub struct Ring {
    base: i64,
    data: VecDeque<(f32, f32)>,
}

impl Ring {
    pub fn new() -> Self {
        Ring {
            base: 0,
            data: VecDeque::new(),
        }
    }
    pub fn base(&self) -> i64 {
        self.base
    }
    pub fn end(&self) -> i64 {
        self.base + self.data.len() as i64
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    /// Write `l`/`r` with the first frame at absolute sample `pos`.
    pub fn write_at(&mut self, pos: i64, l: &[f32], r: &[f32]) {
        let n = l.len().min(r.len());
        let mut skip = 0usize;
        let mut pos = pos;
        if pos < self.base {
            skip = ((self.base - pos) as usize).min(n);
            pos = self.base;
        }
        while self.end() < pos {
            self.data.push_back((0.0, 0.0));
        }
        for k in skip..n {
            let idx = (pos - self.base) as usize + (k - skip);
            if idx < self.data.len() {
                self.data[idx] = (l[k], r[k]);
            } else {
                self.data.push_back((l[k], r[k]));
            }
        }
    }
    /// Take `n` frames from the front (silence past the end) and advance the base.
    fn take(&mut self, n: usize) -> Vec<(f32, f32)> {
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.data.pop_front().unwrap_or((0.0, 0.0)));
        }
        self.base += n as i64;
        v
    }
    fn clear(&mut self) {
        self.data.clear();
    }
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

struct Kept {
    l: Vec<f32>,
    r: Vec<f32>,
}

pub struct Engine {
    pub timing: Timing,
    ring: Ring,
    kept: HashMap<usize, Kept>,
    /// Where the last schedule handed to the ring ends (for `at: None`).
    last_end: i64,
    /// Fractional position into the front frame when the device's rate is not 48 kHz.
    frac: f64,
    wall_base: Option<f64>,
    epoch: i64,
    seq: u32,
    last_stamp: Option<f64>,
    last_frames: usize,
    last_wall_end: f64,
    /// A silent sink's last advance.
    silent_at: Option<f64>,
    silent_frac: f64,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        let mut timing = Timing::new();
        // No Mac-side notion of home or away reaches this API: the away values (the safer, larger
        // cover) are the default; `set_away(false)` is the home one.
        timing.set_away(true);
        Engine {
            timing,
            ring: Ring::new(),
            kept: HashMap::new(),
            last_end: 0,
            frac: 0.0,
            wall_base: None,
            epoch: 0,
            seq: 0,
            last_stamp: None,
            last_frames: 0,
            last_wall_end: 0.0,
            silent_at: None,
            silent_frac: 0.0,
        }
    }

    pub fn ring(&self) -> &Ring {
        &self.ring
    }

    /// Decoded interleaved samples of one packet (`channels` per frame, at 48 kHz), `wall_time` its
    /// first sample's stamp on the Mac's clock, `now` this machine's monotonic seconds.
    pub fn push(
        &mut self,
        samples: &[f32],
        channels: usize,
        wall_time: f64,
        segment_start: bool,
        now: f64,
    ) {
        let channels = channels.max(1);
        let frames = samples.len() / channels;
        if frames == 0 || !wall_time.is_finite() {
            return;
        }
        let base = *self.wall_base.get_or_insert(wall_time);
        let stamp = wall_time - base;
        // A different packet size is a new decoder for the rules.
        if self.epoch == 0 || frames != self.timing.frames_per_packet() {
            self.epoch += 1;
            self.timing.format(self.epoch, frames, 0, &[]);
            self.seq = 0;
            self.last_stamp = None;
        }
        // The header carries no seq: a stamp that does not follow the one before (by over half a
        // packet) is a packet the Mac skipped, which the rules count as a missing seq (rule 8).
        match self.last_stamp {
            Some(prev) if !segment_start => {
                let expected = prev + self.last_frames as f64 / RATE;
                self.seq = self.seq.wrapping_add(
                    if (stamp - expected).abs() > frames as f64 / RATE / 2.0 {
                        2
                    } else {
                        1
                    },
                );
            }
            Some(_) => self.seq = self.seq.wrapping_add(1),
            None => {}
        }
        self.last_stamp = Some(stamp);
        self.last_frames = frames;
        self.last_wall_end = wall_time + frames as f64 / RATE;
        let plan = self
            .timing
            .packet(self.epoch, self.seq, stamp, segment_start, None, now);
        self.act_plan(plan, samples, channels, frames);
    }

    fn act_plan(&mut self, plan: Plan, samples: &[f32], channels: usize, frames: usize) {
        if let Some(id) = plan.id {
            let drop = plan.drop_front.min(frames);
            let mut l = Vec::with_capacity(frames - drop);
            let mut r = Vec::with_capacity(frames - drop);
            for f in drop..frames {
                let a = samples[f * channels];
                let b = if channels > 1 {
                    samples[f * channels + 1]
                } else {
                    a
                };
                l.push(a);
                r.push(b);
            }
            self.kept.insert(id, Kept { l, r });
        }
        self.apply(plan.output);
    }

    fn apply(&mut self, out: Output) {
        for s in out.schedules {
            self.write(s);
        }
        for id in out.discards {
            self.kept.remove(&id);
        }
    }

    fn write(&mut self, s: Schedule) {
        let Some(k) = self.kept.get(&s.id) else {
            return;
        };
        let end = (s.offset + s.count).min(k.l.len());
        let off = s.offset.min(end);
        let mut l = k.l[off..end].to_vec();
        let mut r = k.r[off..end].to_vec();
        if s.part == Part::Tail {
            // A tail is the packet's last piece: nothing more of it is kept.
            self.kept.remove(&s.id);
        }
        timing::change(&mut l, &mut r, s.change);
        if s.fade_in {
            timing::fade(&mut l, true, timing::fade_frames());
            timing::fade(&mut r, true, timing::fade_frames());
        }
        if s.fade_out {
            timing::fade(&mut l, false, timing::fade_frames());
            timing::fade(&mut r, false, timing::fade_frames());
        }
        let pos = match s.at {
            Some(at) => at.round() as i64,
            None => self.last_end,
        };
        self.last_end = pos + l.len() as i64;
        self.ring.write_at(pos, &l, &r);
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.timing.set_muted(muted);
        // Nothing of what was in hand plays on; the timeline starts again at the next reading.
        let out = self.timing.reset(Reset::Playback);
        self.apply(out);
        self.ring.clear();
        self.last_end = self.ring.end();
    }

    /// The device's latencies, seconds.
    pub fn latency(&mut self, output: f64, io: f64) {
        self.timing.latency(output, io, 1.0 / 60.0, 60.0);
    }

    /// Fill `out` (interleaved, `channels` per frame) at `device_rate` for a callback at `now`,
    /// whose first frame is heard `heard_in` seconds later (the output latency). Stereo goes to the
    /// first two channels (the rest silent); mono gets the mean. Underrun is silence. A rate other
    /// than 48 kHz is linear interpolation, with the fraction carried between calls.
    pub fn render(
        &mut self,
        out: &mut [f32],
        channels: usize,
        device_rate: u32,
        now: f64,
        heard_in: f64,
    ) {
        let channels = channels.max(1);
        let n = out.len() / channels;
        let io = n as f64 / device_rate.max(1) as f64;
        self.timing.latency(heard_in, io, 1.0 / 60.0, 60.0);
        self.read_and_tick(now);
        let step = RATE / device_rate.max(1) as f64;
        // Frames needed from the ring, plus one for interpolation.
        let need = ((self.frac + n as f64 * step).ceil() as usize) + 1;
        let src: Vec<(f32, f32)> = self
            .ring
            .data
            .iter()
            .copied()
            .chain(std::iter::repeat((0.0, 0.0)))
            .take(need)
            .collect();
        let mut pos = self.frac;
        for f in 0..n {
            let i = pos.floor() as usize;
            let t = (pos - i as f64) as f32;
            let (a, b) = src[i];
            let (c, d) = src[i + 1];
            let (l, r) = if (step - 1.0).abs() < 1e-12 {
                (a, b)
            } else {
                (a + (c - a) * t, b + (d - b) * t)
            };
            let o = &mut out[f * channels..(f + 1) * channels];
            if channels == 1 {
                o[0] = (l + r) / 2.0;
            } else {
                o[0] = l;
                o[1] = r;
                for x in o.iter_mut().skip(2) {
                    *x = 0.0;
                }
            }
            pos += step;
        }
        let consumed = pos.floor() as usize;
        self.frac = pos - consumed as f64;
        self.ring.take(consumed);
    }

    /// A sink with no device: the clock drains the ring at 48 kHz, and the reading is taken at `now`.
    pub fn advance_silent(&mut self, now: f64) {
        let last = self.silent_at.unwrap_or(now);
        self.silent_at = Some(now);
        let elapsed = (now - last).max(0.0) * RATE + self.silent_frac;
        let n = elapsed.floor();
        self.silent_frac = elapsed - n;
        self.ring.take(n as usize);
        self.read_and_tick(now);
    }

    fn read_and_tick(&mut self, now: f64) {
        // The timeline's reading: the front sample goes out now.
        let reading = self.ring.base() as f64 + self.frac;
        let o = self.timing.played(reading, now, now);
        self.apply(o);
        let o = self.timing.tick(now);
        self.apply(o);
    }

    pub fn reset_session(&mut self) {
        let out = self.timing.reset(Reset::Session);
        self.apply(out);
        self.ring.clear();
        self.kept.clear();
        self.last_end = self.ring.end();
        self.wall_base = None;
        self.epoch = 0;
        self.last_stamp = None;
    }
}
