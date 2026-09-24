use bacnet_objects::clock::ClockFrame;
use bacnet_types::primitives::{Date, Time};
/// Project the request-level and per-value COV timestamps from one sample.
pub(crate) fn cov_multiple_datetime(frame: ClockFrame) -> (Date, Time) {
    (frame.local_date, frame.local_time)
}
