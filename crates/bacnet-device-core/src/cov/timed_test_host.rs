//! A host for the histories' tests: a hand-set clock, the histories behind a
//! shared cell, and a claim that settles itself when dropped.
//!
//! The histories settle a claim only when told and read no clock. Most of
//! their tests were written against a host that does both for them, and
//! keep their bodies: here [`TimedStore::lock`] stands for the host's lock,
//! and dropping a [`TimedClaim`] gives it back at [`now`], as a host's
//! guard would.

use std::cell::{Cell, RefCell, RefMut};
use std::collections::HashSet;
use std::rc::Rc;

use bacnet_time::{Duration, Instant};

use super::{CovSubscriptionKey, TimedChange, TimedHistories, ValueFit};

thread_local! {
    /// This test's clock: a thread runs one test.
    static NOW: Cell<Instant> = const { Cell::new(Instant::from_secs(1_000)) };
}

/// Now, on the test's hand-set clock.
pub(super) fn now() -> Instant {
    NOW.get()
}

/// Step the test's clock by `by`.
pub(super) fn advance(by: Duration) {
    NOW.set(now() + by);
}

/// The histories as a host shares them between a fanout and its claims.
#[derive(Clone)]
pub(super) struct TimedStore(Rc<RefCell<TimedHistories>>);

impl TimedStore {
    pub(super) fn new(local_apdu: usize) -> Self {
        Self(Rc::new(RefCell::new(TimedHistories::new(local_apdu))))
    }

    /// The histories, for one statement or until the guard drops.
    pub(super) fn lock(&self) -> RefMut<'_, TimedHistories> {
        self.0.borrow_mut()
    }
}

/// A claim of `store`'s histories that is given back when dropped, unless
/// it was committed.
pub(super) struct TimedClaim {
    store: TimedStore,
    claim: super::TimedClaim,
}

impl TimedClaim {
    pub(super) fn new(store: TimedStore) -> Self {
        Self {
            store,
            claim: super::TimedClaim::new(),
        }
    }

    fn wrap(&self, claim: super::TimedClaim) -> Self {
        Self {
            store: self.store.clone(),
            claim,
        }
    }

    fn take(&mut self) -> super::TimedClaim {
        std::mem::take(&mut self.claim)
    }

    pub(super) fn add_untimed(
        &mut self,
        key: CovSubscriptionKey,
        generation: u64,
        owed_since: Option<Instant>,
    ) {
        self.claim.add_untimed(key, generation, owed_since);
    }

    pub(super) fn split_untimed(&mut self, keys: &HashSet<CovSubscriptionKey>) -> Self {
        let part = self.claim.split_untimed(|key| keys.contains(key));
        self.wrap(part)
    }

    pub(super) fn forgo_untimed(&mut self) {
        self.store.lock().forgo_untimed(&mut self.claim);
    }

    pub(super) fn without_eviction(mut self) -> Self {
        self.claim = self.take().without_eviction();
        self
    }

    pub(super) fn owing(mut self) -> Self {
        self.claim = self.take().owing();
        self
    }

    pub(super) fn add(
        &mut self,
        key: CovSubscriptionKey,
        incarnation: u64,
        changes: Vec<TimedChange>,
    ) {
        self.claim.add(key, incarnation, changes);
    }

    pub(super) fn split_oldest(&mut self, count: usize) -> Self {
        let part = self.claim.split_oldest(count);
        self.wrap(part)
    }

    pub(super) fn split_values(
        mut self,
        fit: impl Fn(&CovSubscriptionKey, &TimedChange) -> ValueFit,
    ) -> Vec<Self> {
        let (parts, lost) = self.take().split_values(fit);
        self.store.lock().lost_values(&lost);
        parts.into_iter().map(|part| self.wrap(part)).collect()
    }

    pub(super) fn in_order(&self) -> Vec<(&CovSubscriptionKey, &TimedChange)> {
        self.claim.in_order()
    }

    pub(super) fn last_changes(&self) -> impl Iterator<Item = (&CovSubscriptionKey, &TimedChange)> {
        self.claim.last_changes()
    }

    pub(super) fn commit(mut self) {
        let claim = self.take();
        self.store.lock().commit(claim);
    }

    pub(super) fn going_out(&self) {
        self.store.lock().going_out(&self.claim);
    }
}

impl Drop for TimedClaim {
    fn drop(&mut self) {
        let claim = self.take();
        self.store.lock().give_back(claim, now());
    }
}
