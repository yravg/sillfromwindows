//! The settings ledger, minimal (iOSClient/HostSettingsLedger.swift): the Mac's last kind 16 with
//! this PC's unanswered picks laid over it, so a control shows its pick at once. An answer (kind 16
//! whose `answering` is a pick's token) clears that pick only; a pick not answered within
//! max(4 s, 4 x the worst round trip) goes back with "‹Mac› didn’t answer. Try again."; an answer
//! that did not take the pick (the Mac refused it) goes back with "‹Mac› kept its setting.".
use std::time::{Duration, Instant};

use sill_proto::settings::{HostSettingsChange, HostSettingsState};

struct Pick {
    token: i64,
    change: HostSettingsChange,
    at: Instant,
}

#[derive(Default)]
pub struct Ledger {
    state: Option<HostSettingsState>,
    picks: Vec<Pick>,
    /// Strictly increasing for the process: never reset, so a token is never reused.
    next_token: i64,
    problem: Option<String>,
}

impl Ledger {
    pub fn new() -> Self {
        Ledger {
            next_token: 1,
            ..Default::default()
        }
    }

    /// Forget the Mac's state and every pick (a session ended); tokens go on counting.
    pub fn reset(&mut self) {
        self.state = None;
        self.picks.clear();
        self.problem = None;
    }

    pub fn has_state(&self) -> bool {
        self.state.is_some()
    }

    /// A control's pick. Returns the change to send (with its token), or None when nothing goes:
    /// no state from the Mac yet, nothing changed, or it would change nothing of what is shown.
    pub fn pick(
        &mut self,
        mut change: HostSettingsChange,
        now: Instant,
    ) -> Option<HostSettingsChange> {
        let shown = self.shown()?;
        change.token = None;
        if change.is_empty() || change.applied(&shown.settings) == shown.settings {
            return None;
        }
        self.problem = None;
        let token = self.next_token;
        self.next_token += 1;
        change.token = Some(token);
        self.picks.push(Pick {
            token,
            change: change.clone(),
            at: now,
        });
        Some(change)
    }

    /// The Mac's state arrived. `mac` names it in a refusal's words.
    pub fn on_state(&mut self, state: HostSettingsState, mac: &str) {
        if let Some(token) = state.answering {
            if let Some(i) = self.picks.iter().position(|p| p.token == token) {
                let pick = self.picks.remove(i);
                if pick.change.applied(&state.settings) != state.settings {
                    self.problem = Some(format!("{mac} kept its setting."));
                }
            }
        }
        self.state = Some(state);
    }

    /// Picks unanswered too long go back. True when something changed.
    pub fn expire(&mut self, now: Instant, worst_rtt_ms: f64, mac: &str) -> bool {
        let wait = timeout(worst_rtt_ms);
        let before = self.picks.len();
        self.picks.retain(|p| now.duration_since(p.at) < wait);
        if self.picks.len() != before {
            self.problem = Some(format!("{mac} didn’t answer. Try again."));
            true
        } else {
            false
        }
    }

    /// What the controls show: the Mac's state with the unanswered picks over it, in order.
    pub fn shown(&self) -> Option<HostSettingsState> {
        let mut s = self.state.clone()?;
        for p in &self.picks {
            s.settings = p.change.applied(&s.settings);
        }
        Some(s)
    }

    pub fn state(&self) -> Option<&HostSettingsState> {
        self.state.as_ref()
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }
}

/// How long a pick waits for its answer.
pub fn timeout(worst_rtt_ms: f64) -> Duration {
    let ms = (worst_rtt_ms.max(0.0) * 4.0).max(4000.0);
    Duration::from_millis(ms as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sill_proto::settings::StreamSettings;

    fn state(bitrate: i64, answering: Option<i64>) -> HostSettingsState {
        HostSettingsState {
            settings: StreamSettings {
                max_fps: 60,
                bitrate,
                capture_scale: 2.0,
                prioritize_speed: false,
                virtual_display: false,
                direct_wireless: None,
                send_audio: Some(false),
            },
            persistent: true,
            virtual_display_available: true,
            virtual_display_note: None,
            software_encoder: false,
            stream: None,
            answering,
            away: None,
            link: None,
            audio_note: None,
            session_paired: None,
        }
    }

    fn bitrate(b: i64) -> HostSettingsChange {
        HostSettingsChange {
            bitrate: Some(b),
            ..Default::default()
        }
    }

    #[test]
    fn nothing_is_sent_before_the_mac_has_said_its_state() {
        let mut l = Ledger::new();
        assert!(l.pick(bitrate(8_000_000), Instant::now()).is_none());
    }

    #[test]
    fn a_pick_shows_at_once_and_an_answer_clears_only_its_token() {
        let mut l = Ledger::new();
        let t0 = Instant::now();
        l.on_state(state(15_000_000, None), "Studio");
        let a = l.pick(bitrate(40_000_000), t0).unwrap();
        let b = l.pick(bitrate(25_000_000), t0).unwrap();
        assert_eq!((a.token, b.token), (Some(1), Some(2)));
        assert_eq!(l.shown().unwrap().settings.bitrate, 25_000_000);
        // the answer to the first: the Mac took 40; the second is still over it
        l.on_state(state(40_000_000, Some(1)), "Studio");
        assert_eq!(l.shown().unwrap().settings.bitrate, 25_000_000);
        l.on_state(state(25_000_000, Some(2)), "Studio");
        assert_eq!(l.shown().unwrap().settings.bitrate, 25_000_000);
        assert!(l.problem().is_none());
    }

    #[test]
    fn sixty_one_twenty_sixty_never_shows_the_old_answer() {
        let mut l = Ledger::new();
        let t0 = Instant::now();
        l.on_state(state(15_000_000, None), "M");
        let fps = |n| HostSettingsChange {
            max_fps: Some(n),
            ..Default::default()
        };
        l.pick(fps(120), t0).unwrap();
        l.pick(fps(60), t0).unwrap();
        let mut s = state(15_000_000, Some(1));
        s.settings.max_fps = 120;
        l.on_state(s, "M");
        assert_eq!(l.shown().unwrap().settings.max_fps, 60);
    }

    #[test]
    fn a_state_that_is_not_an_answer_clears_no_pick() {
        let mut l = Ledger::new();
        l.on_state(state(15_000_000, None), "M");
        l.pick(bitrate(40_000_000), Instant::now()).unwrap();
        l.on_state(state(15_000_000, None), "M");
        assert_eq!(l.shown().unwrap().settings.bitrate, 40_000_000);
    }

    #[test]
    fn a_refused_pick_goes_back_with_words() {
        let mut l = Ledger::new();
        l.on_state(state(15_000_000, None), "Studio");
        l.pick(bitrate(80_000_000), Instant::now()).unwrap();
        l.on_state(state(15_000_000, Some(1)), "Studio");
        assert_eq!(l.shown().unwrap().settings.bitrate, 15_000_000);
        assert_eq!(l.problem(), Some("Studio kept its setting."));
    }

    #[test]
    fn a_pick_that_changes_nothing_shown_is_not_sent() {
        let mut l = Ledger::new();
        l.on_state(state(15_000_000, None), "M");
        assert!(l.pick(bitrate(15_000_000), Instant::now()).is_none());
        assert!(l
            .pick(HostSettingsChange::default(), Instant::now())
            .is_none());
    }

    #[test]
    fn an_unanswered_pick_expires_after_max_4s_or_4_rtt() {
        assert_eq!(timeout(100.0), Duration::from_secs(4));
        assert_eq!(timeout(2000.0), Duration::from_secs(8));
        let mut l = Ledger::new();
        let t0 = Instant::now();
        l.on_state(state(15_000_000, None), "Studio");
        l.pick(bitrate(40_000_000), t0).unwrap();
        assert!(!l.expire(t0 + Duration::from_millis(3900), 100.0, "Studio"));
        assert!(l.expire(t0 + Duration::from_millis(4100), 100.0, "Studio"));
        assert_eq!(l.shown().unwrap().settings.bitrate, 15_000_000);
        assert_eq!(l.problem(), Some("Studio didn’t answer. Try again."));
        // a new pick clears the words
        l.pick(bitrate(25_000_000), t0).unwrap();
        assert!(l.problem().is_none());
    }

    #[test]
    fn tokens_keep_counting_across_a_reset() {
        let mut l = Ledger::new();
        l.on_state(state(15_000_000, None), "M");
        assert_eq!(
            l.pick(bitrate(40_000_000), Instant::now()).unwrap().token,
            Some(1)
        );
        l.reset();
        l.on_state(state(15_000_000, None), "M");
        assert_eq!(
            l.pick(bitrate(40_000_000), Instant::now()).unwrap().token,
            Some(2)
        );
    }
}
