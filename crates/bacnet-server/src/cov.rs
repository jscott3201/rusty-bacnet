//! COV subscription engine — tracks SubscribeCOV subscriptions and their lifetimes.
//!
//! [`CovTable`] holds every subscription, flight and ticket as plain state.
//! [`CovSubscriptionTable`] is the server's table: the plain one inside the
//! server's lock, and beside it what only a host keeps, which the plain
//! table's [`Fenced`](bacnet_device_core::cov::Fenced) reports drive: the
//! confirmed workers' wake-ups, the revisit queue, the timestamped histories
//! and the telemetry atomics.

use std::collections::BTreeMap;
use std::sync::Arc;

use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::Notify;

use crate::runtime_clock;
use bacnet_device_core::cov::{
    BeginRefusal, CovObservation, CovPolicy, CovRecipient, CovSample, CovSubscription,
    CovSubscriptionKey, CovSubscriptionSnapshot, CovTable, CovTimeRemaining, Delivery,
    FlightTiming, MultipleContextKey, MultipleRefusal, PreparedCovCompletion, SubscriberEndpoint,
    Token,
};
use bacnet_objects::database::ObjectDatabase;

pub(crate) mod active;
mod confirmed;
pub(crate) use confirmed::{ConfirmedFlight, CovRevisits};
pub(crate) mod flags;
pub(crate) mod multiple_reads;
pub(crate) mod prepare;
pub(crate) mod reported;
pub(crate) mod timed;
mod timed_capture;
pub(crate) use timed_capture::TimedWriteCapture;
pub(crate) mod value_source;

mod policy;
pub use policy::*;

#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod policy_tests;
#[cfg(test)]
mod tests;

/// Largest B/IP APDU; the default history bound until a server sets its own.
const DEFAULT_TIMED_APDU_LENGTH: usize = 1476;

/// Table of active COV subscriptions: the plain [`CovTable`] and the host
/// state its mutations drive.
#[derive(Debug)]
pub struct CovSubscriptionTable {
    core: CovTable,
    /// Wake-ups of the confirmed workers with a report outstanding, so a
    /// fence can stop one. No flight state.
    wakes: BTreeMap<Token, Arc<Notify>>,
    revisits: Arc<CovRevisits>,
    counters: Arc<AtomicCovCounters>,
    in_flight: Arc<CovInFlightTracker>,
    dispatch_turn: usize,
    timed: timed::TimedStore,
    /// Live list samples taken for read requests, so tests can see which
    /// requests snapshot the table (#1213).
    #[cfg(test)]
    live_samples: std::sync::atomic::AtomicUsize,
}

impl Default for CovSubscriptionTable {
    fn default() -> Self {
        Self::new()
    }
}

impl CovSubscriptionTable {
    /// Create a new COV subscription table with default policy and counters.
    pub fn new() -> Self {
        Self::with_policy(CovPolicy::default(), Arc::new(AtomicCovCounters::default()))
    }

    /// Create a new COV subscription table with a custom policy and counters.
    pub fn with_policy(policy: CovPolicy, counters: Arc<AtomicCovCounters>) -> Self {
        let timed = timed::TimedStore::new(DEFAULT_TIMED_APDU_LENGTH, Arc::clone(&counters));
        Self {
            core: CovTable::new(policy),
            wakes: BTreeMap::new(),
            revisits: Arc::default(),
            counters,
            in_flight: Arc::new(CovInFlightTracker::default()),
            dispatch_turn: 0,
            timed,
            #[cfg(test)]
            live_samples: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Bound each Multiple context's pending timestamped changes by what one
    /// notification of this maximum APDU length can carry.
    pub fn with_max_apdu_length(mut self, max_apdu_length: usize) -> Self {
        self.timed = timed::TimedStore::new(max_apdu_length, Arc::clone(&self.counters));
        self
    }

    /// Time confirmed reports with `timing`: the APDU timeout and retries
    /// the server's notifications use, which set each report's deadline and
    /// the hold-off after a failure.
    pub fn with_flight_timing(mut self, timing: FlightTiming) -> Self {
        self.core = self.core.with_flight_timing(timing);
        self
    }

    /// Pending timestamped COV-multiple changes of this table's references.
    pub(crate) fn timed(&self) -> &timed::TimedStore {
        &self.timed
    }

    /// Return the next notification dispatch turn counter (wrapping).
    pub(crate) fn next_dispatch_turn(&mut self) -> usize {
        let turn = self.dispatch_turn;
        self.dispatch_turn = self.dispatch_turn.wrapping_add(1);
        turn
    }

    /// Get a reference to the active COV policy.
    pub fn policy(&self) -> &CovPolicy {
        self.core.policy()
    }

    /// Get a reference to the atomic counters.
    pub fn counters(&self) -> &Arc<AtomicCovCounters> {
        &self.counters
    }

    /// Get a reference to the in-flight confirmed notification tracker.
    pub fn in_flight_tracker(&self) -> &Arc<CovInFlightTracker> {
        &self.in_flight
    }

    /// Get the number of active subscriptions for a peer.
    pub fn peer_subscription_count(&self, peer: &CovRecipient) -> usize {
        self.core.peer_subscription_count(peer)
    }

    /// Get the number of active indefinite subscriptions for a peer.
    pub fn peer_indefinite_count(&self, peer: &CovRecipient) -> usize {
        self.core.peer_indefinite_count(peer)
    }

    /// Get an accepted entry by its complete typed identity.
    pub fn get_subscription(&self, key: &CovSubscriptionKey) -> Option<&CovSubscriptionSnapshot> {
        self.core.get_subscription(key)
    }

    /// Every accepted entry, ordered by monitored object.
    pub(crate) fn entries(&self) -> impl Iterator<Item = &CovSubscriptionSnapshot> {
        self.core.entries()
    }

    /// Every accepted reference of one Multiple context.
    pub(crate) fn multiple_context_references<'a>(
        &'a self,
        context: &'a MultipleContextKey,
    ) -> impl Iterator<Item = &'a CovSubscriptionSnapshot> + 'a {
        self.core.multiple_context_references(context)
    }

    /// Whether an exact subscription identity is present.
    pub fn contains(&self, key: &CovSubscriptionKey) -> bool {
        self.core.contains(key)
    }

    /// Cancel only the exact family/form/property/index identity.
    pub fn unsubscribe(&mut self, key: &CovSubscriptionKey) -> bool {
        let removed = self.core.unsubscribe(key);
        self.sync(&[]);
        removed
    }

    /// Cancel every reference in the exact Multiple context.
    pub fn unsubscribe_cov_multiple_context(&mut self, context: &MultipleContextKey) {
        self.core.unsubscribe_cov_multiple_context(context);
        self.sync(&[]);
    }

    /// Remove all subscriptions for a given object (used on DeleteObject).
    pub fn remove_for_object(&mut self, oid: ObjectIdentifier) {
        self.core.remove_for_object(oid);
        self.sync(&[]);
    }

    /// Remove subscriptions using this current route and release shared peer quotas.
    /// An obsolete router cannot remove a subscription migrated elsewhere.
    pub fn remove_peer_subscriptions(
        &mut self,
        mac: &[u8],
        network: Option<&NpduAddress>,
    ) -> usize {
        let removed = self.core.remove_peer_subscriptions(mac, network);
        self.sync(&[]);
        removed
    }

    /// Remove all expired subscriptions, and end confirmed reports past
    /// their deadline. Returns the number of subscriptions removed.
    pub fn purge_expired(&mut self) -> usize {
        let purged = self.core.expire(runtime_clock::core_now());
        self.sync(&[]);
        purged
    }

    /// Test fixture: let every subscription's lifetime run out now, without
    /// waiting for the clock that lifetimes use to reach it.
    #[cfg(test)]
    pub(crate) fn expire_all_for_test(&mut self) {
        self.core.expire_all(runtime_clock::core_now());
    }

    /// Get all active (non-expired) subscriptions for a given object.
    pub fn subscriptions_for(&mut self, oid: &ObjectIdentifier) -> Vec<&CovSubscriptionSnapshot> {
        self.core.expire(runtime_clock::core_now());
        self.sync(&[]);
        self.core.object_entries(*oid).collect()
    }

    /// The finest COV increment each object's live subscriptions to
    /// `property` ask for (#1510); see [`CovTable::finest_increments`].
    pub(crate) fn finest_increments(
        &self,
        property: PropertyIdentifier,
    ) -> BTreeMap<ObjectIdentifier, f64> {
        self.core
            .finest_increments(property, runtime_clock::core_now())
    }

    /// Whether a snapshot still owns a live entry in this table.
    pub fn is_current(&self, snapshot: &CovSubscriptionSnapshot) -> bool {
        self.core.is_current(snapshot, runtime_clock::core_now())
    }

    /// Ordinary whole-object trigger policy; see [`CovTable::should_notify`].
    pub fn should_notify(
        sub: &CovSubscription,
        current_value: Option<&CovSample>,
        cov_increment: Option<f64>,
    ) -> bool {
        CovTable::should_notify(sub, current_value, cov_increment)
    }

    /// Number of active subscriptions.
    pub fn len(&self) -> usize {
        self.core.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.core.is_empty()
    }

    /// Accept an ordinary or Single insertion or renewal; see
    /// [`CovTable::subscribe`].
    #[cfg(test)]
    pub(crate) fn subscribe(
        &mut self,
        sub: CovSubscription,
    ) -> Result<CovSubscriptionSnapshot, Error> {
        let accepted = self.core.subscribe(sub, runtime_clock::core_now());
        let published = match &accepted {
            Ok(accepted) => std::slice::from_ref(accepted),
            Err(_) => &[],
        };
        self.sync(published);
        accepted
    }

    /// SubscribeCOV from `peer`; see [`CovTable::subscribe_cov`].
    pub fn subscribe_cov(
        &mut self,
        db: &ObjectDatabase,
        peer: &SubscriberEndpoint,
        service_data: &[u8],
    ) -> Result<Option<CovSubscriptionSnapshot>, Error> {
        let accepted = self
            .core
            .subscribe_cov(db, peer, service_data, runtime_clock::core_now());
        self.sync(accepted.as_ref().map(Option::as_slice).unwrap_or_default());
        accepted
    }

    /// SubscribeCOVProperty from `peer`; see
    /// [`CovTable::subscribe_cov_property`].
    pub fn subscribe_cov_property(
        &mut self,
        db: &ObjectDatabase,
        peer: &SubscriberEndpoint,
        service_data: &[u8],
    ) -> Result<Option<CovSubscriptionSnapshot>, Error> {
        let accepted =
            self.core
                .subscribe_cov_property(db, peer, service_data, runtime_clock::core_now());
        self.sync(accepted.as_ref().map(Option::as_slice).unwrap_or_default());
        accepted
    }

    /// Accept Multiple references in request order and refresh their exact
    /// context; see [`CovTable::subscribe_multiple`].
    pub(crate) fn subscribe_multiple(
        &mut self,
        context: &MultipleContextKey,
        route: &SubscriberEndpoint,
        expires_at: bacnet_time::Instant,
        max_notification_delay: u32,
        subscriber_max_apdu: Option<u16>,
        subscriptions: Vec<CovSubscription>,
    ) -> Result<Vec<CovSubscriptionSnapshot>, MultipleRefusal> {
        let accepted = self.core.subscribe_multiple(
            context,
            route,
            expires_at,
            max_notification_delay,
            subscriber_max_apdu,
            subscriptions,
            runtime_clock::core_now(),
        );
        let published = match &accepted {
            Ok(accepted) => accepted.as_slice(),
            Err(refusal) => refusal.committed.as_slice(),
        };
        self.sync(published);
        accepted
    }

    /// Resolve captured owner/key/generation/route authority against live
    /// expiry at `now`, on the server's core timeline.
    pub(crate) fn remaining_lifetime(
        &self,
        snapshot: &CovSubscriptionSnapshot,
        now: bacnet_time::Instant,
    ) -> Option<CovTimeRemaining> {
        self.core.remaining_lifetime(snapshot, now)
    }

    /// Issue the completion ticket of `snapshot`'s observation; see
    /// [`CovTable::prepare_completion`]. Call it under the database guard
    /// the observation was read under.
    pub(crate) fn prepare_completion(
        &mut self,
        snapshot: &CovSubscriptionSnapshot,
    ) -> Option<PreparedCovCompletion> {
        self.core.prepare_completion(snapshot)
    }

    /// Commit a delivered observation; see [`CovTable::complete_observation`].
    pub(crate) fn complete_observation(
        &mut self,
        snapshot: &CovSubscriptionSnapshot,
        completion: PreparedCovCompletion,
        value: CovObservation,
    ) -> bool {
        self.core
            .complete_observation(snapshot, completion, value, runtime_clock::core_now())
    }

    /// Mark the coordinate of one confirmed notification's references
    /// outstanding, and register its worker's wake-up for a fence.
    pub(crate) fn begin_confirmed<'a>(
        &mut self,
        completion: PreparedCovCompletion,
        snapshots: impl IntoIterator<Item = &'a CovSubscriptionSnapshot>,
    ) -> Result<ConfirmedFlight, BeginRefusal> {
        let token = self
            .core
            .begin_confirmed(completion, snapshots, runtime_clock::core_now())?;
        // A report it replaced past its deadline is fenced: wake its worker,
        // if one still runs, and drop its wake-up (#2303).
        self.sync(&[]);
        let wake = Arc::new(Notify::new());
        self.wakes.insert(token, Arc::clone(&wake));
        Ok(ConfirmedFlight::new(token, wake))
    }

    /// End the report `flight` names, as `outcome` says.
    pub(crate) fn delivered(&mut self, flight: &ConfirmedFlight, outcome: Delivery) {
        self.wakes.remove(&flight.token());
        self.core
            .delivered(flight.token(), outcome, runtime_clock::core_now());
    }

    /// Whether a fanout may report this ordinary or Single subscription now;
    /// see [`CovTable::confirmed_idle`].
    pub(crate) fn confirmed_idle(&self, snapshot: &CovSubscriptionSnapshot) -> bool {
        self.core
            .confirmed_idle(snapshot, runtime_clock::core_now())
    }

    /// Whether a whole COV-multiple context may report now; see
    /// [`CovTable::context_idle`].
    pub(crate) fn context_idle(
        &self,
        context: &MultipleContextKey,
        snapshots: &[CovSubscriptionSnapshot],
    ) -> bool {
        self.core
            .context_idle(context, snapshots, runtime_clock::core_now())
    }

    /// End of the hold-off a failed report put on a confirmed context, on
    /// Tokio's clock, while it is in force.
    pub(crate) fn context_hold_until(
        &self,
        context: &MultipleContextKey,
    ) -> Option<tokio::time::Instant> {
        let until = self
            .core
            .context_hold_until(context, runtime_clock::core_now())?;
        runtime_clock::core_clock()
            .to_std(until)
            .map(tokio::time::Instant::from_std)
    }

    /// Whether an idle context still owes the follow-up of a failed report;
    /// see [`CovTable::take_owed_context`].
    pub(crate) fn take_owed_context(&mut self, context: &MultipleContextKey) -> bool {
        self.core
            .take_owed_context(context, runtime_clock::core_now())
    }

    /// References owed a fresh evaluation after an acknowledgment.
    pub(crate) fn revisits(&self) -> &Arc<CovRevisits> {
        &self.revisits
    }

    /// Apply what the plain table's last mutation reports, after it
    /// `published` those snapshots: the counters, the removed references'
    /// histories and queued revisits, the published references' histories,
    /// each refreshed context's history terms, fenced workers' wake-ups and
    /// owed revisits. Under the table's write guard, so in the order the
    /// table made the changes.
    fn sync(&mut self, published: &[CovSubscriptionSnapshot]) {
        self.counters.add(&self.core.take_counts());
        let fenced = self.core.take_fenced();
        {
            let mut timed = self.timed.lock();
            for key in &fenced.removed {
                timed.remove(key);
            }
            for snapshot in published {
                let delay = snapshot.max_notification_delay().unwrap_or_default();
                let (key, generation) = (snapshot.key(), snapshot.generation());
                match (key.multiple_context(), snapshot.timestamped) {
                    (Some(_), true) => timed.reset(key, generation, delay),
                    (Some(_), false) => timed.reset_untimed(key, generation, delay),
                    (None, _) => timed.remove(key),
                }
            }
            for context in &fenced.contexts {
                let Some(entry) = self.core.multiple_context_references(context).next() else {
                    continue;
                };
                timed.set_delay(context, entry.max_notification_delay().unwrap_or_default());
                // Lifetime left now, rounded up as notifications report it; an
                // expired one, which no notification reports, sizes as the longest.
                let time_remaining =
                    CovTimeRemaining::at(entry.expires_at, runtime_clock::core_now())
                        .wire_seconds()
                        .unwrap_or(u32::MAX);
                timed.set_sizing(context, entry.subscriber_max_apdu(), time_remaining);
            }
        }
        for key in &fenced.removed {
            self.revisits.forget(key);
        }
        for token in &fenced.tokens {
            if let Some(wake) = self.wakes.remove(token) {
                wake.notify_one();
            }
        }
        self.revisits.request(fenced.revisit);
    }
}

#[cfg(test)]
impl CovSubscriptionTable {
    /// Test fixture: admit one proposal through its family's production
    /// owner. A Multiple reference is a one-reference `subscribe_multiple`
    /// request (finite expiry required) that refreshes its exact context with
    /// the supplied maximum notification delay.
    pub(crate) fn admit_for_test(
        &mut self,
        sub: CovSubscription,
        max_notification_delay: u32,
    ) -> Result<CovSubscriptionSnapshot, Error> {
        let Some(context) = sub.key()?.multiple_context().cloned() else {
            return self.subscribe(sub);
        };
        let expires_at = sub
            .expires_at
            .expect("Multiple contexts always have a finite lifetime");
        let mut accepted = self
            .subscribe_multiple(
                &context,
                &sub.endpoint(),
                expires_at,
                max_notification_delay,
                None,
                vec![sub],
            )
            .map_err(|refusal| refusal.error)?;
        Ok(accepted.remove(0))
    }

    /// Test setup still uses real reservation and completion, not a baseline
    /// bypass. A confirmed reference goes through its own outstanding report.
    pub(crate) fn complete_for_test(
        &mut self,
        snapshot: &CovSubscriptionSnapshot,
        value: CovObservation,
    ) -> bool {
        let Some(completion) = self.prepare_completion(snapshot) else {
            return false;
        };
        if !snapshot.issue_confirmed_notifications {
            return self.complete_observation(snapshot, completion, value);
        }
        let Ok(flight) = self.begin_confirmed(completion, [snapshot]) else {
            return false;
        };
        let completed = self.complete_observation(snapshot, completion, value);
        self.delivered(&flight, Delivery::NotSent);
        completed
    }
}
