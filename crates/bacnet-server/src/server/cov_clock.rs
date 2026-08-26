use bacnet_objects::clock::ClockFrame;
use bacnet_types::primitives::{Date, Time};
use std::time::Instant;

pub(crate) fn cov_multiple_time_remaining(expires_at: Option<Instant>) -> u32 {
    expires_at.map_or(0, |expires_at| {
        u32::try_from(
            expires_at
                .saturating_duration_since(Instant::now())
                .as_secs(),
        )
        .unwrap_or(u32::MAX)
    })
}

/// Project the request-level and per-value COV timestamps from one sample.
pub(crate) fn cov_multiple_datetime(frame: ClockFrame) -> (Date, Time) {
    (frame.local_date, frame.local_time)
}
