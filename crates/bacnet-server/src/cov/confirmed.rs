//! The server's side of a confirmed COV report (#896): the worker's wake-up
//! for a fence, and the queues of references owed a fresh evaluation, now
//! or once there is room for their report: an invoke ID (#2661), a slot of
//! the in-flight pool and one of their peer's in-flight slots (#2671). The
//! flight itself is plain state in the table; `flight.rs` has the policy.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use bacnet_device_core::cov::{CovSubscriptionKey, MultipleContextKey};
use bacnet_types::sync::MutexExt;
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
/// Beside them wait the references whose confirmed report found no room:
/// no invoke ID free (#2661), or its peer at its in-flight limit or the
/// pool of in-flight slots full (#2671). Each is looked at again once room
/// comes back, oldest first, and only as many at a time as the room free
/// lets go, so a wait at a limit never spins.
#[derive(Debug, Default)]
pub(crate) struct CovRevisits {
    pending: Mutex<Pending>,
    wake: Notify,
    /// Wakes the wait for room when a reference is owed a look.
    owed_wake: Notify,
    /// How many references hold a place, waiting or being looked at,
    /// stored under the lock after each change. A caller holding the
    /// table's guard reads it to skip the lock while none does: every place
    /// is made under a table guard, so a zero read there is never stale.
    holding: AtomicUsize,
    counters: Arc<AtomicCovCounters>,
}

/// What [`CovRevisits`] holds under its one lock.
#[derive(Debug, Default)]
struct Pending {
    /// References owed a look now.
    now: HashSet<CovSubscriptionKey>,
    /// References owed a look once there is room.
    owed: Owed,
}

/// References whose confirmed report found no room, in the order each was
/// first owed a look. A reference holds one place however often it is
/// refused, whichever limit refused it, so they are at most as many as the
/// subscriptions; and it keeps its place through a look that is refused
/// again, as when other work takes the freed room first.
#[derive(Debug, Default)]
struct Owed {
    /// The references waiting, by place.
    queue: BTreeMap<u64, CovSubscriptionKey>,
    /// The place of each reference waiting or being looked at.
    places: HashMap<CovSubscriptionKey, u64>,
    next: u64,
}

impl Owed {
    /// Queue `key` in its place: its old one while it waits or is being
    /// looked at, else a new one at the back. Whether it wasn't waiting
    /// already.
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

    /// The look at `keys` is over: those it didn't queue again give up
    /// their places. Whether it queued any again.
    fn looked(&mut self, keys: &[CovSubscriptionKey]) -> bool {
        let mut again = false;
        for key in keys {
            if let Some(place) = self.places.get(key) {
                if self.queue.contains_key(place) {
                    again = true;
                } else {
                    self.places.remove(key);
                }
            }
        }
        again
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
        self.pending.locked()
    }

    /// Store how many references hold a place, under the lock.
    fn publish(&self, pending: &Pending) {
        self.holding
            .store(pending.owed.places.len(), Ordering::Release);
    }

    /// Whether any reference may hold a place; see [`Self::holding`].
    fn any_holding(&self) -> bool {
        self.holding.load(Ordering::Acquire) != 0
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

    /// Owe `keys`, whose confirmed report found no room (an invoke ID, a
    /// slot of the pool or one of its peer's), a look once there is some,
    /// and wake the wait for room. A reference already waiting, or being
    /// looked at, keeps its place. Whether any wasn't waiting already.
    pub(crate) fn owe(&self, keys: impl IntoIterator<Item = CovSubscriptionKey>) -> bool {
        let mut pending = self.pending();
        let mut added = 0;
        for key in keys {
            added += usize::from(pending.owed.owe(key));
        }
        self.publish(&pending);
        drop(pending);
        count(&self.counters.owed_revisits_queued, added);
        if added != 0 {
            self.owed_wake.notify_one();
        }
        added != 0
    }

    /// Completes once a reference is owed a look after this is made, or at
    /// once if one was since the last wait.
    pub(crate) fn owed_notified(&self) -> Notified<'_> {
        self.owed_wake.notified()
    }

    /// Whether any reference waits for room.
    pub(crate) fn owes_any(&self) -> bool {
        !self.pending().owed.queue.is_empty()
    }

    /// Take the references waiting for room that `admit` lets go, oldest
    /// first, for a look; [`Self::looked`] ends it. Those held back keep
    /// their places.
    pub(crate) fn take_owed(
        &self,
        admit: impl FnMut(&CovSubscriptionKey) -> bool,
    ) -> Vec<CovSubscriptionKey> {
        let keys = self.pending().owed.take(admit);
        count(&self.counters.owed_revisits_delivered, keys.len());
        keys
    }

    /// The look at `keys`, which [`Self::take_owed`] gave, is over: those
    /// it owed again, refused for want of room, wait in their old places;
    /// the rest leave. Whether any was owed again.
    pub(crate) fn looked(&self, keys: &[CovSubscriptionKey]) -> bool {
        let mut pending = self.pending();
        let again = pending.owed.looked(keys);
        self.publish(&pending);
        again
    }

    /// Confirmed reports began for `keys` without their looks, a fresh
    /// fanout or a follow-up having found room first: their waits end, and
    /// each report's Ack owes its coordinate the next look (#2671). Without
    /// this a stale place would spend a pass's room and could jump a later
    /// waiter. Neither work nor the lock while no reference holds a place.
    pub(crate) fn sent<'a>(&self, keys: impl IntoIterator<Item = &'a CovSubscriptionKey>) {
        if !self.any_holding() {
            return;
        }
        let mut pending = self.pending();
        let left = keys
            .into_iter()
            .filter(|key| pending.owed.forget(key))
            .count();
        self.publish(&pending);
        drop(pending);
        count(&self.counters.owed_revisits_delivered, left);
    }

    /// [`Self::sent`] for every reference of `context`, whose report
    /// began: one coordinate, so its Ack owes them all the next look. The
    /// places are scanned, not the context's references listed, and only
    /// while some reference holds one.
    pub(crate) fn context_sent(&self, context: &MultipleContextKey) {
        if !self.any_holding() {
            return;
        }
        let mut pending = self.pending();
        let keys: Vec<_> = pending
            .owed
            .places
            .keys()
            .filter(|key| key.multiple_context() == Some(context))
            .cloned()
            .collect();
        let left = keys.iter().filter(|key| pending.owed.forget(key)).count();
        self.publish(&pending);
        drop(pending);
        count(&self.counters.owed_revisits_delivered, left);
    }

    /// The references waiting for room, oldest first.
    #[cfg(test)]
    pub(crate) fn owed(&self) -> Vec<CovSubscriptionKey> {
        self.pending().owed.queue.values().cloned().collect()
    }

    /// How many references hold a place, waiting or being looked at.
    #[cfg(test)]
    pub(crate) fn owed_places(&self) -> usize {
        self.pending().owed.places.len()
    }

    /// Queued references, without taking them.
    #[cfg(test)]
    pub(crate) fn queued(&self) -> HashSet<CovSubscriptionKey> {
        self.pending().now.clone()
    }

    /// Drop a removed reference's pending evaluation, now or for room.
    pub(super) fn forget(&self, key: &CovSubscriptionKey) {
        let mut pending = self.pending();
        pending.now.remove(key);
        let dropped = pending.owed.forget(key);
        self.publish(&pending);
        drop(pending);
        count(&self.counters.owed_revisits_dropped, usize::from(dropped));
    }

    /// Drop the waits for room of references a subscription just replaced:
    /// each reports for itself.
    pub(super) fn forget_owed<'a>(&self, keys: impl IntoIterator<Item = &'a CovSubscriptionKey>) {
        // Every subscription comes here: no lock while none waits.
        if !self.any_holding() {
            return;
        }
        let mut pending = self.pending();
        let dropped = keys
            .into_iter()
            .filter(|key| pending.owed.forget(key))
            .count();
        self.publish(&pending);
        drop(pending);
        count(&self.counters.owed_revisits_dropped, dropped);
    }

    /// The server stopped and nothing will look again: drop every pending
    /// evaluation, now or for room, and count the waits dropped.
    pub(crate) fn clear(&self) {
        let mut pending = self.pending();
        pending.now.clear();
        let dropped = pending.owed.queue.len();
        pending.owed = Owed::default();
        self.publish(&pending);
        drop(pending);
        count(&self.counters.owed_revisits_dropped, dropped);
    }
}

#[cfg(test)]
mod tests;
