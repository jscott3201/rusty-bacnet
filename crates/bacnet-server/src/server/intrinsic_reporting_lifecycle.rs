//! Periodic intrinsic notification owner; uses the server's effective raw acceptance.
use super::super::cov_fanout::CovFanout;
use super::*;

pub(super) async fn run<T: TransportPort + 'static>(
    fanout: CovFanout<T>,
    learned_routers: Arc<Mutex<LearnedRouterCache>>,
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    // The countdown decrements exactly once per call, so a delayed wake
    // must NOT burst-deliver missed ticks (each would decrement
    // `remaining`, compressing the Time_Delay). `Delay` collapses a
    // missed deadline into a single tick, preserving per-second
    // granularity (Time_Delay counts seconds, ASHRAE 135-2020 Clause 13.3).
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        // DCC gates the outbound sender, not event-state detection or
        // the local transition actions in Clause 13.2.2.1.4.
        // Collect resolved transitions under a brief write lock, then
        // drop it before sending (never hold the db lock across a
        // network send — matches the per-write notification path).
        //
        // Event_Enable gates distribution only (Clause 12.12), so every
        // proposal is committed locally before a suppressed
        // transition is omitted from the outbound work list.
        let (fired, committed) = {
            let mut db = fanout.db.write().await;
            let mut out = Vec::new();
            let mut commit = BackgroundCommit::new();
            for oid in db.list_objects() {
                // Event_Algorithm_Inhibit follows its reference once a
                // second at least, under this same guard (#1329).
                db.follow_event_algorithm_inhibit(&oid);
                let Some(outcome) = db
                    .get_mut(&oid)
                    .and_then(|object| object.tick_intrinsic_reporting())
                else {
                    continue;
                };
                // A proposal can commit and still have its projection rejected,
                // so owe the fanout for every proposal; COV criteria drop it if
                // nothing changed.
                commit.before_change(&db, oid);
                commit.changed(oid);
                let resolved =
                    BACnetServer::<T>::commit_intrinsic_transition(&mut db, &oid, outcome);
                if let Some(resolved) = resolved
                    && resolved.distribute
                    && resolved.event_values.is_some()
                {
                    out.push((oid, resolved));
                }
            }
            // Each committed transition changes Status_Flags. Timestamped
            // references capture it at commit time; every subscriber hears of
            // it once the guard is dropped, after the event notifications, as
            // on the write path (#889).
            (
                out,
                commit.finish(&fanout.db, &mut db, &fanout.cov_table).await,
            )
        };
        for (oid, resolved) in fired {
            BACnetServer::<T>::build_and_send_event_notification_with_bindings(
                &fanout.event_delivery(&learned_routers, &device_bindings),
                &oid,
                resolved,
            )
            .await;
        }
        fanout.fire(&committed).await;
    }
}
