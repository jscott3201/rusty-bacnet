//! Periodic intrinsic notification owner; uses the server's effective raw acceptance.
use super::super::cov_fanout::CovFanout;
use super::super::event_notifications::logged_intrinsic_commit;
use super::super::event_send::{Settle, settled};
use super::*;
use crate::committed_cov::CommittedCov;
use bacnet_device_core::event::{next_due, sweep};

/// The sweep's period: the standing duties (FAULT from Reliability,
/// Event_Algorithm_Inhibit references) run this often. It must not exceed
/// the shortest nonzero Time_Delay, one second; see [`sweeps`].
const SWEEP_PERIOD: Duration = Duration::from_secs(1);

/// The least time from one sweep to the next when the next is woken for a
/// due instant rather than by the period. Each wake runs the whole sweep and
/// a second pass over the database for `next_due` (about 0.4 ms at 10,000
/// Analog Inputs, measured in review), and staggered delays could otherwise
/// wake the task a thousand times a second. So a transition can go out up
/// to this long after its due instant.
const DUE_WAKE_FLOOR: Duration = Duration::from_millis(100);

/// The most sweep passes whose COV fanout waits for their event notifications
/// at once (#2537); a pass past them fans out at once, its wait counted in
/// `cov_wait_expired`.
const MAX_WAITING_FANOUTS: usize = 64;

/// One sweep pass's COV fanout, what it waits for first (its event
/// notifications' hand-off, when it pushed any, #889), and when that wait
/// ends at the latest: the APDU timeout after the pass committed.
type Tail = (
    Option<oneshot::Receiver<()>>,
    CommittedCov,
    tokio::time::Instant,
);

/// The passes waiting behind the one [`fan_out`] is on, in sweep order: a
/// lock never held across an await, and the wake for a pass pushed.
#[derive(Default)]
struct Tails {
    queue: std::sync::Mutex<std::collections::VecDeque<Tail>>,
    pushed: tokio::sync::Notify,
}

impl Tails {
    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::VecDeque<Tail>> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The intrinsic-reporting task (#2303).
///
/// Each pass's COV fanout waits for that pass's event notifications in a
/// second future of this task, at most the APDU timeout from the pass's
/// commit, one pass at a time in the order the sweep made them
/// ([`fan_out`]), so a destination that holds its lane (a Device recipient
/// waiting for its I-Am, a stalled send, every invoke ID leased) holds up no
/// later sweep (#2537). One join polls both, so neither sits unpolled while
/// the other waits, and stopping the task ends both.
///
/// A report carries the object's state when it goes, so an object that
/// passes still queued fan out for again is left to the latest of them:
/// passes behind a held lane merge into one report, after the last one's
/// notifications, and a state that came and went meanwhile is told by its
/// event notifications, not by COV (a write's own fanout beside the sweep
/// can merge the same way).
pub(super) async fn run<T: TransportPort + 'static>(
    fanout: CovFanout<T>,
    learned_routers: Arc<Mutex<LearnedRouterCache>>,
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
) {
    let tails = Tails::default();
    let cap = fanout
        .event_delivery(&learned_routers, &device_bindings)
        .retry_timeout_ms;
    let cap = Duration::from_millis(cap);
    let sweeps = sweeps(&fanout, &learned_routers, &device_bindings, &tails, cap);
    tokio::join!(sweeps, fan_out(&fanout, &tails));
}

/// The sweeps, each pass's COV fanout handed to [`fan_out`].
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
async fn sweeps<T: TransportPort + 'static>(
    fanout: &CovFanout<T>,
    learned_routers: &Arc<Mutex<LearnedRouterCache>>,
    device_bindings: &Arc<RwLock<DeviceBindingTable>>,
    tails: &Tails,
    cap: Duration,
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
                        &fanout.event_delivery(learned_routers, device_bindings),
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
        let settle = settle.into_wait();
        if settle.is_none() && committed.is_empty() {
            continue;
        }
        // The fanout waits for those notifications, capped (#889), behind
        // the earlier passes' and off this loop. Past the bound it goes now,
        // ahead of the passes still waiting, and counts as a wait that ran
        // out unless its notifications are out already.
        let deadline = tokio::time::Instant::now() + cap;
        let overflow = {
            let mut queue = tails.lock();
            if queue.len() < MAX_WAITING_FANOUTS {
                queue.push_back((settle, committed, deadline));
                None
            } else {
                Some((settle, committed))
            }
        };
        match overflow {
            None => tails.pushed.notify_one(),
            Some((settle, committed)) => {
                settled(settle, 0, &fanout.notification_transactions).await;
                fanout.fire(&committed).await;
            }
        }
    }
}

/// Each pass's COV fanout, in the order the sweep made them, once that
/// pass's event notifications are handed off or the APDU timeout has passed
/// (#889). Nothing is held across the wait.
async fn fan_out<T: TransportPort + 'static>(fanout: &CovFanout<T>, tails: &Tails) {
    loop {
        let next = tails.lock().pop_front();
        let Some((settle, mut committed, deadline)) = next else {
            tails.pushed.notified().await;
            continue;
        };
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let left = u64::try_from(left.as_millis()).unwrap_or(u64::MAX);
        settled(settle, left, &fanout.notification_transactions).await;
        leave_to_later(&mut committed, &mut tails.lock());
        fanout.fire(&committed).await;
    }
}

/// Leave out of `pass`'s fanout each object a pass queued behind it fans out
/// for too: its report goes with that pass, after its notifications. A Life
/// Safety object's changed properties go along.
fn leave_to_later(pass: &mut CommittedCov, later: &mut std::collections::VecDeque<Tail>) {
    pass.coarse
        .retain(|oid| !later.iter().any(|(_, next, _)| next.coarse.contains(oid)));
    pass.life_safety.retain(|change| {
        let oid = change.object_identifier;
        let Some(next) = later.iter_mut().find_map(|(_, next, _)| {
            next.life_safety
                .iter_mut()
                .find(|next| next.object_identifier == oid)
        }) else {
            return true;
        };
        for property in &change.changed_properties {
            if !next.changed_properties.contains(property) {
                next.changed_properties.push(*property);
            }
        }
        false
    });
}
