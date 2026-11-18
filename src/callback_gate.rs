//! Nonblocking admission and draining for one packet-forwarding direction.

use core::sync::atomic::{AtomicU8, Ordering};

const OPEN: u8 = 1;
const ACTIVE: u8 = 2;

pub struct CallbackGate {
    state: AtomicU8,
}

impl CallbackGate {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(0),
        }
    }

    /// Only the driver owner opens the gate, after publishing the transmit handle.
    ///
    /// # Panics
    /// Panics if the gate is already open or has not finished draining.
    pub fn open(&self) {
        assert!(self
            .state
            .compare_exchange(0, OPEN, Ordering::Release, Ordering::Relaxed)
            .is_ok());
    }

    /// Drop a frame rather than wait if closed or already forwarding this way.
    /// The pinned SDK serializes RX per direction; busy is defensive only.
    /// Admission protects transmit resources, not the lifetime of RX dispatch.
    pub fn try_enter(&self) -> Option<CallbackGuard<'_>> {
        self.state
            .compare_exchange(OPEN, OPEN | ACTIVE, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| CallbackGuard(self))
    }

    /// Prevent new forwarding; an existing guard must still be drained.
    pub fn close(&self) {
        self.state.fetch_and(!OPEN, Ordering::AcqRel);
    }

    /// After closing, acquire the last guard's release before destroying drivers.
    pub fn is_idle(&self) -> bool {
        self.state.load(Ordering::Acquire) & ACTIVE == 0
    }
}

#[must_use = "keep the guard alive until forwarding finishes"]
pub struct CallbackGuard<'a>(&'a CallbackGate);

impl Drop for CallbackGuard<'_> {
    fn drop(&mut self) {
        // Clear ACTIVE without reopening a gate closed during this callback.
        self.0.state.fetch_and(!ACTIVE, Ordering::Release);
    }
}
