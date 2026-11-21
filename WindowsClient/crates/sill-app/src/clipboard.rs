//! OWNER: worker C1. The clipboard both ways, this PC's side (iOSClient/StreamClient+Clipboard.swift,
//! Sources/StreamProtocol/ClipboardPolicy.swift's device rules). Adapted to Windows: there is no
//! Paste chip (the PC's clipboard is readable), so this PC's new text goes by itself.
//!
//! * Subscribe (kind 32 `{}`) while the Mac's list says `clipboard` >= 1 and the switch is on; `stop`
//!   when the switch goes off. A Mac that stops saying it forgot every subscription.
//! * The Mac's text is written to this PC's clipboard, never while the switch is off.
//! * This PC's clipboard is read when the Sill window becomes active and every 0.5 s while it is
//!   active; text that is new (not what Sill just wrote, not what it just sent, not what was there
//!   when sharing began) goes to the Mac: at most 256 KB, at most 4 a second.
//! * Never echoed: the text Sill wrote or sent is remembered until the clipboard holds something else.
//!
//! The UI tells this module when its window has focus: `clipboard::set_window_active(bool)`
//! (until it does, the window counts as active).
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sill_proto::small::Clipboard;
use sill_proto::wire;

/// The most a text may be (Clipboard.maxTextBytes).
pub const MAX_TEXT_BYTES: usize = Clipboard::MAX_TEXT_BYTES;
/// A device's subscriptions and texts a second (ClipboardPolicy.perSecond).
pub const PER_SECOND: usize = 4;
/// How often this PC's clipboard is read while the window is active.
pub const LOOK_EVERY: Duration = Duration::from_millis(500);

static WINDOW_ACTIVE: AtomicBool = AtomicBool::new(true);

/// The UI says whether the Sill window has focus (the UI's call; until it is made the window
/// counts as active).
#[allow(dead_code)]
pub fn set_window_active(active: bool) {
    WINDOW_ACTIVE.store(active, Ordering::Relaxed);
}

fn window_active() -> bool {
    WINDOW_ACTIVE.load(Ordering::Relaxed)
}

// --- pure rules ---------------------------------------------------------------------------------

/// `hostTakes`.
pub fn host_takes(host_clipboard: Option<i64>) -> bool {
    host_clipboard.unwrap_or(0) >= 1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subscription {
    Subscribe,
    Stop,
    Nothing,
}

/// `deviceSubscription`: what to send, and whether the connection is subscribed after it.
pub fn subscription(
    host_clipboard: Option<i64>,
    switch_on: bool,
    subscribed: bool,
) -> (Subscription, bool) {
    if !host_takes(host_clipboard) {
        return (Subscription::Nothing, false);
    }
    match (switch_on, subscribed) {
        (true, false) => (Subscription::Subscribe, true),
        (false, true) => (Subscription::Stop, false),
        _ => (Subscription::Nothing, subscribed),
    }
}

/// `payload(text:)`: the JSON of a kind 32 that sets the Mac's clipboard, or None for empty text or
/// text over the cap.
pub fn payload(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || text.len() > MAX_TEXT_BYTES {
        return None;
    }
    let data = wire::encode(&Clipboard {
        text: Some(text.to_string()),
        ..Default::default()
    });
    (!data.is_empty() && data.len() <= 1 << 20).then_some(data)
}

pub fn subscription_payload() -> Vec<u8> {
    wire::encode(&Clipboard::default())
}

pub fn stop_payload() -> Vec<u8> {
    wire::encode(&Clipboard {
        stop: Some(true),
        ..Default::default()
    })
}

/// `admits`: whether one more subscription or text is taken now, and the arrivals to keep.
pub fn admits(arrivals: &[Instant], now: Instant) -> (bool, Vec<Instant>) {
    let mut recent: Vec<Instant> = arrivals
        .iter()
        .copied()
        .filter(|t| now.duration_since(*t) < Duration::from_secs(1))
        .collect();
    if recent.len() >= PER_SECOND {
        return (false, recent);
    }
    recent.push(now);
    (true, recent)
}

/// `deviceWrites`: the text a kind 32 from the Mac makes this PC write, or None: the switch off, a
/// Mac that does not take the clipboard, no text, an empty or over-long one, or what this PC just
/// sent (the Mac's echo, which a host of this generation never sends).
pub fn writes(
    clip: &Clipboard,
    switch_on: bool,
    host_clipboard: Option<i64>,
    last: Option<&str>,
) -> Option<String> {
    if !switch_on || !host_takes(host_clipboard) {
        return None;
    }
    let text = clip
        .text
        .as_deref()
        .filter(|t| !t.is_empty() && t.len() <= MAX_TEXT_BYTES)?;
    (last != Some(text)).then(|| text.to_string())
}

/// What this PC should send after looking at its clipboard: the text is new when it is not empty and
/// not the one Sill last wrote, sent or found when sharing began.
pub fn is_new(text: &str, seen: Option<&str>) -> bool {
    !text.is_empty() && seen != Some(text)
}

// --- the PC's clipboard -------------------------------------------------------------------------

pub trait PcClipboard: Send {
    fn text(&mut self) -> Option<String>;
    fn set_text(&mut self, text: &str) -> bool;
}

/// arboard, created on first use (a PC with no clipboard service, or one that is busy, reads as
/// empty and writes nothing).
#[derive(Default)]
pub struct Arboard {
    inner: Option<arboard::Clipboard>,
}

impl Arboard {
    fn get(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.inner.is_none() {
            self.inner = arboard::Clipboard::new().ok();
        }
        self.inner.as_mut()
    }
}

impl PcClipboard for Arboard {
    fn text(&mut self) -> Option<String> {
        self.get()?.get_text().ok()
    }
    fn set_text(&mut self, text: &str) -> bool {
        self.get()
            .is_some_and(|c| c.set_text(text.to_string()).is_ok())
    }
}

/// One connection's clipboard state.
pub struct Sync {
    backend: Box<dyn PcClipboard>,
    subscribed: bool,
    arrivals: Vec<Instant>,
    /// The text on this PC's clipboard that needs no sending: written from the Mac, sent, or found
    /// when sharing began.
    seen: Option<String>,
    last_look: Option<Instant>,
    was_active: bool,
}

impl Default for Sync {
    fn default() -> Self {
        Sync::new(Box::<Arboard>::default())
    }
}

impl Sync {
    pub fn new(backend: Box<dyn PcClipboard>) -> Self {
        Sync {
            backend,
            subscribed: false,
            arrivals: Vec::new(),
            seen: None,
            last_look: None,
            was_active: false,
        }
    }

    /// A session ended: the Mac forgot its subscriptions.
    pub fn reset(&mut self) {
        self.subscribed = false;
        self.arrivals.clear();
        self.seen = None;
        self.last_look = None;
    }

    #[cfg(test)]
    pub fn subscribed(&self) -> bool {
        self.subscribed
    }

    /// The Mac's list, or the switch, changed: the kind 32 to send, if any. A new subscription takes
    /// what is on the clipboard now as already seen: old text is never sent by itself.
    pub fn sync_subscription(
        &mut self,
        host_clipboard: Option<i64>,
        switch_on: bool,
        now: Instant,
    ) -> Option<Vec<u8>> {
        let (send, subscribed) = subscription(host_clipboard, switch_on, self.subscribed);
        self.subscribed = subscribed;
        match send {
            Subscription::Nothing => None,
            Subscription::Stop => Some(stop_payload()),
            Subscription::Subscribe => {
                let (taken, kept) = admits(&self.arrivals, now);
                self.arrivals = kept;
                if !taken {
                    self.subscribed = false; // asked again at the next look
                    return None;
                }
                self.seen = self.backend.text();
                self.last_look = Some(now);
                Some(subscription_payload())
            }
        }
    }

    /// A kind 32 from the Mac: writes its text to this PC's clipboard.
    pub fn write_from_mac(
        &mut self,
        clip: &Clipboard,
        switch_on: bool,
        host_clipboard: Option<i64>,
    ) -> bool {
        let Some(text) = writes(clip, switch_on, host_clipboard, self.seen.as_deref()) else {
            return false;
        };
        if self.backend.set_text(&text) {
            self.seen = Some(text);
            true
        } else {
            false
        }
    }

    /// Text for the Mac from a command (`Command::SendClipboard`): the kind 32 payload when it goes.
    pub fn send_text(
        &mut self,
        text: &str,
        switch_on: bool,
        host_clipboard: Option<i64>,
        now: Instant,
    ) -> Option<Vec<u8>> {
        if !switch_on
            || !self.subscribed
            || !host_takes(host_clipboard)
            || self.seen.as_deref() == Some(text)
        {
            return None;
        }
        let data = payload(text)?;
        let (taken, kept) = admits(&self.arrivals, now);
        self.arrivals = kept;
        if !taken {
            return None;
        }
        self.seen = Some(text.to_string());
        Some(data)
    }

    /// A look at this PC's clipboard, when the window has just become active or 0.5 s have passed
    /// while it is active: the payload when the text is new.
    pub fn look(
        &mut self,
        switch_on: bool,
        host_clipboard: Option<i64>,
        now: Instant,
    ) -> Option<Vec<u8>> {
        self.look_as(window_active(), switch_on, host_clipboard, now)
    }

    fn look_as(
        &mut self,
        active: bool,
        switch_on: bool,
        host_clipboard: Option<i64>,
        now: Instant,
    ) -> Option<Vec<u8>> {
        let just_activated = active && !self.was_active;
        self.was_active = active;
        if !active || !self.subscribed || !switch_on || !host_takes(host_clipboard) {
            return None;
        }
        let due = self
            .last_look
            .is_none_or(|t| now.duration_since(t) >= LOOK_EVERY);
        if !(just_activated || due) {
            return None;
        }
        self.last_look = Some(now);
        let text = self.backend.text()?;
        if !is_new(&text, self.seen.as_deref()) {
            return None;
        }
        self.send_text(&text, switch_on, host_clipboard, now)
            .or_else(|| {
                // too long or past the rate: not sent. Over the cap it is never retried (seen); past
                // the rate it is looked at again.
                if text.len() > MAX_TEXT_BYTES {
                    self.seen = Some(text);
                }
                None
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<Option<String>>>);
    impl PcClipboard for Fake {
        fn text(&mut self) -> Option<String> {
            self.0.lock().unwrap().clone()
        }
        fn set_text(&mut self, t: &str) -> bool {
            *self.0.lock().unwrap() = Some(t.into());
            true
        }
    }

    fn sync_with(initial: Option<&str>) -> (Sync, Fake) {
        let f = Fake::default();
        *f.0.lock().unwrap() = initial.map(Into::into);
        (Sync::new(Box::new(f.clone())), f)
    }

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn text_of(p: &[u8]) -> String {
        wire::decode::<Clipboard>(p).unwrap().text.unwrap()
    }

    #[test]
    fn subscription_rules() {
        use Subscription::*;
        assert_eq!(subscription(Some(1), true, false), (Subscribe, true));
        assert_eq!(subscription(Some(1), true, true), (Nothing, true));
        assert_eq!(subscription(Some(1), false, true), (Stop, false));
        assert_eq!(subscription(Some(1), false, false), (Nothing, false));
        // a Mac that stops saying it forgot every subscription
        assert_eq!(subscription(None, true, true), (Nothing, false));
        assert_eq!(subscription(Some(0), true, false), (Nothing, false));
    }

    #[test]
    fn the_wire_bytes() {
        assert_eq!(subscription_payload(), b"{}");
        assert_eq!(stop_payload(), br#"{"stop":true}"#);
        assert_eq!(text_of(&payload("héllo").unwrap()), "héllo");
        assert!(payload("").is_none());
        assert!(payload(&"x".repeat(MAX_TEXT_BYTES)).is_some());
        assert!(payload(&"x".repeat(MAX_TEXT_BYTES + 1)).is_none());
    }

    #[test]
    fn four_a_second() {
        let t = Instant::now();
        let mut kept = Vec::new();
        for i in 0..4 {
            let (ok, k) = admits(&kept, at(t, i * 10));
            assert!(ok);
            kept = k;
        }
        assert!(!admits(&kept, at(t, 500)).0);
        assert!(admits(&kept, at(t, 1100)).0);
    }

    #[test]
    fn what_the_mac_writes_and_what_it_never_does() {
        let c = |t: &str| Clipboard {
            text: Some(t.into()),
            ..Default::default()
        };
        assert_eq!(writes(&c("hi"), true, Some(1), None).as_deref(), Some("hi"));
        assert_eq!(writes(&c("hi"), false, Some(1), None), None);
        assert_eq!(writes(&c("hi"), true, None, None), None);
        assert_eq!(writes(&c(""), true, Some(1), None), None);
        assert_eq!(writes(&c("hi"), true, Some(1), Some("hi")), None);
        assert_eq!(
            writes(&c(&"x".repeat(MAX_TEXT_BYTES + 1)), true, Some(1), None),
            None
        );
        assert_eq!(writes(&Clipboard::default(), true, Some(1), None), None);
    }

    #[test]
    fn new_text_goes_but_what_was_there_when_sharing_began_does_not() {
        let t = Instant::now();
        let (mut s, pc) = sync_with(Some("old"));
        assert_eq!(s.sync_subscription(Some(1), true, t).unwrap(), b"{}");
        assert!(s.subscribed());
        assert!(s.look_as(true, true, Some(1), at(t, 600)).is_none());
        *pc.0.lock().unwrap() = Some("copied".into());
        // not yet half a second since the last look: waits
        assert!(s.look_as(true, true, Some(1), at(t, 700)).is_none());
        let p = s.look_as(true, true, Some(1), at(t, 1200)).unwrap();
        assert_eq!(text_of(&p), "copied");
        // the same text is not sent twice
        assert!(s.look_as(true, true, Some(1), at(t, 1800)).is_none());
    }

    #[test]
    fn never_echoes_what_the_mac_wrote() {
        let t = Instant::now();
        let (mut s, _pc) = sync_with(None);
        s.sync_subscription(Some(1), true, t);
        let c = Clipboard {
            text: Some("from the Mac".into()),
            ..Default::default()
        };
        assert!(s.write_from_mac(&c, true, Some(1)));
        assert!(s.look_as(true, true, Some(1), at(t, 600)).is_none());
        // and the Mac echoing what this PC sent writes nothing
        assert!(s.send_text("mine", true, Some(1), at(t, 700)).is_some());
        assert!(!s.write_from_mac(
            &Clipboard {
                text: Some("mine".into()),
                ..Default::default()
            },
            true,
            Some(1)
        ));
    }

    #[test]
    fn it_looks_only_while_the_window_is_active_and_at_once_when_it_becomes_active() {
        let t = Instant::now();
        let (mut s, pc) = sync_with(None);
        s.sync_subscription(Some(1), true, t);
        assert!(s.look_as(true, true, Some(1), at(t, 600)).is_none()); // active: nothing new
        *pc.0.lock().unwrap() = Some("while away".into());
        assert!(s.look_as(false, true, Some(1), at(t, 1300)).is_none()); // inactive: not read
        assert!(s.look_as(false, true, Some(1), at(t, 2000)).is_none());
        // activation looks at once, though the last look was 0.1 s ago
        let p = s.look_as(true, true, Some(1), at(t, 2100)).unwrap();
        assert_eq!(text_of(&p), "while away");
    }

    #[test]
    fn nothing_without_the_switch_the_subscription_or_a_mac_that_takes_it() {
        let t = Instant::now();
        let (mut s, pc) = sync_with(None);
        *pc.0.lock().unwrap() = Some("x".into());
        assert!(s.look_as(true, true, Some(1), t).is_none()); // not subscribed
        s.sync_subscription(Some(1), true, t);
        *pc.0.lock().unwrap() = Some("y".into());
        assert!(s.look_as(true, false, Some(1), at(t, 600)).is_none());
        assert!(s.look_as(true, true, None, at(t, 1200)).is_none());
        assert_eq!(
            s.sync_subscription(Some(1), false, at(t, 1300)).unwrap(),
            br#"{"stop":true}"#
        );
        assert!(!s.subscribed());
    }

    #[test]
    fn too_long_is_never_sent_and_not_retried() {
        let t = Instant::now();
        let (mut s, pc) = sync_with(None);
        s.sync_subscription(Some(1), true, t);
        *pc.0.lock().unwrap() = Some("x".repeat(MAX_TEXT_BYTES + 1));
        assert!(s.look_as(true, true, Some(1), at(t, 600)).is_none());
        assert!(s.look_as(true, true, Some(1), at(t, 1200)).is_none());
    }

    #[test]
    fn a_burst_of_copies_is_held_to_four_a_second() {
        let t = Instant::now();
        let (mut s, pc) = sync_with(None);
        s.sync_subscription(Some(1), true, t); // one arrival for the subscription
        let mut sent = 0;
        for i in 0..10u64 {
            *pc.0.lock().unwrap() = Some(format!("c{i}"));
            if s.look_as(true, true, Some(1), at(t, 600 + i * 50 + i * 500 / 10))
                .is_some()
            {
                sent += 1;
            }
        }
        assert!(sent < PER_SECOND, "{sent}");
    }

    #[test]
    fn a_session_end_forgets_the_subscription() {
        let t = Instant::now();
        let (mut s, _pc) = sync_with(None);
        s.sync_subscription(Some(1), true, t);
        s.reset();
        assert!(!s.subscribed());
        assert!(s.sync_subscription(Some(1), true, at(t, 3000)).is_some());
    }
}
