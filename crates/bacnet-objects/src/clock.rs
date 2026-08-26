//! Dependency-neutral clock data exposed to BACnet objects.

use bacnet_types::primitives::{Date, Time};

/// One coherent sample of the Device clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockFrame {
    /// Device local date.
    pub local_date: Date,
    /// Device local time.
    pub local_time: Time,
    /// Signed minutes west of UTC.
    pub utc_offset: i16,
    /// Whether daylight-saving time is currently applied.
    pub daylight_savings_status: bool,
}

impl ClockFrame {
    /// Return the BACnetDaysOfWeek bit for this frame, or `None` for an
    /// unavailable/invalid day-of-week value.
    pub fn day_of_week_bit(self) -> Option<u8> {
        (1..=7)
            .contains(&self.local_date.day_of_week)
            .then(|| 1 << (self.local_date.day_of_week - 1))
    }
}

/// Synchronous read port for a coherent Device clock sample.
///
/// `None` means that no wall-clock frame is available. Implementations must
/// return all four fields from the same sample.
pub trait ClockReader: Send + Sync {
    /// Read one coherent frame, or report that no wall clock is available.
    fn read_clock(&self) -> Option<ClockFrame>;
}
