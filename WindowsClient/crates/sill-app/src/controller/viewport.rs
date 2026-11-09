//! Kind 9 is sent only when it changes, and held to what a host takes (ViewportBounds).
use sill_proto::viewport::{Viewport, ViewportBounds};

/// The rate this client asks for (the Mac runs the highest any device wants, capped by its own
/// setting; 60 is what every window picture needs here).
pub const WANTED_FPS: i64 = 60;

#[derive(Default)]
pub struct ViewportTracker {
    sent: Option<Viewport>,
}

impl ViewportTracker {
    /// The viewport to send for these sizes, or None when it equals the last one sent or cannot be
    /// held within the host's bounds. `forget` first to force a send (a new connection).
    pub fn next(&mut self, width: f64, height: f64, scale: Option<f64>) -> Option<Viewport> {
        let wanted = Viewport {
            width,
            height,
            scale,
            local_cursor: None,
            fps: Some(WANTED_FPS),
        };
        let held = ViewportBounds::held(&wanted)?;
        if self.sent == Some(held) {
            return None;
        }
        self.sent = Some(held);
        Some(held)
    }

    /// A new connection knows nothing of what was sent before.
    pub fn forget(&mut self) {
        self.sent = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_viewport_is_sent_once() {
        let mut t = ViewportTracker::default();
        assert!(t.next(1280.0, 800.0, None).is_some());
        assert!(t.next(1280.0, 800.0, None).is_none());
        assert!(t.next(1280.0, 801.0, None).is_some());
        assert!(t.next(1280.0, 801.0, Some(1.25)).is_some());
        assert!(t.next(1280.0, 801.0, Some(1.25)).is_none());
    }

    #[test]
    fn a_new_connection_sends_again() {
        let mut t = ViewportTracker::default();
        t.next(100.0, 100.0, None);
        t.forget();
        assert!(t.next(100.0, 100.0, None).is_some());
    }

    #[test]
    fn what_the_host_would_refuse_is_held_in_bounds_or_not_sent() {
        let mut t = ViewportTracker::default();
        assert!(t.next(f64::NAN, 100.0, None).is_none());
        assert!(t.next(0.0, 100.0, None).is_none());
        let v = t.next(1e300, 100.0, Some(1e-300)).unwrap();
        assert_eq!(v.width, 16384.0);
        assert_eq!(v.scale, Some(0.25));
        assert!(ViewportBounds::refusal(&v).is_none());
        // two sizes that hold to the same bounds are one viewport
        assert!(t.next(2e300, 100.0, Some(1e-300)).is_none());
    }
}
