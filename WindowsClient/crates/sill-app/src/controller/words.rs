//! The words: how a session ended (iOSClient/GoodbyePolicy.swift), the locked Mac's cover
//! (LockedMac), the link line, pairing failures. Pure. "PC" where iOS says the device's kind.
use sill_net::pair_client::{PairError, PairRefusal};
use sill_net::session::{CloseReason, DialError};
use sill_proto::compat::SillVersion;
use sill_proto::remote::Goodbye;
use sill_proto::settings::{LinkReport, QualityPreset};
use sill_proto::switcher::LockedWhy;

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub text: String,
    pub reconnect: bool,
    /// The reconnect may dial the saved Mac's remote addresses.
    pub remote_allowed: bool,
    /// The Mac's own notice (an update, or a reason this build does not know).
    pub notice: bool,
}

/// SafeText.label, enough for a status line: one line, no control or bidirectional characters,
/// at most 300 characters.
pub fn safe_text(text: &str) -> Option<String> {
    let cleaned = sill_net::pairing::safe_label(text, 300);
    let cleaned = cleaned.trim().to_string();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// `GoodbyePolicy.outcome`: `goodbye` None means the connection just ended.
pub fn outcome(goodbye: Option<&Goodbye>, mac: &str, saved: bool) -> Outcome {
    let o = |text: String, reconnect: bool, remote_allowed: bool, notice: bool| Outcome {
        text,
        reconnect,
        remote_allowed,
        notice,
    };
    let Some(g) = goodbye else {
        return o(
            if saved {
                format!("{mac} disconnected. Sill will reconnect when it can reach it.")
            } else {
                format!("{mac} disconnected. It will reconnect when the Mac is back.")
            },
            true,
            saved,
            false,
        );
    };
    match g.reason.as_str() {
        Goodbye::QUIT => o(
            format!("{mac} quit Sill. This PC reconnects when it’s back."),
            true,
            saved,
            false,
        ),
        Goodbye::REMOVED => o(
            format!("{mac} removed this PC. To use it again, pair it again."),
            false,
            false,
            false,
        ),
        Goodbye::REMOTE_OFF => o(
            format!("{mac} turned off Remote Access."),
            true,
            false,
            false,
        ),
        Goodbye::INTERNET_OFF => o(
            format!(
                "{mac} stopped accepting connections from the internet. Connect through your VPN."
            ),
            true,
            false,
            false,
        ),
        Goodbye::BUSY => o(
            format!("{mac} is already serving 8 devices."),
            true,
            saved,
            false,
        ),
        Goodbye::PAIRING_REQUIRED => o(
            format!("{mac} now asks devices to pair. Select it to pair this PC."),
            false,
            false,
            false,
        ),
        Goodbye::UPDATE => {
            let mut own = format!("Update Sill on this PC to keep using {mac}.");
            if let Some(v) = g.minimum_version.as_deref().and_then(SillVersion::parse) {
                let parts: Vec<String> = v.components().iter().map(|c| c.to_string()).collect();
                own.push_str(&format!(" It needs version {} or later.", parts.join(".")));
            }
            o(
                g.message.as_deref().and_then(safe_text).unwrap_or(own),
                g.reconnect == Some(true),
                saved,
                true,
            )
        }
        _ => {
            let reconnect = g.reconnect == Some(true);
            let own = if reconnect {
                format!("{mac} closed the connection. Sill will reconnect when it can.")
            } else {
                format!("{mac} closed the connection. Select it to try again.")
            };
            o(
                g.message.as_deref().and_then(safe_text).unwrap_or(own),
                reconnect,
                saved,
                true,
            )
        }
    }
}

/// The outcome for a session's end. A Mac that never said anything to a paired key refused it
/// (not paired, or removed): that does not reconnect.
pub fn outcome_for_close(
    reason: &CloseReason,
    goodbye: Option<&Goodbye>,
    mac: &str,
    saved: bool,
) -> Outcome {
    if let (CloseReason::Eof { served: false }, None) = (reason, goodbye) {
        return Outcome {
            text: format!("{mac} didn’t accept this PC. To use it again, pair it again."),
            reconnect: false,
            remote_allowed: false,
            notice: false,
        };
    }
    outcome(goodbye, mac, saved)
}

/// A dial that did not connect.
pub fn dial_failure(e: &DialError, mac: &str) -> String {
    match e {
        DialError::PinMismatch => {
            format!("{mac} has a new key since this PC paired. Pair it again.")
        }
        _ => format!("{mac} didn’t answer."),
    }
}

/// A pairing exchange that did not finish, in words for the pairing card.
pub fn pair_failure(e: &PairError, mac: &str) -> String {
    match e {
        PairError::Refused(PairRefusal::WrongCode {
            tries_left: Some(n),
        }) => {
            format!("That code didn’t match ({n} tries left). Check the code on {mac}.")
        }
        PairError::Refused(PairRefusal::WrongCode { tries_left: None }) => {
            format!("That code didn’t match. Check the code on {mac}.")
        }
        PairError::Refused(PairRefusal::Closed) => {
            format!(
                "{mac} has no pairing window open. Open Pair iPhone or iPad… in Sill on the Mac."
            )
        }
        PairError::Refused(PairRefusal::Expired) => {
            "That code has expired. Ask the Mac for a new one.".into()
        }
        PairError::Dial(DialError::Connect(_) | DialError::Resolve(_)) => {
            format!("Couldn’t reach {mac}. Check the address, and that Sill is open on the Mac.")
        }
        PairError::Dial(DialError::PinMismatch) => {
            format!("{mac} answered with another key than the one this PC saw. Nothing was paired.")
        }
        other => other.to_string(),
    }
}

/// What covers the stream (LockedMac.Shade, Switcher.swift:106): another user on the Mac, then
/// "Unlocking…" until its picture comes back. A locked screen of this session's own has no cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shade {
    Locked(LockedWhy),
    Unlocking,
}

/// How long "Unlocking…" waits for the picture at most.
pub const UNLOCKING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);

/// The cover after a window list that says `why` (None: not locked): another user's console covers;
/// a locked screen streams its lock screen and has no cover; after a cover, "Unlocking…" until the
/// next keyframe.
pub fn shade_after_list(was: Option<Shade>, why: Option<LockedWhy>) -> Option<Shade> {
    if why == Some(LockedWhy::OtherUser) {
        return Some(Shade::Locked(LockedWhy::OtherUser));
    }
    was.map(|_| Shade::Unlocking)
}

/// The cover after a frame: a keyframe ends "Unlocking…", nothing else does.
pub fn shade_after_frame(was: Option<Shade>, keyframe: bool) -> Option<Shade> {
    if was == Some(Shade::Unlocking) && keyframe {
        None
    } else {
        was
    }
}

fn name(mac: &str) -> &str {
    if mac.is_empty() {
        "The Mac"
    } else {
        mac
    }
}

/// The Apps drawer's and the Settings panel's line where the windows would be (`LockedMac.line`).
pub fn line(mac: &str, why: LockedWhy) -> String {
    match why {
        LockedWhy::ScreenLocked => format!("{} is locked. Unlock it to see its windows.", name(mac)),
        LockedWhy::OtherUser => "Another user started controlling this Mac. Log in to your account to reactivate screen sharing with Sill.".into(),
    }
}

/// The cover's title (`LockedMac.title`).
pub fn title(mac: &str, shade: Shade) -> String {
    match shade {
        Shade::Locked(LockedWhy::ScreenLocked) => format!("{} is locked", name(mac)),
        Shade::Locked(LockedWhy::OtherUser) => "Another user started controlling this Mac.".into(),
        Shade::Unlocking => "Unlocking…".into(),
    }
}

/// The cover's line under its title (`LockedMac.detail`); `paired`: this session's key is the Mac's.
pub fn detail(shade: Shade, paired: Option<bool>) -> Option<String> {
    match shade {
        Shade::Locked(LockedWhy::ScreenLocked) => Some(
            if paired == Some(true) {
                "To unlock it from here, type its password and press Return. Its windows come back once it is unlocked."
            } else {
                "Its windows come back once it is unlocked."
            }
            .into(),
        ),
        Shade::Locked(LockedWhy::OtherUser) => {
            Some("Log in to your account to reactivate screen sharing with Sill.".into())
        }
        Shade::Unlocking => None,
    }
}

/// The line over the stream while the link cannot carry the quality.
pub fn link_line(link: &LinkReport) -> Option<String> {
    link.is_behind().then(|| {
        format!(
            "The link can’t keep up with {}. Lower it in Settings.",
            QualityPreset::name_for_bitrate(link.bitrate)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(reason: &str) -> Goodbye {
        Goodbye {
            reason: reason.into(),
            message: None,
            minimum_version: None,
            reconnect: None,
        }
    }

    #[test]
    fn the_five_old_reasons_keep_their_words_and_reconnects() {
        let q = outcome(Some(&g("quit")), "Studio", true);
        assert_eq!(
            q.text,
            "Studio quit Sill. This PC reconnects when it’s back."
        );
        assert!(q.reconnect && q.remote_allowed && !q.notice);
        let r = outcome(Some(&g("removed")), "Studio", true);
        assert!(!r.reconnect && !r.remote_allowed);
        assert!(r.text.contains("pair it again"));
        let off = outcome(Some(&g("remoteOff")), "Studio", true);
        assert!(off.reconnect && !off.remote_allowed);
        assert!(outcome(Some(&g("internetOff")), "S", true)
            .text
            .contains("Connect through your VPN"));
        assert_eq!(
            outcome(Some(&g("busy")), "S", false).text,
            "S is already serving 8 devices."
        );
        assert!(!outcome(Some(&g("pairingRequired")), "S", true).reconnect);
    }

    #[test]
    fn no_goodbye_reconnects_with_the_saved_or_the_unsaved_words() {
        let a = outcome(None, "Studio", true);
        assert!(a.reconnect && a.remote_allowed);
        assert!(a
            .text
            .starts_with("Studio disconnected. Sill will reconnect"));
        let b = outcome(None, "Studio", false);
        assert!(b.reconnect && !b.remote_allowed);
    }

    #[test]
    fn update_and_unknown_reasons_are_notices_in_the_macs_words() {
        let mut u = g("update");
        u.message = Some(
            "Update Sill on your PC to keep using Studio. It needs version 1.2 or later.".into(),
        );
        let o = outcome(Some(&u), "Studio", true);
        assert!(o.notice && !o.reconnect);
        assert!(o.text.starts_with("Update Sill on your PC"));
        // no message: this build's words, with the floor
        let mut v = g("update");
        v.minimum_version = Some("1.2".into());
        assert_eq!(
            outcome(Some(&v), "Studio", true).text,
            "Update Sill on this PC to keep using Studio. It needs version 1.2 or later."
        );
        let mut x = g("mystery");
        x.reconnect = Some(true);
        let o = outcome(Some(&x), "Studio", true);
        assert!(o.notice && o.reconnect);
        assert!(outcome(Some(&g("mystery")), "Studio", true)
            .text
            .contains("Select it to try again"));
    }

    #[test]
    fn a_message_is_cleaned_to_one_line() {
        let mut u = g("mystery");
        u.message = Some("Line one\nline two\u{202e}".into());
        let t = outcome(Some(&u), "S", true).text;
        assert!(!t.contains('\n') && !t.contains('\u{202e}'));
    }

    #[test]
    fn a_paired_key_refused_in_silence_does_not_reconnect() {
        let o = outcome_for_close(&CloseReason::Eof { served: false }, None, "Studio", true);
        assert!(!o.reconnect);
        let o = outcome_for_close(&CloseReason::Eof { served: true }, None, "Studio", true);
        assert!(o.reconnect);
        let o = outcome_for_close(&CloseReason::Lost(6.0), None, "Studio", true);
        assert!(o.reconnect);
    }

    use LockedWhy::{OtherUser, ScreenLocked};

    #[test]
    fn why_a_list_says_locked() {
        use sill_proto::switcher::{StreamSource, WindowList};
        let list = |locked: Option<bool>, off: Option<bool>| WindowList {
            mac_name: "M".into(),
            windows: vec![],
            active: StreamSource::Desktop,
            launch_id: None,
            host_version: None,
            protocol: None,
            gestures: None,
            clipboard: None,
            key_repeat: None,
            locked,
            off_console: off,
        };
        assert_eq!(list(None, None).locked_why(), None);
        assert_eq!(list(Some(true), None).locked_why(), Some(ScreenLocked));
        assert_eq!(list(Some(true), Some(true)).locked_why(), Some(OtherUser));
        // offConsole without locked: not locked
        assert_eq!(list(None, Some(true)).locked_why(), None);
    }

    #[test]
    fn the_lines_and_titles() {
        assert_eq!(
            line("Noah’s MacBook Pro", ScreenLocked),
            "Noah’s MacBook Pro is locked. Unlock it to see its windows."
        );
        assert_eq!(
            line("Noah’s MacBook Pro", OtherUser),
            "Another user started controlling this Mac. Log in to your account to reactivate screen sharing with Sill."
        );
        assert_eq!(
            line("", ScreenLocked),
            "The Mac is locked. Unlock it to see its windows."
        );
        assert_eq!(
            title("Mac mini", Shade::Locked(ScreenLocked)),
            "Mac mini is locked"
        );
        assert_eq!(
            title("Mac mini", Shade::Locked(OtherUser)),
            "Another user started controlling this Mac."
        );
        assert_eq!(title("Mac mini", Shade::Unlocking), "Unlocking…");
    }

    #[test]
    fn the_details_password_only_for_a_paired_session() {
        let pw = "To unlock it from here, type its password and press Return. Its windows come back once it is unlocked.";
        let plain = "Its windows come back once it is unlocked.";
        let s = Shade::Locked(ScreenLocked);
        assert_eq!(detail(s, Some(true)).as_deref(), Some(pw));
        assert_eq!(detail(s, Some(false)).as_deref(), Some(plain));
        assert_eq!(detail(s, None).as_deref(), Some(plain));
        let other = "Log in to your account to reactivate screen sharing with Sill.";
        assert_eq!(
            detail(Shade::Locked(OtherUser), Some(true)).as_deref(),
            Some(other)
        );
        assert_eq!(
            detail(Shade::Locked(OtherUser), Some(false)).as_deref(),
            Some(other)
        );
        assert_eq!(detail(Shade::Unlocking, Some(true)), None);
    }

    #[test]
    fn the_cover_is_only_for_another_users_console() {
        let u = Some(Shade::Unlocking);
        let o = Some(Shade::Locked(OtherUser));
        assert_eq!(shade_after_list(None, None), None);
        // this session's own locked screen: no cover, the lock screen streams
        assert_eq!(shade_after_list(None, Some(ScreenLocked)), None);
        assert_eq!(shade_after_list(None, Some(OtherUser)), o);
        assert_eq!(shade_after_list(o, Some(OtherUser)), o);
        assert_eq!(shade_after_list(o, Some(ScreenLocked)), u);
        assert_eq!(shade_after_list(u, Some(ScreenLocked)), u);
        assert_eq!(shade_after_list(o, None), u);
        assert_eq!(shade_after_list(u, None), u);
    }

    #[test]
    fn a_keyframe_ends_unlocking_never_the_locked_cover() {
        let o = Some(Shade::Locked(OtherUser));
        assert_eq!(shade_after_frame(Some(Shade::Unlocking), true), None);
        assert_eq!(
            shade_after_frame(Some(Shade::Unlocking), false),
            Some(Shade::Unlocking)
        );
        assert_eq!(shade_after_frame(o, true), o);
        assert_eq!(shade_after_frame(None, true), None);
        assert_eq!(UNLOCKING_TIMEOUT.as_secs(), 4);
    }

    /// Lock-policy's random sessions: the cover while another user has the console, "Unlocking…"
    /// until a keyframe after it, none otherwise.
    #[test]
    fn random_sessions_of_lists_and_frames() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let mut broken = 0;
        for _ in 0..2000 {
            let mut shade = None;
            let (mut last_off, mut waiting) = (false, false);
            for _ in 0..30 {
                if next(2) == 0 {
                    let why = [None, Some(ScreenLocked), Some(OtherUser)][next(3) as usize];
                    shade = shade_after_list(shade, why);
                    let off = why == Some(OtherUser);
                    if off {
                        waiting = false;
                    } else if last_off || waiting {
                        waiting = true;
                    }
                    last_off = off;
                } else {
                    let key = next(3) == 0;
                    shade = shade_after_frame(shade, key);
                    if key && waiting {
                        waiting = false;
                    }
                }
                if last_off && shade != Some(Shade::Locked(OtherUser)) {
                    broken += 1;
                }
                if waiting && shade != Some(Shade::Unlocking) {
                    broken += 1;
                }
                if !last_off && !waiting && shade.is_some() {
                    broken += 1;
                }
            }
        }
        assert_eq!(broken, 0);
    }

    #[test]
    fn the_link_line_names_the_preset() {
        let behind = LinkReport {
            state: "behind".into(),
            withheld_per_second: 40,
            bitrate: 40_000_000,
            carried_kbps: None,
            suggested_bitrate: None,
            suggested_capture_scale: None,
        };
        assert_eq!(
            link_line(&behind).unwrap(),
            "The link can’t keep up with Pro. Lower it in Settings."
        );
        let mut stalled = behind.clone();
        stalled.state = "stalled".into();
        assert!(link_line(&stalled).is_none());
    }

    #[test]
    fn pairing_words() {
        let e = PairError::Refused(PairRefusal::WrongCode {
            tries_left: Some(3),
        });
        assert!(pair_failure(&e, "Studio").contains("3 tries left"));
        let e = PairError::Dial(DialError::Connect("x".into()));
        assert!(pair_failure(&e, "Studio").starts_with("Couldn’t reach Studio"));
        assert_eq!(
            dial_failure(&DialError::Connect("x".into()), "Studio"),
            "Studio didn’t answer."
        );
        assert!(dial_failure(&DialError::PinMismatch, "Studio").contains("new key"));
    }
}
