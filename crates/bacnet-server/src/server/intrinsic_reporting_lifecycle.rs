//! Periodic intrinsic notification owner; uses the server's effective raw acceptance.
use super::super::cov_fanout::CovFanout;
use super::super::event_notifications::logged_intrinsic_commit;
use super::super::event_send::{Settle, settled};
use super::*;
use bacnet_device_core::event::{next_due, sweep};

/// The sweep's period: the standing duties (FAULT from Reliability,
/// Event_Algorithm_Inhibit references) run this often. It must not exceed
/// the shortest nonzero Time_Delay, one second; see [`run`].
const SWEEP_PERIOD: Duration = Duration::from_secs(1);

/// The least time from one sweep to the next when the next is woken for a
/// due instant rather than by the period. Each wake runs the whole sweep and
/// a second pass over the database for `next_due` (about 0.4 ms at 10,000
/// Analog Inputs, measured in review), and staggered delays could otherwise
/// wake the task a thousand times a second. So a transition can go out up
/// to this long after its due instant.
const DUE_WAKE_FLOOR: Duration = Duration::from_millis(100);

/// The intrinsic-reporting task (#2303).
///
/// It sweeps every [`SWEEP_PERIOD`], and between those sweeps when the
/// database's next Time_Delay ends ([`next_due`]), no sooner than
/// [`DUE_WAKE_FLOOR`] after the last sweep, so a transition goes out when
/// its delay ends rather than at the next whole second.
///
/// The task isn't told about writes, and needn't be. It never sleeps past
/// the next periodic sweep; a write seeds a due instant at least one second
/// (the shortest nonzero Time_Delay) after the write; and the write path
/// and the sweep take the same database lock. So a sweep always runs after
/// the write and before its due instant, sees it, and recomputes when to
/// wake. That holds only while the period is no longer than the shortest
/// nonzero delay: a host that sweeps less often must also wake on writes.
///
/// A sweep at or after the periodic instant moves the cadence on: the next
/// periodic sweep is a period after the instant it replaced when it ran on
/// time, and a period after it ran when it ran late, so a stall runs one
/// sweep, not one per missed second. A sweep woken for a due instant leaves
/// the cadence as it was.
pub(super) async fn run<T: TransportPort + 'static>(
    fanout: CovFanout<T>,
    learned_routers: Arc<Mutex<LearnedRouterCache>>,
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
) {
    let mut next_sweep = tokio::time::Instant::now();
    let mut last_sweep = next_sweep;
    let mut due: Option<tokio::time::Instant> = None;
    loop {
        let wake = match due {
            Some(due) if due < next_sweep => due.max(last_sweep + DUE_WAKE_FLOOR).min(next_sweep),
            _ => next_sweep,
        };
        tokio::time::sleep_until(wake).await;
        let woke = tokio::time::Instant::now();
        last_sweep = woke;
        if woke >= next_sweep {
            next_sweep = if woke > next_sweep {
                woke + SWEEP_PERIOD
            } else {
                next_sweep + SWEEP_PERIOD
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
        let (settle, committed) = {
            let mut db = fanout.db.write().await;
            let mut settle = Settle::default();
            let mut commit = BackgroundCommit::new();
            // A proposal can commit and still have its projection rejected,
            // so owe the fanout for every proposal; COV criteria drop it if
            // nothing changed.
            let swept_at = crate::runtime_clock::core_now();
            let swept = sweep(&mut db, swept_at, |db, oid| {
                commit.before_change(db, oid);
                commit.changed(oid);
            });
            // Each distributed transition joins its recipients' lanes under
            // this guard, in commit order (#2537).
            for (oid, result) in swept {
                if let Some(resolved) = logged_intrinsic_commit(&oid, result)
                    && resolved.distribute
                    && resolved.event_values.is_some()
                {
                    BACnetServer::<T>::enqueue_event_notification(
                        &fanout.event_delivery(&learned_routers, &device_bindings),
                        &mut db,
                        &oid,
                        resolved,
                        Some(&mut settle),
                    );
                }
            }
            // When the next delay after this sweep ends, on tokio's clock.
            due = next_due(&db, swept_at)
                .and_then(|due| crate::runtime_clock::core_clock().to_std(due))
                .map(tokio::time::Instant::from_std);
            // Each committed transition changes Status_Flags. Timestamped
            // references capture it at commit time; every subscriber hears of
            // it once the guard is dropped, after the event notifications, as
            // on the write path (#889).
            (
                settle,
                commit.finish(&fanout.db, &mut db, &fanout.cov_table).await,
            )
        };
        // The fanout waits for those notifications, capped (#889).
        if let Some(waiting) = settle.into_wait() {
            let delivery = fanout.event_delivery(&learned_routers, &device_bindings);
            let timeout_ms = delivery.retry_timeout_ms;
            settled(Some(waiting), timeout_ms, delivery.suppressions).await;
        }
        fanout.fire(&committed).await;
    }
}
