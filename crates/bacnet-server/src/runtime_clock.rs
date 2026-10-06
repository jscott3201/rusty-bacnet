//! The clock the server's timed windows read (#1548, #1550, #1556).
//!
//! Every window here takes the instants it stores and compares from
//! [`now`]:
//! - the rate limiters: discovery, time synchronization, the DCC disable
//!   budget and the received-event-log allowances;
//! - how long a device binding learned from an I-Am stays fresh;
//! - COV subscription lifetimes;
//! - how long a completed LifeSafetyOperation response is kept for replay;
//! - how long a received ConfirmedEventNotification is remembered, so a
//!   retransmission isn't forwarded twice.
//!
//! The forwarding-cap warning throttle still reads the system clock: it only
//! picks the level of a log line.

use std::time::Instant;

/// Now, on tokio's clock.
///
/// In production this is exactly `Instant::now()`: only tokio's `test-util`
/// feature can pause the clock, and it is a dev-dependency, so a release
/// build reads the system's monotonic clock with or without a runtime. Under
/// `#[tokio::test(start_paused = true)]` it reads the paused clock, so a test
/// steps these windows with `tokio::time::advance` and a runner stall can't
/// move one.
///
/// Each of these windows takes every instant it stores or compares from
/// here, never from `Instant::now()` directly: under a paused clock the two
/// differ, and a window measured between them would be wrong. A paused test
/// that hands one of them an instant builds it from here too.
pub(crate) fn now() -> Instant {
    tokio::time::Instant::now().into_std()
}
