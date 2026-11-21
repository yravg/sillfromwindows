//! The timing rules of the sound's playout: a port of iOSClient/AudioPlayout.swift (docs/audio-plan.md
//! §7.2), pure, on a clock the caller gives. Device times are seconds of one monotonic clock;
//! stamps are the Mac's wall clock (the caller subtracts a constant so they stay small); the offset
//! between the two is never needed, since every comparison goes through the floor. Samples are the
//! output timeline at 48 kHz, whatever the device (the callback converts).
//!
//! The rules, as the Swift lists them: 1 the floor (smallest arrival - stamp of the last 10 s), 2 the
//! need (cover + one packet + IO + output latency + 2 ms), 3 the picture's lag, 4 the delay,
//! 5 placement by time then packet after packet, 6 late packets, 7 duplicates, 8 missing seqs,
//! 9 clean starts, 10 the held tail (the last 5 ms wait for the next packet), 11 drift a frame a packet,
//! 12 jumps over 40 ms or ahead of the picture, 13 a step of the Mac's clock or the path.
//!
//! Difference from the Swift: the picture's lag may be given directly (`set_lag_override`), as this
//! crate's API receives it from the video side instead of the frames' arrivals.

use std::f64::consts::PI;

pub const RATE: f64 = 48_000.0;
pub const FLOOR_SECONDS: i64 = 10;
pub const COVER_START_HOME: f64 = 0.040;
pub const COVER_START_AWAY: f64 = 0.150;
pub const COVER_GRACE: f64 = 2.0;
pub const COVER_HOME: (f64, f64) = (0.020, 0.150);
pub const COVER_AWAY: (f64, f64) = (0.060, 0.600);
pub const MARGIN: f64 = 0.002;
pub const PICTURE_SECONDS: f64 = 2.0;
pub const PICTURE_REFRESHES: f64 = 1.5;
pub const PICTURE_EXTRA: f64 = 0.004;
pub const CORRECT_FROM: f64 = 0.001;
pub const CORRECT_TO: f64 = 0.00025;
pub const JUMP_OVER: f64 = 0.040;
pub const STEP_ABOVE: f64 = 0.050;
pub const STEP_BELOW: f64 = 0.020;
pub const STEP_PACKETS: usize = 50;
pub const FADE_SECONDS: f64 = 0.005;
pub const DEDUPE_COUNT: usize = 256;
pub const WAITING_LIMIT: usize = 100;

/// A fade's frames at the timeline's rate: 5 ms.
pub fn fade_frames() -> usize {
    (FADE_SECONDS * RATE).round() as usize
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decision {
    pub due: f64,
    pub ear: f64,
    pub picture: f64,
}
impl Decision {
    pub fn error(&self) -> f64 {
        self.due - self.ear
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Body,
    Tail,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Schedule {
    pub id: usize,
    pub part: Part,
    pub offset: usize,
    pub count: usize,
    /// First frame's place on the timeline, or None: right after the buffer before it.
    pub at: Option<f64>,
    pub start: f64,
    pub frames: i64,
    pub change: i32,
    pub fade_in: bool,
    pub fade_out: bool,
    pub stamp: f64,
    pub seq: u32,
    pub epoch: i64,
    pub decision: Decision,
    pub goes_out: f64,
}
impl Schedule {
    pub fn end(&self) -> f64 {
        self.start + self.frames as f64
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    pub schedules: Vec<Schedule>,
    pub discards: Vec<usize>,
}
impl Output {
    fn add(&mut self, mut other: Output) {
        self.schedules.append(&mut other.schedules);
        self.discards.append(&mut other.discards);
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub decode: bool,
    pub reset_first: bool,
    pub drop_front: usize,
    pub id: Option<usize>,
    pub output: Output,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JumpCause {
    Step,
    Error,
    Ahead,
    Route,
}
impl JumpCause {
    pub fn words(self) -> &'static str {
        match self {
            JumpCause::Step => "clock step",
            JumpCause::Error => "off by over 40 ms",
            JumpCause::Ahead => "ahead of the picture",
            JumpCause::Route => "route",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reset {
    Session,
    Playback,
    Jump,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Second {
    pub behind_ms: Option<i64>,
    pub placed: u32,
    pub late: u32,
    pub duplicates: u32,
    pub frames_added: u32,
    pub frames_dropped: u32,
    pub jumps: u32,
    pub joins: u32,
    pub jump_drops: u32,
    pub need: f64,
    pub cover: f64,
    pub delay: f64,
    pub lag: Option<f64>,
    pub error: Option<f64>,
}

#[derive(Clone, Copy)]
struct Bucket {
    second: i64,
    min: f64,
    max: Option<f64>,
    #[allow(dead_code)]
    count: usize,
}

#[derive(Clone, Copy)]
struct Run {
    since: f64,
    count: usize,
    min: f64,
    max: f64,
}

#[derive(Clone, Copy)]
struct Admitted {
    id: usize,
    epoch: i64,
    seq: u32,
    stamp: f64,
    frames: usize,
    fade_in: bool,
    segment_start: bool,
}

#[derive(Clone, Copy)]
struct Planned {
    packet: Admitted,
    start: f64,
    contiguous: bool,
    change: i32,
    fade_in: bool,
    decision: Decision,
}
impl Planned {
    fn tail_count(&self) -> usize {
        fade_frames().min(self.packet.frames)
    }
    fn body_count(&self) -> usize {
        self.packet.frames - self.tail_count()
    }
    fn body_frames(&self) -> i64 {
        self.body_count() as i64 + self.change as i64
    }
    fn tail_start(&self) -> f64 {
        self.start + self.body_frames() as f64
    }
    fn end(&self) -> f64 {
        self.tail_start() + self.tail_count() as f64
    }
    fn takes(&self, next: &Admitted) -> bool {
        next.epoch == self.packet.epoch
            && next.seq == self.packet.seq.wrapping_add(1)
            && !next.segment_start
            && !next.fade_in
    }
    fn body(&self, goes_out: f64) -> Option<Schedule> {
        if self.body_count() == 0 {
            return None;
        }
        Some(Schedule {
            id: self.packet.id,
            part: Part::Body,
            offset: 0,
            count: self.body_count(),
            at: if self.contiguous {
                None
            } else {
                Some(self.start)
            },
            start: self.start,
            frames: self.body_frames(),
            change: self.change,
            fade_in: self.fade_in,
            fade_out: false,
            stamp: self.packet.stamp,
            seq: self.packet.seq,
            epoch: self.packet.epoch,
            decision: self.decision,
            goes_out,
        })
    }
    fn tail(&self, fade_out: bool, goes_out: f64) -> Schedule {
        let first = self.body_count() == 0;
        Schedule {
            id: self.packet.id,
            part: Part::Tail,
            offset: self.body_count(),
            count: self.tail_count(),
            at: if first && !self.contiguous {
                Some(self.start)
            } else {
                None
            },
            start: self.tail_start(),
            frames: self.tail_count() as i64,
            change: 0,
            fade_in: first && self.fade_in,
            fade_out,
            stamp: self.packet.stamp,
            seq: self.packet.seq,
            epoch: self.packet.epoch,
            decision: self.decision,
            goes_out,
        }
    }
}

pub struct Timing {
    epoch: Option<i64>,
    frames_per_packet: usize,
    priming_frames: usize,
    cookie: Vec<u8>,
    output_latency: f64,
    io_buffer: f64,
    refresh: f64,
    stream_fps: f64,
    rtt_median: Option<f64>,
    away: bool,
    muted: bool,
    buckets: Vec<Bucket>,
    floor_kept: Option<f64>,
    session_start: Option<f64>,
    above: Option<Run>,
    frame_arrivals: Vec<f64>,
    frame_raws: Vec<f64>,
    frame_head: usize,
    sorted_raws: Vec<f64>,
    last_picture_raw: Option<f64>,
    lag_override: Option<f64>,
    recent_keys: Vec<i64>,
    recent_set: std::collections::HashSet<i64>,
    recent_next: usize,
    expected_seq: Option<u32>,
    priming_left: usize,
    fade_next: bool,
    mapping: Option<(f64, f64)>,
    hand: Option<Planned>,
    chain_end: Option<f64>,
    waiting: Vec<Admitted>,
    next_id: usize,
    jump_owed: bool,
    owed_cause: Option<JumpCause>,
    made_jump: Option<(f64, JumpCause)>,
    correcting: i32,
    last_error: Option<f64>,
    second: Second,
    behind: Vec<f64>,
    placed_this_session: bool,
}

impl Default for Timing {
    fn default() -> Self {
        Self::new()
    }
}

fn median(sorted: &[f64]) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    }
}

fn insertion_index(sorted: &[f64], value: f64) -> usize {
    sorted.partition_point(|x| *x < value)
}

impl Timing {
    pub fn new() -> Self {
        Timing {
            epoch: None,
            frames_per_packet: 480,
            priming_frames: 240,
            cookie: Vec::new(),
            output_latency: 0.0,
            io_buffer: 0.005,
            refresh: 1.0 / 60.0,
            stream_fps: 60.0,
            rtt_median: None,
            away: false,
            muted: false,
            buckets: Vec::new(),
            floor_kept: None,
            session_start: None,
            above: None,
            frame_arrivals: Vec::new(),
            frame_raws: Vec::new(),
            frame_head: 0,
            sorted_raws: Vec::new(),
            last_picture_raw: None,
            lag_override: None,
            recent_keys: Vec::new(),
            recent_set: Default::default(),
            recent_next: 0,
            expected_seq: None,
            priming_left: 0,
            fade_next: true,
            mapping: None,
            hand: None,
            chain_end: None,
            waiting: Vec::new(),
            next_id: 1,
            jump_owed: false,
            owed_cause: None,
            made_jump: None,
            correcting: 0,
            last_error: None,
            second: Second::default(),
            behind: Vec::new(),
            placed_this_session: false,
        }
    }

    // ---- inputs

    /// A format message. True for a new epoch; false for the epoch playing with the same cookie.
    pub fn format(
        &mut self,
        epoch: i64,
        frames_per_packet: usize,
        priming_frames: usize,
        cookie: &[u8],
    ) -> bool {
        if self.epoch == Some(epoch)
            && self.cookie == cookie
            && self.frames_per_packet == frames_per_packet
            && self.priming_frames == priming_frames
        {
            return false;
        }
        self.epoch = Some(epoch);
        self.frames_per_packet = frames_per_packet.max(1);
        self.priming_frames = priming_frames;
        self.cookie = cookie.to_vec();
        self.expected_seq = None;
        self.priming_left = 0;
        self.fade_next = true;
        true
    }

    /// A video frame, stamped at arrival: for the picture's lag (rule 3).
    pub fn frame(&mut self, stamp: f64, arrival: f64) {
        let raw = arrival - stamp;
        if !raw.is_finite() {
            return;
        }
        self.frame_arrivals.push(arrival);
        self.frame_raws.push(raw);
        let i = insertion_index(&self.sorted_raws, raw);
        self.sorted_raws.insert(i, raw);
        self.expire_frames(arrival);
    }

    pub fn latency(&mut self, output: f64, io: f64, refresh: f64, stream_fps: f64) {
        self.output_latency = if output.is_finite() {
            output.max(0.0)
        } else {
            0.0
        };
        self.io_buffer = if io.is_finite() { io.max(0.0) } else { 0.0 };
        self.refresh = if refresh.is_finite() {
            refresh.max(0.0)
        } else {
            0.0
        };
        self.stream_fps = if stream_fps > 0.0 && stream_fps.is_finite() {
            stream_fps
        } else {
            60.0
        };
    }

    /// The packet size, learned from the first full packet, without a new epoch.
    pub fn set_frames_per_packet(&mut self, n: usize) {
        self.frames_per_packet = n.max(1);
    }

    /// The priming the decoder dropped from the next segment's first packet, without a new epoch.
    pub fn set_priming(&mut self, n: usize) {
        self.priming_frames = n;
    }

    pub fn rtt(&mut self, median: Option<f64>) {
        self.rtt_median = median.filter(|m| m.is_finite() && *m >= 0.0);
    }

    pub fn set_away(&mut self, away: bool) {
        self.away = away;
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// The picture's lag given directly, seconds (this crate's API); None goes back to rule 3's frames.
    pub fn set_lag_override(&mut self, lag: Option<f64>) {
        self.lag_override = lag.filter(|l| l.is_finite());
    }

    /// The player's timeline, read: `sample` goes out at device time `time`. The first reading after a
    /// start places what waited for it.
    pub fn played(&mut self, sample: f64, time: f64, now: f64) -> Output {
        if !sample.is_finite() || !time.is_finite() {
            return Output::default();
        }
        let first = self.mapping.is_none();
        self.mapping = Some((sample, time));
        if !first || self.waiting.is_empty() {
            return Output::default();
        }
        let mut out = Output::default();
        let list = std::mem::take(&mut self.waiting);
        for a in list {
            let o = self.place(a, now);
            out.add(o);
        }
        out
    }

    /// One packet. `arrival` is when it came, if it waited behind the engine starting (rule 6).
    pub fn packet(
        &mut self,
        epoch: i64,
        seq: u32,
        stamp: f64,
        segment_start: bool,
        arrival_came: Option<f64>,
        now: f64,
    ) -> Plan {
        let arrival = arrival_came.unwrap_or(now);
        let Some(playing) = self.epoch else {
            return Plan::default();
        };
        if epoch != playing || !stamp.is_finite() || !arrival.is_finite() {
            return Plan::default();
        }
        let key = ((epoch & 0xFFFF) << 32) | seq as i64;
        if self.recent_set.contains(&key) {
            self.second.duplicates += 1;
            return Plan::default();
        }
        self.remember(key);
        self.record(arrival - stamp, arrival);
        if self.muted {
            self.expected_seq = None;
            return Plan::default();
        }
        let mut plan = Plan {
            decode: true,
            ..Default::default()
        };
        let continuous = !segment_start && self.expected_seq == Some(seq);
        self.expected_seq = Some(seq.wrapping_add(1));
        if segment_start {
            plan.reset_first = true;
            self.priming_left = self.priming_frames;
            self.fade_next = true;
        } else if !continuous {
            plan.reset_first = true;
            plan.drop_front = self.frames_per_packet;
            self.priming_left = 0;
            self.fade_next = true;
            self.second.joins += 1;
            plan.output = self.hand_over(true);
            return plan;
        }
        let drop = self.priming_left.min(self.frames_per_packet);
        self.priming_left -= drop;
        plan.drop_front = drop;
        let kept = self.frames_per_packet - drop;
        if kept == 0 {
            return plan;
        }
        let a = Admitted {
            id: self.next_id,
            epoch,
            seq,
            stamp: stamp + drop as f64 / RATE,
            frames: kept,
            fade_in: self.fade_next,
            segment_start,
        };
        self.next_id += 1;
        self.fade_next = false;
        plan.id = Some(a.id);
        if self.mapping.is_none() {
            self.waiting.push(a);
            if self.waiting.len() > WAITING_LIMIT {
                plan.output.discards.push(self.waiting.remove(0).id);
                if let Some(first) = self.waiting.first_mut() {
                    first.fade_in = true;
                }
            }
            return plan;
        }
        let o = self.place(a, now);
        plan.output.add(o);
        plan
    }

    pub fn decode_failed(&mut self) {
        self.expected_seq = None;
        self.priming_left = 0;
        self.fade_next = true;
    }

    /// Time passing: the tail in hand goes out, faded, once its deadline has passed (rule 10).
    pub fn tick(&mut self, now: f64) -> Output {
        match self.deadline() {
            Some(d) if now >= d => self.hand_over(true),
            _ => Output::default(),
        }
    }

    pub fn deadline(&self) -> Option<f64> {
        let h = self.hand.as_ref()?;
        Some(self.time_of_sample(h.tail_start()) - self.io_buffer - MARGIN)
    }

    pub fn reset(&mut self, kind: Reset) -> Output {
        match kind {
            Reset::Jump => {
                self.owe(JumpCause::Route);
                Output::default()
            }
            Reset::Playback | Reset::Session => {
                let mut out = Output::default();
                if let Some(h) = &self.hand {
                    out.discards.push(h.packet.id);
                }
                out.discards.extend(self.waiting.iter().map(|w| w.id));
                self.hand = None;
                self.waiting.clear();
                self.chain_end = None;
                self.mapping = None;
                self.jump_owed = false;
                self.owed_cause = None;
                self.made_jump = None;
                self.correcting = 0;
                self.expected_seq = None;
                self.priming_left = 0;
                self.fade_next = true;
                if kind == Reset::Session {
                    self.epoch = None;
                    self.cookie.clear();
                    self.buckets.clear();
                    self.floor_kept = None;
                    self.session_start = None;
                    self.above = None;
                    self.frame_arrivals.clear();
                    self.frame_raws.clear();
                    self.frame_head = 0;
                    self.sorted_raws.clear();
                    self.last_picture_raw = None;
                    self.recent_keys.clear();
                    self.recent_set.clear();
                    self.recent_next = 0;
                    self.last_error = None;
                    self.placed_this_session = false;
                    self.second = Second::default();
                    self.behind.clear();
                }
                out
            }
        }
    }

    /// The second that ends now, and the next one starts.
    pub fn take_second(&mut self, now: f64) -> Second {
        self.expire_frames(now);
        let mut s = self.second.clone();
        s.behind_ms = if self.behind.is_empty() {
            None
        } else {
            let mut b = self.behind.clone();
            b.sort_by(|a, b| a.partial_cmp(b).unwrap());
            Some((median(&b) * 1000.0).round() as i64)
        };
        s.need = self.need(now);
        s.cover = self.cover(now);
        let lag = self.picture_lag(now);
        s.lag = lag;
        s.delay = self.delay(now, lag);
        s.error = self.last_error;
        self.second = Second::default();
        self.behind.clear();
        s
    }

    // ---- the numbers

    pub fn floor(&self) -> Option<f64> {
        if self.buckets.is_empty() {
            self.floor_kept
        } else {
            self.buckets.iter().map(|b| b.min).reduce(f64::min)
        }
    }

    pub fn cover(&self, now: f64) -> f64 {
        let start = if self.away {
            COVER_START_AWAY
        } else {
            COVER_START_HOME
        };
        let (Some(begun), Some(floor)) = (self.session_start, self.floor()) else {
            return start;
        };
        if now - begun < COVER_GRACE {
            return start;
        }
        let Some(top) = self.buckets.iter().filter_map(|b| b.max).reduce(f64::max) else {
            return start;
        };
        let (lo, hi) = if self.away { COVER_AWAY } else { COVER_HOME };
        (top - floor).max(lo).min(hi)
    }

    pub fn need(&self, now: f64) -> f64 {
        self.cover(now)
            + self.frames_per_packet as f64 / RATE
            + self.io_buffer
            + self.output_latency
            + MARGIN
    }

    pub fn picture_lag(&self, now: f64) -> Option<f64> {
        if let Some(l) = self.lag_override {
            return Some(l);
        }
        let floor = self.floor()?;
        let raw = if !self.sorted_raws.is_empty() && self.live_frames_after_expiry(now) > 0 {
            Some(median(&self.sorted_raws))
        } else {
            self.last_picture_raw
        };
        raw.map(|r| r - floor + PICTURE_REFRESHES * self.refresh + PICTURE_EXTRA)
    }

    pub fn delay(&self, now: f64, lag: Option<f64>) -> f64 {
        let guard = lag
            .map(|l| l + 1.0 / self.stream_fps)
            .unwrap_or(f64::NEG_INFINITY);
        self.need(now).max(guard)
    }

    pub fn holding(&self) -> bool {
        self.hand.is_some()
    }
    pub fn last_error(&self) -> Option<f64> {
        self.last_error
    }
    pub fn placed_this_session(&self) -> bool {
        self.placed_this_session
    }
    pub fn epoch(&self) -> Option<i64> {
        self.epoch
    }
    pub fn frames_per_packet(&self) -> usize {
        self.frames_per_packet
    }
    pub fn output_latency(&self) -> f64 {
        self.output_latency
    }

    /// The jump the last placement by time made, once: how far (seconds) and why.
    pub fn take_jump(&mut self) -> Option<(f64, JumpCause)> {
        self.made_jump.take()
    }

    // ---- placement

    fn place(&mut self, a: Admitted, now: f64) -> Output {
        let mut out = Output::default();
        if let Some(d) = self.deadline() {
            if now >= d {
                let o = self.hand_over(true);
                out.add(o);
            }
        }
        let Some(floor) = self.floor() else {
            return out;
        };
        self.expire_frames(now);
        let lag = self.picture_lag(now);
        let delay = self.delay(now, lag);
        let due = a.stamp + floor + delay;
        let picture = lag
            .map(|l| a.stamp + floor + l)
            .unwrap_or(f64::NEG_INFINITY);
        let mut placed: Option<Planned> = None;
        if let (Some(h), false) = (self.hand, self.jump_owed) {
            if h.takes(&a) {
                let start = h.end();
                let ear = self.time_of_sample(start) + self.output_latency;
                let decision = Decision { due, ear, picture };
                self.last_error = Some(decision.error());
                if ear >= picture && decision.error().abs() <= JUMP_OVER {
                    let change = if a.frames as i64 - fade_frames().min(a.frames) as i64 >= 4 {
                        self.correction(decision.error())
                    } else {
                        0
                    };
                    placed = Some(Planned {
                        packet: a,
                        start,
                        contiguous: true,
                        change,
                        fade_in: false,
                        decision,
                    });
                } else {
                    self.owe(if ear < picture {
                        JumpCause::Ahead
                    } else {
                        JumpCause::Error
                    });
                }
            }
        }
        if placed.is_none() {
            let mut start = self.sample_at_time(due - self.output_latency);
            if self.jump_owed {
                if let (Some(cause), Some(chain)) = (
                    self.owed_cause,
                    self.hand.map(|h| h.end()).or(self.chain_end),
                ) {
                    self.made_jump = Some(((start - chain) / RATE, cause));
                    self.owed_cause = None;
                }
            }
            if let Some(chain) = self.hand.map(|h| h.end()).or(self.chain_end) {
                if start < chain {
                    if self.jump_owed && chain - start > self.frames_per_packet as f64 / 2.0 {
                        self.second.jump_drops += 1;
                        let o = self.hand_over(true);
                        out.add(o);
                        out.discards.push(a.id);
                        return out;
                    }
                    start = chain;
                }
            }
            if self.time_of_sample(start) - now < self.io_buffer + MARGIN {
                self.second.late += 1;
                let o = self.hand_over(true);
                out.add(o);
                out.discards.push(a.id);
                return out;
            }
            self.jump_owed = false;
            self.correcting = 0;
            let ear = self.time_of_sample(start) + self.output_latency;
            placed = Some(Planned {
                packet: a,
                start,
                contiguous: false,
                change: 0,
                fade_in: true,
                decision: Decision { due, ear, picture },
            });
        }
        let Some(p) = placed else { return out };
        if let Some(h) = self.hand {
            out.schedules
                .push(h.tail(!p.contiguous, self.time_of_sample(h.tail_start())));
        }
        if let Some(body) = p.body(self.time_of_sample(p.start)) {
            out.schedules.push(body);
        }
        self.hand = Some(p);
        self.chain_end = None;
        self.second.placed += 1;
        self.placed_this_session = true;
        if p.change > 0 {
            self.second.frames_added += 1;
        } else if p.change < 0 {
            self.second.frames_dropped += 1;
        }
        if p.decision.picture > f64::NEG_INFINITY {
            self.behind.push(p.decision.ear - p.decision.picture);
        }
        out
    }

    fn hand_over(&mut self, fade_out: bool) -> Output {
        let Some(h) = self.hand.take() else {
            return Output::default();
        };
        self.chain_end = Some(h.end());
        Output {
            schedules: vec![h.tail(fade_out, self.time_of_sample(h.tail_start()))],
            discards: vec![],
        }
    }

    fn owe(&mut self, cause: JumpCause) {
        if self.jump_owed {
            return;
        }
        self.jump_owed = true;
        if self.hand.is_some() || self.chain_end.is_some() {
            self.second.jumps += 1;
            self.owed_cause = Some(cause);
        }
    }

    fn correction(&mut self, error: f64) -> i32 {
        if self.correcting != 0 && error.abs() < CORRECT_TO {
            self.correcting = 0;
        } else if error > CORRECT_FROM {
            self.correcting = 1;
        } else if error < -CORRECT_FROM {
            self.correcting = -1;
        } else if self.correcting != 0 && (error > 0.0) != (self.correcting > 0) {
            self.correcting = 0;
        }
        self.correcting
    }

    fn time_of_sample(&self, sample: f64) -> f64 {
        match self.mapping {
            Some((s, t)) => t + (sample - s) / RATE,
            None => f64::NEG_INFINITY,
        }
    }

    fn sample_at_time(&self, time: f64) -> f64 {
        match self.mapping {
            Some((s, t)) => (s + (time - t) * RATE).round(),
            None => 0.0,
        }
    }

    // ---- the windows

    fn record(&mut self, raw: f64, arrival: f64) {
        if self.session_start.is_none() {
            self.session_start = Some(arrival);
        }
        let now = arrival.floor() as i64;
        self.buckets.retain(|b| b.second > now - FLOOR_SECONDS);
        if let Some(floor) = self.floor() {
            if raw < floor - STEP_BELOW {
                let b = Bucket {
                    second: now,
                    min: raw,
                    max: self.cover_value(raw, arrival),
                    count: 1,
                };
                self.restart(b);
                return;
            }
            if raw - floor > STEP_ABOVE.max(2.0 * self.rtt_median.unwrap_or(0.0)) {
                let mut run = self.above.unwrap_or(Run {
                    since: arrival,
                    count: 0,
                    min: raw,
                    max: raw,
                });
                run.count += 1;
                run.min = run.min.min(raw);
                run.max = run.max.max(raw);
                self.above = Some(run);
                if arrival - run.since >= 1.0 && run.count >= STEP_PACKETS {
                    let b = Bucket {
                        second: now,
                        min: run.min,
                        max: self.cover_value(run.max, arrival),
                        count: run.count,
                    };
                    self.restart(b);
                    return;
                }
            } else {
                self.above = None;
            }
        }
        if self.buckets.last().map(|b| b.second) != Some(now) {
            self.buckets.push(Bucket {
                second: now,
                min: raw,
                max: None,
                count: 0,
            });
        }
        let cv = self.cover_value(raw, arrival);
        let i = self.buckets.len() - 1;
        self.buckets[i].min = self.buckets[i].min.min(raw);
        self.buckets[i].count += 1;
        if let Some(m) = cv {
            self.buckets[i].max = Some(self.buckets[i].max.map_or(m, |x| x.max(m)));
        }
        self.floor_kept = self.floor();
    }

    fn cover_value(&self, raw: f64, arrival: f64) -> Option<f64> {
        let begun = self.session_start?;
        if arrival - begun >= COVER_GRACE {
            Some(raw)
        } else {
            None
        }
    }

    fn restart(&mut self, bucket: Bucket) {
        self.buckets = vec![bucket];
        self.floor_kept = Some(bucket.min);
        self.above = None;
        self.frame_arrivals.clear();
        self.frame_raws.clear();
        self.frame_head = 0;
        self.sorted_raws.clear();
        self.last_picture_raw = None;
        self.owe(JumpCause::Step);
    }

    fn remember(&mut self, key: i64) {
        if self.recent_keys.len() < DEDUPE_COUNT {
            self.recent_keys.push(key);
        } else {
            self.recent_set.remove(&self.recent_keys[self.recent_next]);
            self.recent_keys[self.recent_next] = key;
            self.recent_next = (self.recent_next + 1) % DEDUPE_COUNT;
        }
        self.recent_set.insert(key);
    }

    fn expire_frames(&mut self, now: f64) {
        let from = now - PICTURE_SECONDS;
        while self.frame_head < self.frame_arrivals.len()
            && self.frame_arrivals[self.frame_head] < from
        {
            let raw = self.frame_raws[self.frame_head];
            if self.sorted_raws.len() == 1 {
                self.last_picture_raw = Some(self.sorted_raws[0]);
            }
            let i = insertion_index(&self.sorted_raws, raw);
            if i < self.sorted_raws.len() && self.sorted_raws[i] == raw {
                self.sorted_raws.remove(i);
            } else if i > 0 && self.sorted_raws[i - 1] == raw {
                self.sorted_raws.remove(i - 1);
            }
            self.frame_head += 1;
        }
        if self.frame_head > 512 {
            self.frame_arrivals.drain(..self.frame_head);
            self.frame_raws.drain(..self.frame_head);
            self.frame_head = 0;
        }
        if !self.sorted_raws.is_empty() {
            self.last_picture_raw = Some(median(&self.sorted_raws));
        }
    }

    fn live_frames_after_expiry(&self, now: f64) -> usize {
        let from = now - PICTURE_SECONDS;
        self.frame_arrivals[self.frame_head..]
            .iter()
            .rev()
            .take_while(|a| **a >= from)
            .count()
    }
}

// ---- the samples

/// A raised-cosine fade over the first (`fade_in`) or last `frames` frames of `x`, in place.
pub fn fade(x: &mut [f32], fade_in: bool, frames: usize) {
    let n = frames.min(x.len());
    if n == 0 {
        return;
    }
    let len = x.len();
    for i in 0..n {
        let g = (0.5 - 0.5 * (PI * (i + 1) as f64 / (n + 1) as f64).cos()) as f32;
        if fade_in {
            x[i] *= g;
        } else {
            x[len - 1 - i] *= g;
        }
    }
}

/// One frame added (+1) or dropped (-1), both channels alike, at the packet's quietest point in its
/// middle half, blended into its neighbours. Nothing for 0, or for a packet under 4 frames.
pub fn change(left: &mut Vec<f32>, right: &mut Vec<f32>, by: i32) {
    let n = left.len().min(right.len());
    if by == 0 || n < 4 {
        return;
    }
    let from = n / 4;
    let to = (from + 1).max((3 * n) / 4);
    let mut quietest = from;
    let mut level = f32::INFINITY;
    for i in from..to.min(n - 1) {
        let l = left[i].abs() + right[i].abs() + left[i + 1].abs() + right[i + 1].abs();
        if l < level {
            level = l;
            quietest = i;
        }
    }
    let q = quietest;
    if by > 0 {
        let (a, b) = (
            (left[q] + left[q + 1]) / 2.0,
            (right[q] + right[q + 1]) / 2.0,
        );
        left.insert(q + 1, a);
        right.insert(q + 1, b);
    } else {
        left[q] = (left[q] + left[q + 1]) / 2.0;
        right[q] = (right[q] + right[q + 1]) / 2.0;
        left.remove(q + 1);
        right.remove(q + 1);
    }
}
