//! Feeding the decoder: a queue that holds two frames at most, and the rule for what happens when
//! it is full (latency beats quality: drop, never wait). A dropped delta breaks the chain of
//! pictures, so after one every delta is dropped until a keyframe has gone in, and the Mac is
//! asked for one (kind 44, at most once a second: the host's own limit).
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use sill_proto::framing::ParameterSets;

/// The least the queue holds: two frames.
pub const QUEUE_FRAMES: usize = 2;
/// The most it ever holds, however fast the decoder is.
pub const QUEUE_MAX: usize = 8;
/// What it holds before the decoder's speed is known.
pub const QUEUE_START: usize = 4;
/// How much backlog the queue may be: frames x decode time stay under this (milliseconds).
const BACKLOG_MS: f64 = 100.0;

/// How many frames the queue holds for a decoder that takes `decode_ms` a frame: a burst that
/// arrives at once (the network delivers in batches) is kept while the decoder will catch up inside
/// `BACKLOG_MS`, never fewer than two, never more than `QUEUE_MAX`. Unknown speed: `QUEUE_START`.
pub fn capacity(decode_ms: Option<f64>) -> usize {
    match decode_ms {
        Some(ms) if ms.is_finite() => {
            ((BACKLOG_MS / ms.max(1.0)) as usize).clamp(QUEUE_FRAMES, QUEUE_MAX)
        }
        _ => QUEUE_START,
    }
}
const REQUEST_EVERY: Duration = Duration::from_secs(1);
/// A keyframe that was asked for and did not come within this long is asked for again.
const STILL_WAITING: Duration = Duration::from_secs(2);

pub enum Job {
    Sets(ParameterSets),
    Frame {
        avcc: Vec<u8>,
        keyframe: bool,
    },
    /// A session ended: start from nothing.
    Reset,
    Stop,
}

#[derive(Default)]
struct State {
    q: VecDeque<Job>,
    frames: usize,
}

/// Frames are bounded, control jobs (parameter sets, reset, stop) are never dropped and keep
/// their order among the frames.
pub struct JobQueue {
    inner: Mutex<State>,
    cv: Condvar,
    cap: AtomicUsize,
}

impl Default for JobQueue {
    fn default() -> Self {
        JobQueue {
            inner: Default::default(),
            cv: Default::default(),
            cap: AtomicUsize::new(QUEUE_FRAMES),
        }
    }
}

impl JobQueue {
    pub fn has_room(&self) -> bool {
        self.inner
            .lock()
            .map(|s| s.frames < self.cap.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    pub fn set_capacity(&self, frames: usize) {
        self.cap
            .store(frames.clamp(QUEUE_FRAMES, QUEUE_MAX), Ordering::Relaxed);
    }

    /// False (nothing queued) when two frames already wait.
    pub fn push_frame(&self, avcc: Vec<u8>, keyframe: bool) -> bool {
        let Ok(mut s) = self.inner.lock() else {
            return false;
        };
        if s.frames >= self.cap.load(Ordering::Relaxed) {
            return false;
        }
        s.frames += 1;
        s.q.push_back(Job::Frame { avcc, keyframe });
        self.cv.notify_one();
        true
    }

    pub fn push_control(&self, job: Job) {
        if let Ok(mut s) = self.inner.lock() {
            s.q.push_back(job);
            self.cv.notify_one();
        }
    }

    /// Drops every waiting frame and queues a Reset.
    pub fn reset(&self) {
        if let Ok(mut s) = self.inner.lock() {
            s.q.retain(|j| !matches!(j, Job::Frame { .. }));
            s.frames = 0;
            s.q.push_back(Job::Reset);
            self.cv.notify_one();
        }
    }

    /// Blocks for the next job.
    pub fn pop(&self) -> Job {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(j) = s.q.pop_front() {
                if matches!(j, Job::Frame { .. }) {
                    s.frames -= 1;
                }
                return j;
            }
            s = self.cv.wait(s).unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// What to do with one arriving frame.
#[derive(Debug, PartialEq, Eq)]
pub struct FeedAction {
    pub send: bool,
    pub request_keyframe: bool,
}

pub struct FeedPolicy {
    need_key: bool,
    last_request: Option<Instant>,
    waiting_since: Option<Instant>,
    /// Some frame has arrived on this connection: a Mac that streams nothing is not asked for a
    /// keyframe, there is no picture to break.
    seen_frame: bool,
}

impl FeedPolicy {
    /// A new connection: the Mac sends a keyframe with its parameter sets when a device joins, so
    /// nothing is asked for yet.
    pub fn new(now: Instant) -> Self {
        FeedPolicy {
            need_key: true,
            last_request: Some(now),
            waiting_since: Some(now),
            seen_frame: false,
        }
    }

    /// The decoder failed, or new parameter sets arrived: the next keyframe is needed.
    pub fn need_keyframe(&mut self, now: Instant) {
        if !self.need_key {
            self.need_key = true;
            self.waiting_since = Some(now);
        }
    }

    pub fn frame(&mut self, keyframe: bool, room: bool, now: Instant) -> FeedAction {
        self.seen_frame = true;
        if keyframe && room {
            self.need_key = false;
            self.waiting_since = None;
            return FeedAction {
                send: true,
                request_keyframe: false,
            };
        }
        if !keyframe && !self.need_key && room {
            return FeedAction {
                send: true,
                request_keyframe: false,
            };
        }
        // dropped: a full queue, or a delta with no keyframe before it
        if !self.need_key {
            self.need_key = true;
            self.waiting_since = Some(now);
        }
        FeedAction {
            send: false,
            request_keyframe: self.due(now),
        }
    }

    /// A once-a-quarter-second look: a keyframe asked for and not come within 2 s is asked for again.
    pub fn tick(&mut self, now: Instant) -> bool {
        match self.waiting_since {
            Some(since)
                if self.need_key
                    && self.seen_frame
                    && now.duration_since(since) >= STILL_WAITING =>
            {
                self.due(now)
            }
            _ => false,
        }
    }

    fn due(&mut self, now: Instant) -> bool {
        if self
            .last_request
            .is_none_or(|t| now.duration_since(t) >= REQUEST_EVERY)
        {
            self.last_request = Some(now);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64, base: Instant) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn the_queue_holds_two_frames_and_never_loses_control_jobs() {
        let q = JobQueue::default();
        assert!(q.push_frame(vec![1], true));
        q.push_control(Job::Sets(ParameterSets {
            nal_unit_header_length: 4,
            sets: vec![],
        }));
        assert!(q.push_frame(vec![2], false));
        assert!(!q.has_room());
        assert!(!q.push_frame(vec![3], false));
        q.push_control(Job::Stop);
        assert!(matches!(q.pop(), Job::Frame { keyframe: true, .. }));
        assert!(q.has_room());
        assert!(matches!(q.pop(), Job::Sets(_)));
        assert!(matches!(
            q.pop(),
            Job::Frame {
                keyframe: false,
                ..
            }
        ));
        assert!(matches!(q.pop(), Job::Stop));
    }

    #[test]
    fn the_queue_grows_with_a_fast_decoder_and_never_below_two() {
        assert_eq!(capacity(None), 4);
        assert_eq!(capacity(Some(56.0)), 2);
        assert_eq!(capacity(Some(30.0)), 3);
        assert_eq!(capacity(Some(10.0)), 8);
        assert_eq!(capacity(Some(0.2)), 8);
        assert_eq!(capacity(Some(500.0)), 2);
        assert_eq!(capacity(Some(f64::NAN)), 4);
        let q = JobQueue::default();
        q.set_capacity(capacity(Some(25.0)));
        for _ in 0..4 {
            assert!(q.push_frame(vec![0], false));
        }
        assert!(!q.push_frame(vec![0], false));
        q.set_capacity(1000);
        for _ in 0..4 {
            assert!(q.push_frame(vec![0], false));
        }
        assert!(!q.has_room());
    }

    #[test]
    fn reset_drops_frames_and_keeps_order_of_what_follows() {
        let q = JobQueue::default();
        q.push_frame(vec![1], true);
        q.push_frame(vec![2], false);
        q.reset();
        assert!(q.has_room());
        assert!(matches!(q.pop(), Job::Reset));
    }

    #[test]
    fn deltas_flow_after_the_first_keyframe() {
        let t = Instant::now();
        let mut f = FeedPolicy::new(t);
        // a delta before any keyframe is dropped
        assert!(!f.frame(false, true, at(10, t)).send);
        assert!(f.frame(true, true, at(20, t)).send);
        assert!(!f.need_key);
        assert_eq!(
            f.frame(false, true, at(30, t)),
            FeedAction {
                send: true,
                request_keyframe: false
            }
        );
    }

    #[test]
    fn a_full_queue_drops_the_delta_and_asks_once_a_second() {
        let t = Instant::now();
        let mut f = FeedPolicy::new(t);
        f.frame(true, true, at(0, t));
        let a = f.frame(false, false, at(2000, t));
        assert_eq!(
            a,
            FeedAction {
                send: false,
                request_keyframe: true
            }
        );
        // room again, but the chain is broken: deltas stay dropped, no second ask inside a second
        let b = f.frame(false, true, at(2100, t));
        assert_eq!(
            b,
            FeedAction {
                send: false,
                request_keyframe: false
            }
        );
        let c = f.frame(false, true, at(3100, t));
        assert_eq!(
            c,
            FeedAction {
                send: false,
                request_keyframe: true
            }
        );
        // the keyframe heals it
        assert!(f.frame(true, true, at(3200, t)).send);
        assert!(f.frame(false, true, at(3216, t)).send);
    }

    #[test]
    fn a_keyframe_that_finds_no_room_is_dropped_and_asked_for_again() {
        let t = Instant::now();
        let mut f = FeedPolicy::new(t);
        let a = f.frame(true, false, at(1500, t));
        assert_eq!(
            a,
            FeedAction {
                send: false,
                request_keyframe: true
            }
        );
        assert!(f.need_key);
    }

    #[test]
    fn a_mac_that_streams_nothing_is_never_asked_for_a_keyframe() {
        let t = Instant::now();
        let mut f = FeedPolicy::new(t);
        for s in 1..20 {
            assert!(!f.tick(at(s * 1000, t)));
        }
    }

    #[test]
    fn a_decoder_error_needs_a_keyframe_and_the_wait_is_watched() {
        let t = Instant::now();
        let mut f = FeedPolicy::new(t);
        f.frame(true, true, at(0, t));
        f.need_keyframe(at(5000, t));
        assert!(!f.frame(false, true, at(5010, t)).send);
        assert!(!f.tick(at(6000, t)));
        assert!(f.tick(at(7100, t)));
        // and asks again only a second later
        assert!(!f.tick(at(7600, t)));
        assert!(f.tick(at(8200, t)));
    }
}
