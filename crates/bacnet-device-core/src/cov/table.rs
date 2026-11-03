//! The COV subscription table: entries keyed by the monitored object first,
//! the per-peer accounting, and what each mutation removed or fenced.
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::cov::{
    CovCounters, CovPolicy, CovRecipient, CovSample, CovSubscriptionKey, MultipleContextKey,
    SubscriberEndpoint,
};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_time::{Instant, earliest};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

use super::flight::{FlightState, FlightTiming, Token};
use super::observation_order::Pending;
use super::vec_map::VecMap;
use super::{CovSubscription, CovSubscriptionSnapshot};

/// Gives each table its own identity, so a snapshot from one table can't
/// complete another's entry.
static NEXT_TABLE: AtomicUsize = AtomicUsize::new(0);

/// What table mutations ended, replaced or removed since the host last took
/// them ([`CovTable::take_fenced`]). A host that keeps wake-ups or histories
/// outside the table acts on exactly these.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Fenced {
    /// Outstanding confirmed reports whose flight a renewal or a route
    /// change replaced, or whose deadline passed: their workers stop.
    pub tokens: Vec<Token>,
    /// References that left the table: cancelled, expired, or removed with
    /// their object or route.
    pub removed: Vec<CovSubscriptionKey>,
    /// References a fence or an abandoned report left owing a fresh
    /// evaluation (#896, #923).
    pub revisit: Vec<CovSubscriptionKey>,
    /// Multiple contexts an admission refreshed: their lifetime, maximum
    /// notification delay and subscriber APDU size may have changed.
    pub contexts: Vec<MultipleContextKey>,
}

impl Fenced {
    /// Whether it names nothing to act on.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
            && self.removed.is_empty()
            && self.revisit.is_empty()
            && self.contexts.is_empty()
    }
}

/// The entries in one vector sorted by their key, which puts the monitored
/// object first, so an object's entries are one run found by binary search.
/// Measured against a `BTreeMap` for #2303 (design §11): smaller in an
/// image and faster on the fanout's lookup and on admission, slower only
/// on removal, which a cancellation or a deleted object makes.
#[derive(Debug, Default)]
pub(super) struct Entries(Vec<CovSubscriptionSnapshot>);

impl Entries {
    fn find(&self, key: &CovSubscriptionKey) -> Result<usize, usize> {
        self.0.binary_search_by(|entry| entry.key.cmp(key))
    }

    pub(super) fn get(&self, key: &CovSubscriptionKey) -> Option<&CovSubscriptionSnapshot> {
        self.find(key).ok().map(|at| &self.0[at])
    }

    pub(super) fn get_mut(
        &mut self,
        key: &CovSubscriptionKey,
    ) -> Option<&mut CovSubscriptionSnapshot> {
        self.find(key).ok().map(|at| &mut self.0[at])
    }

    pub(super) fn contains(&self, key: &CovSubscriptionKey) -> bool {
        self.find(key).is_ok()
    }

    /// Insert under the snapshot's own key, returning the entry it replaced.
    pub(super) fn insert(
        &mut self,
        snapshot: CovSubscriptionSnapshot,
    ) -> Option<CovSubscriptionSnapshot> {
        match self.find(&snapshot.key) {
            Ok(at) => Some(core::mem::replace(&mut self.0[at], snapshot)),
            Err(at) => {
                self.0.insert(at, snapshot);
                None
            }
        }
    }

    pub(super) fn remove(&mut self, key: &CovSubscriptionKey) -> Option<CovSubscriptionSnapshot> {
        self.find(key).ok().map(|at| self.0.remove(at))
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    pub(super) fn values(&self) -> impl Iterator<Item = &CovSubscriptionSnapshot> {
        self.0.iter()
    }

    pub(super) fn values_mut(&mut self) -> impl Iterator<Item = &mut CovSubscriptionSnapshot> {
        self.0.iter_mut()
    }

    /// The entries monitoring `oid`, found without visiting the others.
    pub(super) fn of_object(
        &self,
        oid: ObjectIdentifier,
    ) -> impl Iterator<Item = &CovSubscriptionSnapshot> {
        let start = self.0.partition_point(|entry| entry.key.object() < oid);
        self.0[start..]
            .iter()
            .take_while(move |entry| entry.key.object() == oid)
    }
}

/// Table of active COV subscriptions, with their confirmed flights and
/// completion tickets as plain state. It takes no lock and reads no clock:
/// the host holds it under its own guard and passes `now`, on one monotonic
/// timeline, to every call that can expire something.
#[derive(Debug)]
pub struct CovTable {
    pub(super) id: usize,
    pub(super) subs: Entries,
    pub(super) generation: u64,
    /// The last completion ticket issued; tickets start at one.
    pub(super) issued: u64,
    /// Flights with an outstanding report or a hold-off; an absent flight
    /// is idle.
    pub(super) flights: VecMap<u64, FlightState>,
    /// Unconfirmed reports handed to the host whose sends haven't ended.
    pub(super) pending: Vec<Pending>,
    pub(super) next_flight: u64,
    pub(super) peer_counts: VecMap<CovRecipient, usize>,
    pub(super) peer_indefinite_counts: VecMap<CovRecipient, usize>,
    pub(super) policy: CovPolicy,
    pub(super) timing: FlightTiming,
    /// Subscription counts since the host last took them.
    pub(super) counts: CovCounters,
    pub(super) fenced: Fenced,
    /// No subscription expires, and no outstanding report reaches its
    /// deadline, before this instant; `None` when nothing can. Lowered as
    /// each lifetime or deadline is set, left as it is by a removal (so it
    /// may be early, never late), and recomputed by each full expiry pass.
    pub(super) next_due: Option<Instant>,
}

impl Default for CovTable {
    fn default() -> Self {
        Self::new(CovPolicy::default())
    }
}

impl CovTable {
    /// An empty table under `policy`, sanitized.
    pub fn new(policy: CovPolicy) -> Self {
        Self {
            id: NEXT_TABLE.fetch_add(1, Ordering::Relaxed),
            subs: Entries::default(),
            generation: 0,
            issued: 0,
            flights: VecMap::default(),
            pending: Vec::new(),
            next_flight: 0,
            peer_counts: VecMap::default(),
            peer_indefinite_counts: VecMap::default(),
            policy: policy.sanitized(),
            timing: FlightTiming::default(),
            counts: CovCounters::default(),
            fenced: Fenced::default(),
            next_due: None,
        }
    }

    /// This table with confirmed reports timed by `timing`: their deadline
    /// and the hold-off after a failure.
    pub fn with_flight_timing(mut self, timing: FlightTiming) -> Self {
        self.timing = timing;
        self
    }

    /// The active COV policy.
    pub fn policy(&self) -> &CovPolicy {
        &self.policy
    }

    /// How confirmed reports are timed.
    pub fn flight_timing(&self) -> FlightTiming {
        self.timing
    }

    /// Counts since the last call: subscriptions created, cancelled, purged
    /// and refused, and notifications [`evaluate`](super::evaluate()) cut
    /// off at their input's budget. `subscriptions_active` is the live
    /// count; the other notification fields stay zero.
    pub fn take_counts(&mut self) -> CovCounters {
        let mut counts = core::mem::take(&mut self.counts);
        counts.subscriptions_active = self.subs.len() as u64;
        counts
    }

    /// What mutations removed, replaced or fenced since the last call.
    pub fn take_fenced(&mut self) -> Fenced {
        core::mem::take(&mut self.fenced)
    }

    /// Get the number of active subscriptions for a peer.
    pub fn peer_subscription_count(&self, peer: &CovRecipient) -> usize {
        self.peer_counts.get(peer).copied().unwrap_or(0)
    }

    /// Get the number of active indefinite subscriptions for a peer.
    pub fn peer_indefinite_count(&self, peer: &CovRecipient) -> usize {
        self.peer_indefinite_counts.get(peer).copied().unwrap_or(0)
    }

    /// Get an accepted entry by its complete typed identity.
    pub fn get_subscription(&self, key: &CovSubscriptionKey) -> Option<&CovSubscriptionSnapshot> {
        self.subs.get(key)
    }

    /// Every accepted entry, ordered by monitored object.
    pub fn entries(&self) -> impl Iterator<Item = &CovSubscriptionSnapshot> {
        self.subs.values()
    }

    /// The accepted entries monitoring `oid`, expired ones included.
    pub fn object_entries(
        &self,
        oid: ObjectIdentifier,
    ) -> impl Iterator<Item = &CovSubscriptionSnapshot> {
        self.subs.of_object(oid)
    }

    /// Every accepted reference of one Multiple context.
    pub fn multiple_context_references<'a>(
        &'a self,
        context: &'a MultipleContextKey,
    ) -> impl Iterator<Item = &'a CovSubscriptionSnapshot> + 'a {
        self.subs
            .values()
            .filter(move |sub| sub.key.multiple_context() == Some(context))
    }

    /// Whether an exact subscription identity is present.
    pub fn contains(&self, key: &CovSubscriptionKey) -> bool {
        self.subs.contains(key)
    }

    fn remove_internal(&mut self, key: &CovSubscriptionKey, was_cancelled: bool) -> bool {
        let Some(sub) = self.subs.remove(key) else {
            return false;
        };
        self.fenced.removed.push(key.clone());
        // A removed entry's flight ends without a fence, as dropping its
        // marker did: an outstanding report runs on, and its outcome no
        // longer counts. A context's flight goes with its last reference.
        match key.multiple_context() {
            Some(context) if self.multiple_context_references(context).next().is_some() => {}
            _ => {
                self.flights.remove(&sub.flight);
            }
        }
        let peer = sub.recipient();
        if let Some(count) = self.peer_counts.get_mut(&peer) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.peer_counts.remove(&peer);
            }
        }
        if sub.expires_at.is_none()
            && let Some(count) = self.peer_indefinite_counts.get_mut(&peer)
        {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.peer_indefinite_counts.remove(&peer);
            }
        }
        if was_cancelled {
            self.counts.subscriptions_cancelled += 1;
        }
        true
    }

    /// Cancel only the exact family/form/property/index identity.
    pub fn unsubscribe(&mut self, key: &CovSubscriptionKey) -> bool {
        self.remove_internal(key, true)
    }

    /// Cancel every reference in the exact Multiple context.
    pub fn unsubscribe_cov_multiple_context(&mut self, context: &MultipleContextKey) {
        let keys: Vec<_> = self
            .multiple_context_references(context)
            .map(|sub| sub.key.clone())
            .collect();
        for key in keys {
            self.remove_internal(&key, true);
        }
    }

    /// Remove all subscriptions for a given object (used on DeleteObject).
    pub fn remove_for_object(&mut self, oid: ObjectIdentifier) {
        let to_remove: Vec<_> = self
            .subs
            .of_object(oid)
            .map(|sub| sub.key.clone())
            .collect();
        for key in to_remove {
            self.remove_internal(&key, false);
        }
    }

    /// Remove subscriptions using this current route and release shared peer quotas.
    /// An obsolete router cannot remove a subscription migrated elsewhere.
    pub fn remove_peer_subscriptions(
        &mut self,
        mac: &[u8],
        network: Option<&NpduAddress>,
    ) -> usize {
        let target = SubscriberEndpoint::new(mac, network);
        let to_remove: Vec<_> = self
            .subs
            .values()
            .filter(|entry| entry.endpoint() == target)
            .map(|entry| entry.key.clone())
            .collect();
        let count = to_remove.len();
        for key in to_remove {
            self.remove_internal(&key, true);
        }
        count
    }

    /// Remove every subscription expired at `now`, and end every confirmed
    /// report whose deadline has passed (its worker never reported; its
    /// coordinate's references are owed a fresh look). Returns the number of
    /// subscriptions removed.
    pub fn expire(&mut self, now: Instant) -> usize {
        // Nothing can be due yet: the pass would find nothing.
        if self.next_due.is_none_or(|due| now < due) {
            return 0;
        }
        self.end_abandoned(now);
        let mut next_due = None;
        let mut to_remove = Vec::new();
        for sub in self.subs.values() {
            match sub.expires_at {
                Some(expires) if expires <= now => to_remove.push(sub.key.clone()),
                expires => next_due = earliest([next_due, expires]),
            }
        }
        let mut purged = 0;
        for key in to_remove {
            purged += usize::from(self.remove_internal(&key, false));
        }
        self.counts.subscriptions_purged += purged as u64;
        // Outstanding reports here are all before their deadlines.
        for state in self.flights.values() {
            next_due = earliest([next_due, state.outstanding_deadline()]);
        }
        self.next_due = next_due;
        purged
    }

    /// The earliest instant [`expire`](Self::expire) can find something to
    /// remove or end: a subscription's expiry or an outstanding report's
    /// deadline. `None` when nothing can expire. It may be earlier than
    /// needed after a removal, never later, so a host may skip both its
    /// clock and the pass until then.
    pub fn next_expiry(&self) -> Option<Instant> {
        self.next_due
    }

    /// Note a lifetime or deadline just set: nothing can be due before
    /// the earliest of them.
    pub(super) fn due_by(&mut self, at: Option<Instant>) {
        self.next_due = earliest([self.next_due, at]);
    }

    /// End every subscription's lifetime at `now`, so the next lookup or
    /// [`expire`](Self::expire) purges them all. Clause 13.1 lets a COV
    /// server lose its subscriptions when it restarts; a host can drop them
    /// this way.
    pub fn expire_all(&mut self, now: Instant) {
        for entry in self.subs.values_mut() {
            entry.subscription.expires_at = Some(now);
        }
        if self.subs.len() > 0 {
            self.due_by(Some(now));
        }
    }

    /// Get all active (non-expired) subscriptions for a given object,
    /// purging the expired ones first.
    pub fn subscriptions_for(
        &mut self,
        oid: &ObjectIdentifier,
        now: Instant,
    ) -> Vec<&CovSubscriptionSnapshot> {
        self.expire(now);
        self.subs.of_object(*oid).collect()
    }

    /// The finest COV increment each object's live subscriptions to
    /// `property` ask for (#1510): SubscribeCOVProperty subscriptions and
    /// Multiple references that give one. A negative increment reports any
    /// change, as zero does, so it counts as zero; a NaN or infinite one
    /// reports nothing finer, so it's left out.
    pub fn finest_increments(
        &self,
        property: PropertyIdentifier,
        now: Instant,
    ) -> BTreeMap<ObjectIdentifier, f64> {
        let mut finest = BTreeMap::new();
        for sub in self.subs.values() {
            if sub.monitored_property != Some(property)
                || sub.expires_at.is_some_and(|expires| expires <= now)
            {
                continue;
            }
            let Some(increment) = sub
                .cov_increment
                .map(f64::from)
                .filter(|increment| increment.is_finite())
            else {
                continue;
            };
            let increment = increment.max(0.0);
            finest
                .entry(sub.monitored_object_identifier)
                .and_modify(|finest: &mut f64| *finest = finest.min(increment))
                .or_insert(increment);
        }
        finest
    }

    /// Whether a snapshot still owns a live entry in this table at `now`.
    pub fn is_current(&self, snapshot: &CovSubscriptionSnapshot, now: Instant) -> bool {
        self.remaining_lifetime(snapshot, now)
            .and_then(super::CovTimeRemaining::wire_seconds)
            .is_some()
    }

    /// Ordinary whole-object trigger policy: a numeric Present_Value must move
    /// by the increment, any other value must change, and the first report
    /// always fires. Status_Flags and other Table 13-1 trigger values (such as
    /// Staging's Present_Stage) are checked separately by the caller.
    /// An unchanged object therefore reports nothing, however often it is
    /// fanned out. Property subscriptions compare their prepared sample instead.
    pub fn should_notify(
        sub: &CovSubscription,
        current_value: Option<&CovSample>,
        cov_increment: Option<f64>,
    ) -> bool {
        let Some(current) = current_value else {
            return true;
        };
        current.reports(
            sub.last_notified_observation
                .as_ref()
                .map(|observation| observation.sample()),
            cov_increment,
            true,
        )
    }

    /// The last generation the table issued. A test hook, behind the
    /// `test-support` feature: hosts' table tests reach generation
    /// exhaustion through it and
    /// [`set_generation_for_test`](Self::set_generation_for_test).
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn generation_for_test(&self) -> u64 {
        self.generation
    }

    /// Make `generation` the last generation issued. A test hook, see
    /// [`generation_for_test`](Self::generation_for_test).
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn set_generation_for_test(&mut self, generation: u64) {
        self.generation = generation;
    }

    /// Number of active subscriptions.
    pub fn len(&self) -> usize {
        self.subs.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.subs.len() == 0
    }
}

#[cfg(test)]
mod due_tests;
#[cfg(test)]
mod tests;
