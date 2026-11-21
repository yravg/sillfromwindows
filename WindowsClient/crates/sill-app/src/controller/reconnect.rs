//! The reconnect after a loss (iOSClient/DiscoveryPolicy.swift `redialRow`, `reconnectLine`):
//! the first dial at once, then 5, 10, 20 and every 30 s after the last started, 12 dials at most.
use std::time::Duration;

/// Seconds after the last dial started, by dials that did not connect so far (1, 2, 3, 4 or more).
pub const RETRY: [u64; 4] = [5, 10, 20, 30];
pub const DIALS: u32 = 12;
pub const QUIET_AFTER: u32 = 3;

/// How long after the last dial started the next is due, after `failed` dials that did not connect;
/// `None` once `DIALS` have failed (stop), `Some(0)` for the first.
pub fn next_delay(failed: u32) -> Option<Duration> {
    if failed >= DIALS {
        return None;
    }
    if failed == 0 {
        return Some(Duration::ZERO);
    }
    Some(Duration::from_secs(
        RETRY[(failed as usize).min(RETRY.len()) - 1],
    ))
}

/// The status line while it dials (`failed` dials did not connect, one under way not counted).
pub fn line(mac: &str, failed: u32) -> String {
    if failed >= DIALS {
        return format!("Stopped trying to reach {mac}. Tap it to try again.");
    }
    if failed >= QUIET_AFTER {
        return format!(
            "{mac} isn’t answering. It may be asleep. Sill keeps trying, or tap it to try now."
        );
    }
    format!("Reconnecting to {mac}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schedule_is_at_once_then_5_10_20_then_30() {
        let secs: Vec<Option<u64>> = (0..14)
            .map(|f| next_delay(f).map(|d| d.as_secs()))
            .collect();
        assert_eq!(
            secs,
            vec![
                Some(0),
                Some(5),
                Some(10),
                Some(20),
                Some(30),
                Some(30),
                Some(30),
                Some(30),
                Some(30),
                Some(30),
                Some(30),
                Some(30),
                None,
                None
            ]
        );
    }

    #[test]
    fn twelve_dials_in_all() {
        let dials = (0..).take_while(|f| next_delay(*f).is_some()).count();
        assert_eq!(dials, 12);
    }

    #[test]
    fn the_words_follow_the_dials() {
        assert_eq!(line("Mac", 0), "Reconnecting to Mac…");
        assert_eq!(line("Mac", 2), "Reconnecting to Mac…");
        assert!(line("Mac", 3).contains("isn’t answering"));
        assert!(line("Mac", 11).contains("may be asleep"));
        assert_eq!(
            line("Mac", 12),
            "Stopped trying to reach Mac. Tap it to try again."
        );
    }
}
