//! The tail a periodic background task's COV fanout goes through (#2537,
//! #2535): the intrinsic sweep's, the fault-detection task's, the Schedule
//! task's and the operation task's.
//!
//! Each of those tasks serves many objects from one loop, and a pass's COV
//! fanout waits for the pass's own event notifications (#889), capped. That
//! wait runs here, in a second future of the task's own beside its loop, one
//! pass at a time in the order the loop made them, so a destination that
//! holds its lane (a Device recipient waiting for its I-Am, a stalled send,
//! every invoke ID leased) holds up none of the loop's later passes; other
//! destinations' notifications go in their own lanes. One join polls both,
//! so neither sits unpolled while the other waits, and stopping the task ends
//! both.
//!
//! A report carries the object's state when it goes. A pass's report goes
//! within the cap of its commit. When a pass's notifications go in time, each
//! object a pass still queued also fans out for is handed to the first such
//! pass ([`leave_to_later`]) and reported with it: after that pass's
//! notifications when they go, at its deadline when they don't, and at most
//! two caps after the commit that first wanted it, since an object is handed
//! on once at most and a pass whose wait ran out hands nothing on. So two
//! passes behind a held lane merge into one report, and a state that came and
//! went meanwhile is told by its event notifications, not by COV (a write's
//! own fanout beside the task's can merge the same way). A pass that commits
//! while an earlier pass's fanout is already reading the object can still be
//! reported by that earlier fanout, with the state it reads then. A report
//! from a frozen copy ([`Snapshot`]) tells the state the copy holds and is
//! never handed on.

use super::super::binding_probes::after;
use super::super::command_runs::CommandRunner;
use super::super::cov_fanout::CovFanout;
use super::super::event_send::{Settle, settled, settled_by};
use super::*;
use crate::committed_cov::CommittedCov;
use std::collections::{HashMap, HashSet, VecDeque};

/// The most passes of one task whose COV fanout waits for their event
/// notifications at once (#2537); a pass past them fans out at once, its wait
/// counted in `cov_wait_expired`.
const MAX_WAITING_FANOUTS: usize = 64;

/// A COV report from a frozen copy of an object, taken under the pass's guard
/// (the operation task's, #1510).
pub(in crate::server) type Snapshot = (
    ObjectIdentifier,
    Box<dyn bacnet_objects::traits::BACnetObject>,
);

/// One pass's COV fanout, waiting its turn in [`FanoutTail::run`].
struct Pass {
    /// What it waits for first: its event notifications' hand-off, when it
    /// pushed any (#889).
    settle: Option<oneshot::Receiver<()>>,
    committed: CommittedCov,
    snapshots: Vec<Snapshot>,
    /// When that wait ends at the latest: the cap after the pass committed.
    deadline: tokio::time::Instant,
    /// The objects an earlier pass handed to this one ([`leave_to_later`]):
    /// this pass reports them itself, never handing them on again.
    handed_in: HashSet<ObjectIdentifier>,
}

/// One task's COV tail: the passes waiting behind the one [`Self::run`] is
/// on, in commit order, behind a lock never held across an await, and the
/// wake for a pass pushed.
pub(in crate::server) struct FanoutTail<'a, T: TransportPort + 'static> {
    fanout: &'a CovFanout<T>,
    /// Starts the Command runs a pass's writes queued once its fanout is made
    /// (the Schedule task's); without one they end unstarted.
    runner: Option<CommandRunner<T>>,
    /// How long a pass waits for its notifications: `cov_retry_timeout_ms`.
    cap: Duration,
    queue: std::sync::Mutex<VecDeque<Pass>>,
    pushed: tokio::sync::Notify,
}

impl<'a, T: TransportPort + 'static> FanoutTail<'a, T> {
    pub(in crate::server) fn new(
        fanout: &'a CovFanout<T>,
        runner: Option<CommandRunner<T>>,
    ) -> Self {
        let cap = Duration::from_millis(fanout.event_delivery().retry_timeout_ms);
        Self {
            fanout,
            runner,
            cap,
            queue: std::sync::Mutex::default(),
            pushed: tokio::sync::Notify::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<Pass>> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Hand the fanout of a pass that has just committed to the tail: what
    /// `committed` and `snapshots` report, behind the notifications counted
    /// in `settle`, its deadline stamped now. Never waits for a lane: past
    /// [`MAX_WAITING_FANOUTS`] it fans out at once, ahead of the passes still
    /// waiting, counted as a wait that ran out unless its notifications are
    /// out already.
    pub(in crate::server) async fn hand_off(
        &self,
        settle: Settle,
        committed: CommittedCov,
        snapshots: Vec<Snapshot>,
    ) {
        let settle = settle.into_wait();
        if settle.is_none() && committed.is_empty() && snapshots.is_empty() {
            return;
        }
        let pass = Pass {
            settle,
            committed,
            snapshots,
            deadline: after(tokio::time::Instant::now(), self.cap),
            handed_in: HashSet::new(),
        };
        let overflow = {
            let mut queue = self.lock();
            if queue.len() < MAX_WAITING_FANOUTS {
                queue.push_back(pass);
                None
            } else {
                Some(pass)
            }
        };
        match overflow {
            None => self.pushed.notify_one(),
            Some(mut pass) => {
                let transactions = &self.fanout.notification_transactions;
                settled(pass.settle.take(), 0, transactions).await;
                self.fire(pass).await;
            }
        }
    }

    /// Each pass's COV fanout, in the order the task made them, once that
    /// pass's event notifications are handed off or its deadline has passed
    /// (#889). Only a pass whose notifications went in time hands objects on.
    /// Nothing is held across the wait. Runs until the task ends.
    pub(in crate::server) async fn run(&self) {
        loop {
            let next = self.lock().pop_front();
            let Some(mut pass) = next else {
                self.pushed.notified().await;
                continue;
            };
            let settle = pass.settle.take();
            let transactions = &self.fanout.notification_transactions;
            if settled_by(settle, pass.deadline, transactions).await {
                leave_to_later(&mut pass, self);
            }
            self.fire(pass).await;
        }
    }

    /// `pass`'s COV fanout, then its frozen copies' reports, then the runs
    /// its writes queued.
    async fn fire(&self, pass: Pass) {
        let Pass {
            committed,
            snapshots,
            ..
        } = pass;
        self.fanout.fire(&committed).await;
        for (oid, snapshot) in &snapshots {
            let ctx = self.fanout.notify_context();
            BACnetServer::<T>::fire_cov_notifications_from_snapshot(&ctx, oid, snapshot.as_ref())
                .await;
        }
        if let Some(runner) = &self.runner {
            runner.start(committed.command_runs);
        }
    }
}

/// Leave out of `pass`'s fanout each object a pass queued behind it fans out
/// for too, unless an earlier pass handed the object to `pass`: its report
/// goes with the first such pass, which reports it itself. A Life Safety
/// object's changed properties go along. Frozen copies stay.
///
/// So an object is handed on once at most, and is reported at most two caps
/// after the commit that first wanted it, however often it changes: the
/// pass it goes to committed before `pass`'s deadline, and reports within a
/// cap of its own commit. The lock covers only the walks over the later
/// passes.
fn leave_to_later<T: TransportPort + 'static>(pass: &mut Pass, tail: &FanoutTail<'_, T>) {
    if tail.lock().is_empty() {
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
    for later in tail.lock().iter_mut() {
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
