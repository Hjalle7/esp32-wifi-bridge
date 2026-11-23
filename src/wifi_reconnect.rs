//! Station event state shared with the main task and its retry schedule.

use core::{
    sync::atomic::{AtomicU8, Ordering::Relaxed},
    time::Duration,
};

const RETRY_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StationState {
    Disconnected,
    Connecting,
    Connected,
    Unavailable,
}

pub struct StationLink(AtomicU8);

impl StationLink {
    pub const fn new() -> Self {
        Self(AtomicU8::new(StationState::Disconnected as u8))
    }

    pub fn state(&self) -> StationState {
        match self.0.load(Relaxed) {
            n if n == StationState::Disconnected as u8 => StationState::Disconnected,
            n if n == StationState::Connecting as u8 => StationState::Connecting,
            n if n == StationState::Connected as u8 => StationState::Connected,
            _ => StationState::Unavailable,
        }
    }

    pub fn set_associated(&self, associated: bool) {
        let state = if associated {
            StationState::Connected
        } else {
            StationState::Disconnected
        };
        // Only this byte is shared. Association events cannot undo a stop or a
        // fatal connect error reported by the owner.
        let _ = self.0.try_update(Relaxed, Relaxed, |previous| {
            (previous != StationState::Unavailable as u8).then_some(state as u8)
        });
    }

    pub fn disable(&self) {
        self.0.store(StationState::Unavailable as u8, Relaxed);
    }

    pub fn connect_rejected(&self) {
        // Keep any association outcome already delivered by the event loop.
        let _ = self.0.compare_exchange(
            StationState::Connecting as u8,
            StationState::Disconnected as u8,
            Relaxed,
            Relaxed,
        );
    }
}

pub struct WifiReconnect {
    next_attempt: Duration,
}

impl WifiReconnect {
    pub const fn new() -> Self {
        Self {
            next_attempt: Duration::ZERO,
        }
    }

    /// Called only by the owner, after the station and Ethernet have started.
    pub fn begin_attempt(&self, link: &StationLink, now: Duration) -> bool {
        now >= self.next_attempt
            && link
                .0
                .compare_exchange(
                    StationState::Disconnected as u8,
                    StationState::Connecting as u8,
                    Relaxed,
                    Relaxed,
                )
                .is_ok()
    }

    /// Set the retry deadline from the time the call returned, even on error.
    pub fn attempt_returned(&mut self, now: Duration) {
        self.next_attempt = now + RETRY_INTERVAL;
    }
}

#[cfg(all(test, not(target_os = "espidf")))]
mod tests {
    use super::{Duration, StationLink, StationState, WifiReconnect, RETRY_INTERVAL};

    #[test]
    fn accepted_attempt_waits_for_an_outcome_event() {
        let link = StationLink::new();
        let mut retry = WifiReconnect::new();
        assert!(retry.begin_attempt(&link, Duration::ZERO));
        retry.attempt_returned(Duration::ZERO);

        assert_eq!(link.state(), StationState::Connecting);
        let later = Duration::from_secs(60);
        assert!(!retry.begin_attempt(&link, later));
        link.set_associated(true);
        assert_eq!(link.state(), StationState::Connected);
        assert!(!retry.begin_attempt(&link, later));
    }

    #[test]
    fn repeated_disconnects_retry_and_associate_again() {
        let link = StationLink::new();
        let mut retry = WifiReconnect::new();
        assert!(retry.begin_attempt(&link, Duration::ZERO));
        retry.attempt_returned(Duration::ZERO);
        link.set_associated(true);

        for cycle in 1..=3 {
            link.set_associated(false);
            link.set_associated(false);
            let now = RETRY_INTERVAL * cycle;
            assert!(!retry.begin_attempt(&link, now - Duration::from_millis(1)));
            assert!(retry.begin_attempt(&link, now));
            assert!(!retry.begin_attempt(&link, now));
            retry.attempt_returned(now);
            link.set_associated(true);
            assert_eq!(link.state(), StationState::Connected);
        }
    }

    #[test]
    fn extended_outage_retries_failed_associations_and_rejected_calls() {
        let link = StationLink::new();
        let mut retry = WifiReconnect::new();
        let mut now = Duration::ZERO;
        let outcomes = [
            (false, true, false),
            (false, false, true),
            (true, false, false),
            (true, true, false),
            (true, false, true),
        ];
        for (rejected, disconnect_before_return, disconnect_after_return) in
            outcomes.into_iter().cycle().take(100)
        {
            assert!(retry.begin_attempt(&link, now));
            let returned = now + Duration::from_millis(100);
            if disconnect_before_return {
                link.set_associated(false);
            }
            retry.attempt_returned(returned);
            if rejected {
                link.connect_rejected();
            }
            if disconnect_after_return {
                link.set_associated(false);
            }
            assert_eq!(link.state(), StationState::Disconnected);
            now = returned + Duration::from_secs(5);
            assert!(!retry.begin_attempt(&link, now - Duration::from_micros(1)));
        }
        assert!(retry.begin_attempt(&link, now));
    }

    #[test]
    fn connect_return_preserves_an_early_association_event() {
        for rejected in [false, true] {
            let link = StationLink::new();
            let mut retry = WifiReconnect::new();
            assert!(retry.begin_attempt(&link, Duration::ZERO));
            link.set_associated(true);

            let returned = Duration::from_millis(100);
            retry.attempt_returned(returned);
            if rejected {
                link.connect_rejected();
            }
            assert_eq!(link.state(), StationState::Connected);
            let deadline = returned + Duration::from_secs(5);
            assert!(!retry.begin_attempt(&link, deadline));

            link.set_associated(false);
            assert!(retry.begin_attempt(&link, deadline));
        }
    }

    #[test]
    fn driver_unavailability_prevents_retries_even_with_late_events() {
        let association_events = [true, false];
        for stop_after in 0..=association_events.len() {
            let (before_stop, after_stop) = association_events.split_at(stop_after);
            let link = StationLink::new();
            let mut retry = WifiReconnect::new();
            assert!(retry.begin_attempt(&link, Duration::ZERO));
            for &associated in before_stop {
                link.set_associated(associated);
            }

            link.disable();
            retry.attempt_returned(Duration::ZERO);
            link.connect_rejected();
            assert_eq!(link.state(), StationState::Unavailable);
            for &associated in after_stop {
                link.set_associated(associated);
                assert_eq!(link.state(), StationState::Unavailable);
            }
            assert!(!retry.begin_attempt(&link, RETRY_INTERVAL));
        }
    }
}
