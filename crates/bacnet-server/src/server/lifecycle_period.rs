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

/// The database's monotonic clock: its origin, and the notify its objects
/// wake when a write arms a deadline (#1384).
pub(in crate::server) struct MonotonicClocks {
    pub(in crate::server) origin: tokio::time::Instant,
    pub(in crate::server) deadline_armed: Arc<tokio::sync::Notify>,
}

/// Install wall and monotonic clocks before the database becomes shared.
pub(super) fn install_database_clocks(
    db: &mut ObjectDatabase,
    clock_config: Option<ClockConfig>,
) -> (Option<Arc<ServerClock>>, MonotonicClocks) {
    let clock = clock_config.map(|config| Arc::new(ServerClock::new(config)));
    let reader = clock
        .as_ref()
        .map(|clock| Arc::clone(clock) as Arc<dyn bacnet_objects::clock::ClockReader>);
    db.set_clock_reader(reader);
    let origin = tokio::time::Instant::now();
    let monotonic_clock: Arc<bacnet_objects::traits::MonotonicClock> =
        Arc::new(move || tokio::time::Instant::now().saturating_duration_since(origin));
    db.set_monotonic_clock(Some(monotonic_clock));
    let deadline_armed = Arc::new(tokio::sync::Notify::new());
    let waker = Arc::clone(&deadline_armed);
    db.set_deadline_waker(Some(Arc::new(move || waker.notify_one())));

    (
        clock,
        MonotonicClocks {
            origin,
            deadline_armed,
        },
    )
}
