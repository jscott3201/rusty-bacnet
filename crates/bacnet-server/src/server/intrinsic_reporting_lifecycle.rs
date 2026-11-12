//! Periodic intrinsic notification owner; uses the server's effective raw acceptance.
use super::super::cov_fanout::CovFanout;
use super::super::event_notifications::logged_intrinsic_commit;
use super::*;
use bacnet_device_core::event::{next_due, sweep};

pub(super) async fn run<T: TransportPort + 'static>(
    fanout: CovFanout<T>,
    learned_routers: Arc<Mutex<LearnedRouterCache>>,
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
) {
    // The sweep runs once a second, as it always has: it follows
    // Event_Algorithm_Inhibit references and re-derives FAULT from
    // Reliability, which no write may report. Between those sweeps it also
    // runs when the database's next Time_Delay ends, so a transition fires at
    // its due instant rather than the next whole second (#2303). A late wake
    // runs one sweep, not one per missed second, and the cadence restarts a
    // period after it, as `MissedTickBehavior::Delay` did.
    let period = Duration::from_secs(1);
    let mut next_sweep = tokio::time::Instant::now();
    let mut due: Option<tokio::time::Instant> = None;
    loop {
        let now = tokio::time::Instant::now();
        let wake = match due {
            Some(due) if due > now && due < next_sweep => due,
            _ => next_sweep,
        };
        tokio::time::sleep_until(wake).await;
        let woke = tokio::time::Instant::now();
        if woke >= next_sweep {
            next_sweep = if woke > next_sweep {
                woke + period
            } else {
                next_sweep + period
            };
        }
        // DCC gates the outbound sender, not event-state detection or
        // the local transition actions in Clause 13.2.2.1.4.
        // Collect resolved transitions under a brief write lock, then
        // drop it before sending (never hold the db lock across a
        // network send — matches the per-write notification path).
        //
        // Event_Enable gates distribution only (Clause 13.2.5), so every
        // proposal is committed locally before a suppressed
        // transition is omitted from the outbound work list.
        let (fired, committed) = {
            let mut db = fanout.db.write().await;
            let mut out = Vec::new();
            let mut commit = BackgroundCommit::new();
            // A proposal can commit and still have its projection rejected,
            // so owe the fanout for every proposal; COV criteria drop it if
            // nothing changed.
            let swept = sweep(&mut db, crate::runtime_clock::core_now(), |db, oid| {
                commit.before_change(db, oid);
                commit.changed(oid);
            });
            for (oid, result) in swept {
                if let Some(resolved) = logged_intrinsic_commit(&oid, result)
                    && resolved.distribute
                    && resolved.event_values.is_some()
                {
                    out.push((oid, resolved));
                }
            }
            // When the next delay ends, on tokio's clock.
            due = next_due(&db)
                .and_then(|due| crate::runtime_clock::core_clock().to_std(due))
                .map(tokio::time::Instant::from_std);
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
