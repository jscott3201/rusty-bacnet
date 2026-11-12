//! A test-only monotonic clock for the detectors' `now` (#2303).
//!
//! Each test thread has its own clock starting at zero. [`test_now`] reads
//! it; [`test_tick`] moves it on one second and reads it, standing for the
//! periodic task's next call, so a test written against the one-second
//! countdown keeps its meaning: a transition seeded at one reading with
//! `Time_Delay` N fires at the Nth tick after it.

use core::cell::Cell;

use bacnet_time::{Duration, Instant};

use super::PendingTransition;

std::thread_local! {
    static NOW: Cell<Instant> = const { Cell::new(Instant::from_nanos(0)) };
}

/// The clock's current reading.
pub(crate) fn test_now() -> Instant {
    NOW.with(Cell::get)
}

/// Move the clock on one second and read it.
pub(crate) fn test_tick() -> Instant {
    NOW.with(|now| {
        let next = now.get() + Duration::from_secs(1);
        now.set(next);
        next
    })
}

/// The instant `seconds` after the clock's reading: a pending transition
/// due then fires at the `seconds`th [`test_tick`].
pub(crate) fn in_seconds(seconds: u32) -> Instant {
    test_now() + Duration::from_secs(u64::from(seconds))
}

impl PendingTransition {
    /// Whole seconds from the test clock's reading to when this is due, as
    /// the one-second countdown counted them: the ticks left before it fires.
    pub(crate) fn remaining(&self) -> u32 {
        u32::try_from(self.due.saturating_duration_since(test_now()).as_secs())
            .expect("a test's delay fits in u32")
    }
}
