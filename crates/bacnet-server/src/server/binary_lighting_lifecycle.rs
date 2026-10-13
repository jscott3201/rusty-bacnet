use super::cov_fanout::CovFanout;
use super::*;
use crate::committed_cov::BackgroundCommit;
use tokio::time::{Instant, MissedTickBehavior};

/// Spawn the task that drives every object's monotonic operation deadlines.
///
/// It is generic over objects: each wake first takes the Averaging samples
/// that are due (#1144) and counts each Pulse Converter's input from the
/// property its Input_Reference names (#1341), then asks every object to
/// expire what is due (`advance_monotonic_time_internal`) and to report its
/// next deadline, and fans COV out for each object that changed once the
/// database guard is dropped. Binary Lighting Output egress, Lighting Output
/// fades, ramps and egress (#1384), Color and Color Temperature fades and
/// ramps (#1474), Access Door pulse relock (#1073), Averaging sampling and
/// Pulse Converter counting all run on it; the name predates the others.
///
/// Before each advance it passes every object the finest COV increment its
/// Tracking_Value subscribers ask for, so a fade or ramp samples Tracking_Value
/// at least that finely (#1510). It reads that from the COV table inside the
/// database guard, in the server's lock order, and lets the table go before
/// any object runs.
///
/// It wakes once a second, at the earliest deadline it gathered, and when an
/// object arms a deadline in a write (`MonotonicClocks::deadline_armed`), so
/// one sooner than the next routine pass, such as a short fade's end, is
/// still met on time. None of the three futures it waits on takes a lock.
pub(super) fn spawn_binary_lighting_operation_task<T: TransportPort + 'static>(
    fanout: CovFanout<T>,
    monotonic: super::lifecycle::MonotonicClocks,
) -> JoinHandle<()> {
    let owner = fanout.notification_transactions.audit_owner_lease();
    let monotonic_origin = monotonic.origin;
    let deadline_armed = monotonic.deadline_armed;
    super::heap_futures::spawn_boxed(move || async move {
        let _owner = owner;
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut next_deadline: Option<Duration> = None;
        loop {
            let deadline = async {
                match next_deadline {
                    Some(deadline) => tokio::time::sleep_until(monotonic_origin + deadline).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = interval.tick() => {}
                _ = deadline => {}
                _ = deadline_armed.notified() => {}
            }
            let now = Instant::now().saturating_duration_since(monotonic_origin);

            let (changed, sampled) = {
                let mut database = fanout.db.write().await;
                // Sampling first steps each due schedule, so the deadlines
                // gathered below are the next ones. The samples fan out as a
                // background commit, like a schedule write.
                let mut sampled = BackgroundCommit::new();
                for oid in database.sample_due_averaging_objects(now) {
                    sampled.changed(oid);
                }
                for oid in database.count_pulse_inputs() {
                    sampled.changed(oid);
                }
                let finest = fanout
                    .cov_table
                    .read()
                    .await
                    .finest_increments(PropertyIdentifier::TRACKING_VALUE);
                let mut changed = Vec::new();
                next_deadline = None;
                database.for_each_object_mut(|oid, object| {
                    object.set_tracking_cov_increment_internal(finest.get(&oid).copied());
                    if object.advance_monotonic_time_internal(now) {
                        if let Some(snapshot) = object.cov_snapshot_internal() {
                            changed.push((oid, snapshot));
                        }
                    }
                    if let Some(deadline) = object.next_monotonic_deadline_internal() {
                        next_deadline =
                            Some(next_deadline.map_or(deadline, |next| next.min(deadline)));
                    }
                });
                // A deadline this pass left due would wake the task at once,
                // over and over, without letting paused or real time move on;
                // look at it again a millisecond later instead.
                next_deadline = next_deadline
                    .map(|deadline: Duration| deadline.max(now + Duration::from_millis(1)));
                if !changed.is_empty() {
                    let captures: Vec<_> = {
                        let table = fanout.cov_table.read().await;
                        changed
                            .iter()
                            .map(|(oid, _)| table.timed_capture(*oid))
                            .collect()
                    };
                    for capture in captures {
                        capture.run(&database);
                    }
                }
                let sampled = sampled
                    .finish(&fanout.db, &mut database, &fanout.cov_table)
                    .await;
                (changed, sampled)
            };

            fanout.fire(&sampled).await;
            for (oid, snapshot) in changed {
                BACnetServer::<T>::fire_cov_notifications_from_snapshot(
                    &fanout.notify_context(),
                    &oid,
                    snapshot.as_ref(),
                )
                .await;
            }
        }
    })
}
