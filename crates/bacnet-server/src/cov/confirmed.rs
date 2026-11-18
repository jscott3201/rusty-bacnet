//! The server's side of a confirmed COV report (#896): the worker's wake-up
//! for a fence, and the queues of references owed a fresh evaluation, now
//! or once an invoke ID comes back (#2661). The flight itself is plain
//! state in the table; `flight.rs` has the policy.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bacnet_device_core::cov::CovSubscriptionKey;
use tokio::sync::Notify;
use tokio::sync::futures::Notified;

use super::{AtomicCovCounters, Token};

/// The outstanding confirmed report of one notification, as its worker
/// holds it: the token its outcome goes back with
/// ([`CovSubscriptionTable::delivered`](super::CovSubscriptionTable::delivered))
/// and the wake-up a fence fires.
///
/// Dropping it ends nothing: a worker reports every outcome, and a report
/// whose worker was aborted ends itself at its deadline.
#[derive(Debug)]
pub(crate) struct ConfirmedFlight {
    token: Token,
    wake: Arc<Notify>,
}

impl ConfirmedFlight {
    pub(super) fn new(token: Token, wake: Arc<Notify>) -> Self {
        Self { token, wake }
    }

    /// The report's token.
    pub(crate) fn token(&self) -> Token {
        self.token
    }

    /// Complete once a fresh flight has replaced this report's, after a route
    /// change or a renewal, or once its deadline passed: retrying it would
    /// only land after the replacement's own report. Cancel-safe: a fence
    /// that came first leaves its permit for the next call.
    pub(crate) async fn fenced(&self) {
        self.wake.notified().await;
    }
}

/// References whose confirmed report was acknowledged or fenced off, and so need
/// evaluating again. The server drains them in a background task and fans them
/// out through the usual path, which reports only what differs from each
/// baseline.
///
/// Beside them wait the references whose confirmed report found no invoke
/// ID free (#2661), each looked at again once IDs come back, oldest first,
/// and only as many at a time as the IDs and in-flight slots free let go,
/// so a wait at a limit never spins.
#[derive(Debug, Default)]
pub(crate) struct CovRevisits {
    pending: Mutex<Pending>,
    wake: Notify,
    /// Wakes the wait for invoke IDs when a reference is owed one.
    owed_wake: Notify,
    counters: Arc<AtomicCovCounters>,
}

/// What [`CovRevisits`] holds under its one lock.
#[derive(Debug, Default)]
struct Pending {
    /// References owed a look now.
    now: HashSet<CovSubscriptionKey>,
    /// References owed a look once an invoke ID comes back.
    owed: OwedIds,
}

/// References whose confirmed report found no invoke ID free, in the order
/// each was first owed one (#2661). A reference holds one place however
/// often it is refused, and keeps it through a look that is refused again,
/// as when other work takes the freed ID first or its peer's in-flight
/// slots are full.
#[derive(Debug, Default)]
struct OwedIds {
    /// The references waiting, by place.
    queue: BTreeMap<u64, CovSubscriptionKey>,
    /// The place of each reference waiting or being looked at.
    places: HashMap<CovSubscriptionKey, u64>,
    next: u64,
}

impl OwedIds {
    /// Queue `key` in its place; whether it wasn't waiting already.
    fn owe(&mut self, key: CovSubscriptionKey) -> bool {
        let place = match self.places.get(&key) {
            Some(&place) => place,
            None => {
                let place = self.next;
                self.next += 1;
                self.places.insert(key.clone(), place);
                place
            }
        };
        match self.queue.entry(place) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(key);
                true
            }
            std::collections::btree_map::Entry::Occupied(_) => false,
        }
    }

    /// Queue `key` again in its place if it has one: it waits, or is being
    /// looked at. Whether it was queued.
    fn requeue(&mut self, key: CovSubscriptionKey) -> bool {
        self.places.contains_key(&key) && self.owe(key)
    }

    /// Take the references `admit` lets go, oldest first, for a look; each
    /// keeps its place until [`Self::looked`], and those it holds back
    /// keep theirs in the queue.
    fn take(
        &mut self,
        mut admit: impl FnMut(&CovSubscriptionKey) -> bool,
    ) -> Vec<CovSubscriptionKey> {
        let places: Vec<u64> = self
            .queue
            .iter()
            .filter(|(_, key)| admit(key))
            .map(|(place, _)| *place)
            .collect();
        places
            .iter()
            .filter_map(|place| self.queue.remove(place))
            .collect()
    }

    /// The look at `keys` is over: those it didn't owe an ID again give
    /// up their places.
    fn looked(&mut self, keys: &[CovSubscriptionKey]) {
        for key in keys {
            if let Some(place) = self.places.get(key)
                && !self.queue.contains_key(place)
            {
                self.places.remove(key);
            }
        }
    }

    /// Drop `key`'s wait or look; whether it was waiting.
    fn forget(&mut self, key: &CovSubscriptionKey) -> bool {
        self.places
            .remove(key)
            .is_some_and(|place| self.queue.remove(&place).is_some())
    }
}

/// Add `count` to `counter`, when there is anything to add.
fn count(counter: &AtomicU64, count: usize) {
    if count != 0 {
        counter.fetch_add(count as u64, Ordering::Relaxed);
    }
}

impl CovRevisits {
    /// Revisits counted in `counters`.
    pub(super) fn new(counters: Arc<AtomicCovCounters>) -> Self {
        Self {
            counters,
            ..Self::default()
        }
    }

    fn pending(&self) -> MutexGuard<'_, Pending> {
        // A panic while holding this lock leaves only keys and places.
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queue references for evaluation and wake the drain.
    pub(crate) fn request(&self, keys: impl IntoIterator<Item = CovSubscriptionKey>) {
        let mut keys = keys.into_iter().peekable();
        // Nothing to queue: neither the lock nor the drain's wake-up.
        if keys.peek().is_none() {
            return;
        }
        let mut pending = self.pending();
        let before = pending.now.len();
        pending.now.extend(keys);
        let added = pending.now.len() > before;
        drop(pending);
        if added {
            self.wake.notify_one();
        }
    }

    /// Wait for queued references and take them all.
    pub(crate) async fn next(&self) -> Vec<CovSubscriptionKey> {
        loop {
            let keys: Vec<_> = self.pending().now.drain().collect();
            if !keys.is_empty() {
                return keys;
            }
            self.wake.notified().await;
        }
    }

    /// Owe `keys`, whose confirmed report found no invoke ID free, a look
    /// once one comes back (#2661), and wake the wait for one. Whether any
    /// wasn't waiting already.
    pub(crate) fn owe(&self, keys: impl IntoIterator<Item = CovSubscriptionKey>) -> bool {
        self.queue(keys, OwedIds::owe)
    }

    /// Keep waiting those of `keys` that wait for an invoke ID already,
    /// whose look an in-flight limit refused: each in its place, looked at
    /// again once its peer has room. Others, a first send among them, are
    /// left as the limit left them.
    pub(crate) fn keep_waiting(&self, keys: impl IntoIterator<Item = CovSubscriptionKey>) {
        self.queue(keys, OwedIds::requeue);
    }

    fn queue(
        &self,
        keys: impl IntoIterator<Item = CovSubscriptionKey>,
        mut queue: impl FnMut(&mut OwedIds, CovSubscriptionKey) -> bool,
    ) -> bool {
        let mut pending = self.pending();
        let mut added = 0;
        for key in keys {
            added += usize::from(queue(&mut pending.owed, key));
        }
        drop(pending);
        count(&self.counters.owed_id_revisits_queued, added);
        if added != 0 {
            self.owed_wake.notify_one();
        }
        added != 0
    }

    /// Completes once a reference is owed an invoke ID after this is made,
    /// or at once if one was since the last wait.
    pub(crate) fn owed_notified(&self) -> Notified<'_> {
        self.owed_wake.notified()
    }

    /// Whether any reference waits for an invoke ID.
    pub(crate) fn owes_invoke_id(&self) -> bool {
        !self.pending().owed.queue.is_empty()
    }

    /// Take the references waiting for an invoke ID that `admit` lets go,
    /// oldest first, for a look; [`Self::looked`] ends it. Those held back
    /// keep their places.
    pub(crate) fn take_owed(
        &self,
        admit: impl FnMut(&CovSubscriptionKey) -> bool,
    ) -> Vec<CovSubscriptionKey> {
        let keys = self.pending().owed.take(admit);
        count(&self.counters.owed_id_revisits_delivered, keys.len());
        keys
    }

    /// The look at `keys`, which [`Self::take_owed`] gave, is over: those
    /// it owed an ID again wait in their old places; the rest leave.
    pub(crate) fn looked(&self, keys: &[CovSubscriptionKey]) {
        self.pending().owed.looked(keys);
    }

    /// The references waiting for an invoke ID, oldest first.
    #[cfg(test)]
    pub(crate) fn owed(&self) -> Vec<CovSubscriptionKey> {
        self.pending().owed.queue.values().cloned().collect()
    }

    /// Queued references, without taking them.
    #[cfg(test)]
    pub(crate) fn queued(&self) -> HashSet<CovSubscriptionKey> {
        self.pending().now.clone()
    }

    /// Drop a removed reference's pending evaluation, now or for an ID.
    pub(super) fn forget(&self, key: &CovSubscriptionKey) {
        let mut pending = self.pending();
        pending.now.remove(key);
        let dropped = pending.owed.forget(key);
        drop(pending);
        count(
            &self.counters.owed_id_revisits_dropped,
            usize::from(dropped),
        );
    }

    /// Drop the waits for an invoke ID of references a subscription just
    /// replaced: each reports for itself.
    pub(super) fn forget_owed<'a>(&self, keys: impl IntoIterator<Item = &'a CovSubscriptionKey>) {
        let mut pending = self.pending();
        if pending.owed.places.is_empty() {
            return;
        }
        let dropped = keys
            .into_iter()
            .filter(|key| pending.owed.forget(key))
            .count();
        drop(pending);
        count(&self.counters.owed_id_revisits_dropped, dropped);
    }
}

#[cfg(test)]
mod tests;
