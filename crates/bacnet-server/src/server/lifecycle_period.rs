use super::*;

/// Resolve the configured Event Enrollment interval into a tick period.
///
/// `tokio::time::interval` panics on a zero period, and that panic would land
/// inside a spawned task — `start` would still return `Ok` while enrollment
/// evaluation was silently dead. A configured `0` is clamped to one second
/// instead, matching how an invalid `vendor_id` is handled: warn loudly and
/// keep the device running. Use `enable_event_enrollment(false)` to actually
/// disable evaluation.
pub(in crate::server) fn event_enrollment_period(secs: u64) -> Duration {
    if secs == 0 {
        warn!(
            "event_enrollment_interval_secs is 0; clamping to 1s. \
             Use enable_event_enrollment(false) to disable Event Enrollment evaluation"
        );
        return Duration::from_secs(1);
    }
    Duration::from_secs(secs)
}
