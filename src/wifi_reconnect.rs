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

#[cfg(test)]
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
        for attempt in 0..100 {
            assert!(retry.begin_attempt(&link, now));
            let returned = now + Duration::from_millis(100);
            retry.attempt_returned(returned);
            if attempt % 2 == 0 {
                link.set_associated(false);
            } else {
                link.connect_rejected();
            }
            now = returned + RETRY_INTERVAL;
            assert!(!retry.begin_attempt(&link, now - Duration::from_millis(1)));
        }
        assert!(retry.begin_attempt(&link, now));
    }

    #[test]
    fn connect_rejection_preserves_an_early_association_event() {
        let link = StationLink::new();
        let mut retry = WifiReconnect::new();
        assert!(retry.begin_attempt(&link, Duration::ZERO));
        link.set_associated(true);
        retry.attempt_returned(Duration::ZERO);
        link.connect_rejected();
        assert_eq!(link.state(), StationState::Connected);
        assert!(!retry.begin_attempt(&link, RETRY_INTERVAL));
    }

    #[test]
    fn driver_unavailability_prevents_retries_even_with_late_events() {
        let link = StationLink::new();
        let mut retry = WifiReconnect::new();
        assert!(retry.begin_attempt(&link, Duration::ZERO));
        link.disable();
        retry.attempt_returned(Duration::ZERO);
        link.connect_rejected();
        link.set_associated(true);
        link.set_associated(false);
        assert_eq!(link.state(), StationState::Unavailable);
        assert!(!retry.begin_attempt(&link, RETRY_INTERVAL));
    }
}
