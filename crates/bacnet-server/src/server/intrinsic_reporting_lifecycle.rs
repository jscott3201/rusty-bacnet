//! Periodic intrinsic notification owner; uses the server's effective raw acceptance.
use super::super::binding_probes::after;
use super::super::cov_fanout::CovFanout;
use super::super::event_notifications::logged_intrinsic_commit;
use super::super::event_send::{Settle, settled, settled_by};
use super::*;
use crate::committed_cov::CommittedCov;
use bacnet_device_core::event::{next_due, sweep};
use std::collections::{HashMap, HashSet, VecDeque};

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

/// One sweep pass's COV fanout, waiting its turn in [`fan_out`].
struct Tail {
    /// What it waits for first: its event notifications' hand-off, when it
    /// pushed any (#889).
    settle: Option<oneshot::Receiver<()>>,
    committed: CommittedCov,
    /// When that wait ends at the latest: the APDU timeout after the pass
    /// committed.
    deadline: tokio::time::Instant,
    /// The objects an earlier pass handed to this one ([`leave_to_later`]):
    /// this pass reports them itself, never handing them on again.
    handed_in: HashSet<ObjectIdentifier>,
}

/// The passes waiting behind the one [`fan_out`] is on, in sweep order: a
/// lock never held across an await, and the wake for a pass pushed.
#[derive(Default)]
struct Tails {
    queue: std::sync::Mutex<VecDeque<Tail>>,
    pushed: tokio::sync::Notify,
}

impl Tails {
    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<Tail>> {
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
/// A report carries the object's state when it goes. A pass's report goes
/// within the cap of its commit. When a pass's notifications go in time,
/// each object a pass still queued also fans out for is handed to the first
/// such pass ([`leave_to_later`]) and reported with it: after that pass's
/// notifications when they go, at its deadline when they don't, and at most
/// two caps after the commit that first wanted it, since an object is
/// handed on once at most and a pass whose wait ran out hands nothing on. So
/// two passes behind a held lane merge into one report, and a state that
/// came and went meanwhile is told by its event notifications, not by COV
/// (a write's own fanout beside the sweep can merge the same way). A pass
/// that commits while an earlier pass's fanout is already reading the
/// object can still be reported by that earlier fanout, with the state it
/// reads then.
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
        let deadline = after(tokio::time::Instant::now(), cap);
        let overflow = {
            let mut queue = tails.lock();
            if queue.len() < MAX_WAITING_FANOUTS {
                queue.push_back(Tail {
                    settle,
                    committed,
                    deadline,
                    handed_in: HashSet::new(),
                });
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
/// pass's event notifications are handed off or its deadline has passed
/// (#889). Only a pass whose notifications went in time hands objects on.
/// Nothing is held across the wait.
async fn fan_out<T: TransportPort + 'static>(fanout: &CovFanout<T>, tails: &Tails) {
    loop {
        let next = tails.lock().pop_front();
        let Some(mut pass) = next else {
            tails.pushed.notified().await;
            continue;
        };
        let settle = pass.settle.take();
        let transactions = &fanout.notification_transactions;
        if settled_by(settle, pass.deadline, transactions).await {
            leave_to_later(&mut pass, tails);
        }
        fanout.fire(&pass.committed).await;
    }
}

/// Leave out of `pass`'s fanout each object a pass queued behind it fans out
/// for too, unless an earlier pass handed the object to `pass`: its report
/// goes with the first such pass, which reports it itself. A Life Safety
/// object's changed properties go along.
///
/// So an object is handed on once at most, and is reported at most two caps
/// after the commit that first wanted it, however often it changes: the
/// pass it goes to committed before `pass`'s deadline, and reports within a
/// cap of its own commit. The lock covers only the walks over the later
/// passes.
fn leave_to_later(pass: &mut Tail, tails: &Tails) {
    if tails.lock().is_empty() {
        return;
    }
    let mine = |oid: &ObjectIdentifier| !pass.handed_in.contains(oid);
    let mut coarse: HashSet<ObjectIdentifier> =
        pass.committed.coarse.iter().copied().filter(mine).collect();
    let mut life_safety: HashMap<ObjectIdentifier, usize> = pass
        .committed
        .life_safety
        .iter()
        .enumerate()
        .map(|(index, change)| (change.object_identifier, index))
        .filter(|(oid, _)| mine(oid))
        .collect();
    if coarse.is_empty() && life_safety.is_empty() {
        return;
    }
    let mut moved = HashSet::new();
    for later in tails.lock().iter_mut() {
        if coarse.is_empty() && life_safety.is_empty() {
            break;
        }
        for oid in &later.committed.coarse {
            if coarse.remove(oid) {
                later.handed_in.insert(*oid);
                moved.insert(*oid);
            }
        }
        for change in &mut later.committed.life_safety {
            let oid = change.object_identifier;
            let Some(index) = life_safety.remove(&oid) else {
                continue;
            };
            for property in &pass.committed.life_safety[index].changed_properties {
                if !change.changed_properties.contains(property) {
                    change.changed_properties.push(*property);
                }
            }
            later.handed_in.insert(oid);
            moved.insert(oid);
        }
    }
    let committed = &mut pass.committed;
    committed.coarse.retain(|oid| !moved.contains(oid));
    committed
        .life_safety
        .retain(|change| !moved.contains(&change.object_identifier));
}
