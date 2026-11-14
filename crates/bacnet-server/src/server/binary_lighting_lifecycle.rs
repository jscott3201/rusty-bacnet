use super::cov_fanout::CovFanout;
use super::event_notifications::committing;
use super::event_send::Settle;
use super::lifecycle::tail::{FanoutTail, Snapshot};
use super::*;
use crate::committed_cov::{BackgroundCommit, CommitTransition};
use tokio::time::{Instant, MissedTickBehavior};

/// Spawn the task that drives every object's monotonic operation deadlines.
///
/// It is generic over objects: each wake first takes the Averaging samples
/// that are due (#1144) and counts each Pulse Converter's input from the
/// property its Input_Reference names (#1341), then asks every object with
/// staged writes to drop the ones that waited too long
/// (`DurableWrites::expire_staged`) and every timed object to expire what is
/// due (`Timed::advance_monotonic_time`) and to report its next deadline.
/// Each object that changed commits the intrinsic transition its new state
/// causes under the same guard (#2535), and the pass's COV fanout goes to the
/// task's [`FanoutTail`], behind the pass's event notifications and off its
/// loop (#2537). Binary Lighting Output egress, Lighting Output
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
/// A Timer that expires in the pass leaves the writes its transition makes
/// (#2206). They are taken under the pass's guard, as a write's runs are,
/// and started through `runner` once the guard is dropped.
///
/// It wakes once a second, at the earliest deadline it gathered, and when an
/// object arms a deadline in a write (`MonotonicClocks::deadline_armed`), so
/// one sooner than the next routine pass, such as a short fade's end, is
/// still met on time. None of the three futures it waits on takes a lock.
pub(super) fn spawn_binary_lighting_operation_task<T: TransportPort + 'static>(
    fanout: CovFanout<T>,
    monotonic: super::lifecycle::MonotonicClocks,
    runner: super::command_runs::CommandRunner<T>,
) -> JoinHandle<()> {
    let owner = fanout.notification_transactions.audit_owner_lease();
    let monotonic_origin = monotonic.origin;
    let deadline_armed = monotonic.deadline_armed;
    super::heap_futures::spawn_boxed(move || async move {
        let _owner = owner;
        let tail = FanoutTail::new(&fanout, None);
        let passes = async {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut next_deadline: Option<Duration> = None;
            loop {
                let deadline = async {
                    match next_deadline {
                        Some(deadline) => {
                            tokio::time::sleep_until(monotonic_origin + deadline).await
                        }
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = deadline => {}
                    _ = deadline_armed.notified() => {}
                }
                let now = Instant::now().saturating_duration_since(monotonic_origin);
                let mut settle = Settle::default();
                let (changed, sampled, runs) = {
                    let mut database = fanout.db.write().await;
                    let database = &mut *database;
                    // Sampling first steps each due schedule, so the deadlines
                    // gathered below are the next ones. The samples fan out as
                    // a background commit, like a schedule write.
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
                    let mut advanced = Vec::new();
                    next_deadline = None;
                    database.for_each_object_mut(|oid, object| {
                        if let Some(timed) = object.timed() {
                            timed.set_tracking_cov_increment(finest.get(&oid).copied());
                        }
                        if let Some(durable) = object.durable_writes() {
                            durable.expire_staged(now);
                        }
                        let Some(timed) = object.timed() else {
                            return;
                        };
                        if timed.advance_monotonic_time(now) {
                            advanced.push(oid);
                            if let Some(snapshot) = timed.cov_snapshot() {
                                changed.push((oid, snapshot));
                            }
                        }
                        if let Some(deadline) = timed.next_monotonic_deadline() {
                            next_deadline =
                                Some(next_deadline.map_or(deadline, |next| next.min(deadline)));
                        }
                    });
                    // A deadline this pass left due would wake the task at
                    // once, over and over, without letting paused or real time
                    // move on; look at it again a millisecond later instead.
                    next_deadline = next_deadline
                        .map(|deadline: Duration| deadline.max(now + Duration::from_millis(1)));
                    let mut transition = committing(|| fanout.event_delivery(), &mut settle);
                    transitions(
                        database,
                        &advanced,
                        &mut changed,
                        &mut sampled,
                        &mut transition,
                    );
                    if !changed.is_empty() {
                        let captures: Vec<_> = {
                            let table = fanout.cov_table.read().await;
                            changed
                                .iter()
                                .map(|(oid, _)| table.timed_capture(*oid))
                                .collect()
                        };
                        for capture in captures {
                            capture.run(database);
                        }
                    }
                    let runs =
                        crate::command_lists::TakenRuns::take(&fanout.db, database, &advanced);
                    let (db, table) = (&fanout.db, &fanout.cov_table);
                    let sampled = sampled.finish(db, database, table, &mut transition).await;
                    (changed, sampled, runs)
                };

                runner.start(runs);
                // The samples' fanout, then each changed object's from its
                // frozen copy, behind the pass's event notifications.
                tail.hand_off(settle, sampled, changed).await;
            }
        };
        tokio::join!(passes, tail.run());
    })
}

/// Commit, under the pass's guard `db`, the intrinsic transition each object
/// in `advanced` causes (#2535). One that commits takes its frozen copy in
/// `changed` again, after the commit, so its report carries its new
/// Status_Flags, or joins `sampled`'s fanout when it lends no copy.
fn transitions(
    db: &mut ObjectDatabase,
    advanced: &[ObjectIdentifier],
    changed: &mut [Snapshot],
    sampled: &mut BackgroundCommit,
    transition: CommitTransition<'_>,
) {
    for oid in advanced {
        if !transition(db, *oid) {
            continue;
        }
        match changed.iter_mut().find(|(changed, _)| changed == oid) {
            Some((_, snapshot)) => {
                let again = db.get_mut(oid).and_then(|object| object.timed());
                if let Some(again) = again.and_then(|timed| timed.cov_snapshot()) {
                    *snapshot = again;
                }
            }
            None => sampled.changed(*oid),
        }
    }
}
