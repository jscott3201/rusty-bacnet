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
//! never handed on; once a later pass's copy of an object has gone out (one
//! past the bound goes at once), an earlier pass's copy of it is dropped, so
//! a subscriber never hears an object step back ([`Copies`]).
//!
//! The Command runs a pass's writes queued start as it is handed off, as a
//! write's runs start, not behind the pass's notifications.

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
    /// Its place in the order the task made its passes ([`Copies`]).
    seq: u64,
}

/// The order frozen copies are reported in: a pass past the bound fans out
/// ahead of earlier passes still waiting, so each object keeps the newest
/// pass whose copy of it has gone out, and an earlier pass's copy is
/// dropped. Only objects with a copy in a pass not yet fired are kept.
#[derive(Default)]
struct Copies {
    /// The next pass's place in the order.
    next: u64,
    /// Per object: how many passes not yet fired hold a copy of it, and the
    /// latest pass whose copy of it went out.
    objects: HashMap<ObjectIdentifier, (usize, Option<u64>)>,
}

impl Copies {
    /// Number the next pass, counting the copies it holds.
    fn number(&mut self, snapshots: &[Snapshot]) -> u64 {
        let seq = self.next;
        self.next += 1;
        for (oid, _) in snapshots {
            self.objects.entry(*oid).or_default().0 += 1;
        }
        seq
    }

    /// Whether pass `seq`'s copy of `oid` goes out: not once a later pass's
    /// has. Either way that copy no longer waits.
    fn turn(&mut self, oid: ObjectIdentifier, seq: u64) -> bool {
        let Some((waiting, sent)) = self.objects.get_mut(&oid) else {
            return true;
        };
        let goes = sent.is_none_or(|sent| sent < seq);
        if goes {
            *sent = Some(seq);
        }
        *waiting -= 1;
        if *waiting == 0 {
            self.objects.remove(&oid);
        }
        goes
    }
}

/// One task's COV tail: the passes waiting behind the one [`Self::run`] is
/// on, in commit order, behind a lock never held across an await, and the
/// wake for a pass pushed.
pub(in crate::server) struct FanoutTail<'a, T: TransportPort + 'static> {
    fanout: &'a CovFanout<T>,
    /// Starts the Command runs a pass's writes queued as it is handed off
    /// (the Schedule task's); without one they end unstarted.
    runner: Option<CommandRunner<T>>,
    /// How long a pass waits for its notifications: `cov_retry_timeout_ms`.
    cap: Duration,
    queue: std::sync::Mutex<VecDeque<Pass>>,
    pushed: tokio::sync::Notify,
    copies: std::sync::Mutex<Copies>,
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
            copies: std::sync::Mutex::default(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<Pass>> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn copies(&self) -> std::sync::MutexGuard<'_, Copies> {
        self.copies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Hand the fanout of a pass that has just committed to the tail: what
    /// `committed` and `snapshots` report, behind the notifications counted
    /// in `settle`, its deadline stamped now. The Command runs its writes
    /// queued start now. Never waits for a lane: past
    /// [`MAX_WAITING_FANOUTS`] it fans out at once, ahead of the passes still
    /// waiting, counted as a wait that ran out unless its notifications are
    /// out already.
    pub(in crate::server) async fn hand_off(
        &self,
        settle: Settle,
        mut committed: CommittedCov,
        snapshots: Vec<Snapshot>,
    ) {
        let runs = std::mem::take(&mut committed.command_runs);
        if let Some(runner) = &self.runner {
            runner.start(runs);
        }
        let settle = settle.into_wait();
        if settle.is_none() && committed.is_empty() && snapshots.is_empty() {
            return;
        }
        let pass = Pass {
            settle,
            committed,
            seq: self.copies().number(&snapshots),
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

    /// `pass`'s COV fanout, then the reports of its frozen copies no later
    /// pass's copy has overtaken.
    async fn fire(&self, pass: Pass) {
        let Pass {
            committed,
            snapshots,
            seq,
            ..
        } = pass;
        self.fanout.fire(&committed).await;
        for (oid, snapshot) in &snapshots {
            if !self.copies().turn(*oid, seq) {
                continue;
            }
            let ctx = self.fanout.notify_context();
            BACnetServer::<T>::fire_cov_notifications_from_snapshot(&ctx, oid, snapshot.as_ref())
                .await;
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
