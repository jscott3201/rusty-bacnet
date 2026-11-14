//! The fault-detection and Schedule tasks. Each pass commits the intrinsic
//! transitions its changes cause under the guard that makes them, as a write
//! does (#2535), and hands its COV fanout to the task's [`FanoutTail`], which
//! waits for the pass's event notifications off the task's loop (#2537).

use super::super::cov_fanout::CovFanout;
use super::super::event_notifications::committing;
use super::super::event_send::Settle;
use super::tail::FanoutTail;
use super::*;
use crate::committed_cov::BackgroundCommit;

/// The opt-in fault-detection task: every 10 seconds, each object's own
/// reliability evaluation, then the transition each changed Reliability
/// causes, FAULT or back to NORMAL, both under one guard.
pub(super) async fn fault_detection<T: TransportPort + 'static>(fanout: CovFanout<T>) {
    let tail = FanoutTail::new(&fanout, None);
    let passes = async {
        let detector = crate::fault_detection::FaultDetector::default();
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            let mut settle = Settle::default();
            let committed = {
                let mut db_guard = fanout.db.write().await;
                // The detector mutates objects in place, so snapshot Life
                // Safety state up front.
                let mut commit = BackgroundCommit::snapshot_all(&db_guard);
                for change in detector.evaluate(&mut db_guard) {
                    debug!(
                        object = %change.object_id,
                        old = %change.old_reliability,
                        new = %change.new_reliability,
                        "Fault detection: reliability changed"
                    );
                    commit.changed(change.object_id);
                }
                let mut transition = committing(|| fanout.event_delivery(), &mut settle);
                let (db, table) = (&fanout.db, &fanout.cov_table);
                commit
                    .finish(db, &mut db_guard, table, &mut transition)
                    .await
            };
            tail.hand_off(settle, committed, Vec::new()).await;
        }
    };
    tokio::join!(passes, tail.run());
}

/// The 60-second Schedule pass, and the work an application's own `add` or
/// `remove` queued on the database (`crate::membership`), woken by
/// `membership`. The Command runs a pass's writes queue start through
/// `runner` as the pass is handed to the tail, without waiting for its event
/// notifications, as a write's runs do.
pub(super) async fn schedules<T: TransportPort + 'static>(
    fanout: CovFanout<T>,
    membership: Arc<tokio::sync::Notify>,
    runner: super::super::command_runs::CommandRunner<T>,
) {
    let tail = FanoutTail::new(&fanout, Some(runner));
    let passes = async {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            let (db, cov_table) = (&fanout.db, &fanout.cov_table);
            let mut settle = Settle::default();
            let committed = {
                let mut transition = committing(|| fanout.event_delivery(), &mut settle);
                tokio::select! {
                    _ = interval.tick() => {
                        crate::schedule::tick_schedules_committed(db, cov_table, &mut transition)
                            .await
                    }
                    () = membership.notified() => {
                        crate::membership::settle_committed(db, cov_table, &mut transition).await
                    }
                }
            };
            tail.hand_off(settle, committed, Vec::new()).await;
        }
    };
    tokio::join!(passes, tail.run());
}
