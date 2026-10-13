//! COV subscription engine — tracks SubscribeCOV subscriptions and their lifetimes.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;

use crate::runtime_clock;

mod identity;
pub use identity::*;
pub(crate) mod active;
mod admission;
pub use admission::MultipleRefusal;
mod confirmed;
pub(crate) use confirmed::{BeginRefusal, CovRevisits};
mod sample;
pub use sample::CovSample;
mod observation;
mod observation_order;
pub use observation::CovObservation;
use observation_order::ObservationOwner;
pub(crate) use observation_order::PreparedCovCompletion;
pub(crate) mod flags;
mod lifetime;
pub(crate) mod multiple_reads;
pub(crate) mod prepare;
pub(crate) mod reported;
pub(crate) mod timed;
mod timed_capture;
pub(crate) use timed_capture::TimedWriteCapture;
pub(crate) mod value_source;
pub use lifetime::CovTimeRemaining;

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

/// Proposed COV subscription data. Table acceptance validates its canonical
/// identity and returns an immutable [`CovSubscriptionSnapshot`] for delivery.
#[derive(Debug, Clone)]
pub struct CovSubscription {
    /// Immediate delivery MAC: the local subscriber or its current router.
    pub subscriber_mac: MacAddr,
    /// Routed source address when the subscriber is behind a BACnet router.
    pub subscriber_network: Option<NpduAddress>,
    /// Process identifier chosen by the subscriber.
    pub subscriber_process_identifier: u32,
    /// The object being monitored.
    pub monitored_object_identifier: ObjectIdentifier,
    /// Notification form. Mutable renewal data for ordinary/Single subscriptions;
    /// part of the canonical identity for Multiple references.
    pub issue_confirmed_notifications: bool,
    /// When this subscription expires (None = infinite lifetime), on tokio's
    /// clock: `tokio::time::Instant::now().into_std()` plus the lifetime,
    /// which is `Instant::now()` plus it unless a test has paused the clock.
    pub expires_at: Option<Instant>,
    /// Last delivered bounded observation, including specialized command fields.
    pub last_notified_observation: Option<CovObservation>,
    /// Monitored property for Single-property and Multiple-reference subscriptions.
    pub monitored_property: Option<PropertyIdentifier>,
    /// Accepted property index; absent, zero and element indexes are independent.
    pub monitored_property_array_index: Option<u32>,
    /// COV increment override (SubscribeCOVProperty only).
    pub cov_increment: Option<f32>,
    /// Notification service family used for this subscription.
    pub notification_kind: CovNotificationKind,
    /// Whether COVNotificationMultiple values should include timeOfChange.
    pub timestamped: bool,
}

impl CovSubscription {
    /// Original client address shared by subscription identity and accounting.
    /// Process, family and monitored coordinates additionally distinguish subscriptions.
    pub fn recipient(&self) -> CovRecipient {
        CovRecipient::from_endpoint(&self.subscriber_mac, self.subscriber_network.as_ref())
    }
}

/// COV notification service family for a stored subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovNotificationKind {
    /// COVNotification / ConfirmedCOVNotification.
    Single,
    /// COVNotificationMultiple / ConfirmedCOVNotificationMultiple.
    Multiple,
}

/// Table of active COV subscriptions.
#[derive(Debug)]
pub struct CovSubscriptionTable {
    subs: HashMap<CovSubscriptionKey, CovSubscriptionSnapshot>,
    generation: u64,
    owner: Arc<ObservationOwner>,
    peer_counts: HashMap<CovRecipient, usize>,
    peer_indefinite_counts: HashMap<CovRecipient, usize>,
    policy: CovPolicy,
    counters: Arc<AtomicCovCounters>,
    in_flight: Arc<CovInFlightTracker>,
    revisits: Arc<CovRevisits>,
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
            subs: HashMap::new(),
            generation: 0,
            owner: Arc::new(ObservationOwner::default()),
            peer_counts: HashMap::new(),
            peer_indefinite_counts: HashMap::new(),
            policy: policy.sanitized(),
            counters,
            in_flight: Arc::new(CovInFlightTracker::default()),
            revisits: Arc::default(),
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
        &self.policy
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

    /// Every accepted reference of one Multiple context.
    pub(crate) fn multiple_context_references<'a>(
        &'a self,
        context: &'a MultipleContextKey,
    ) -> impl Iterator<Item = &'a CovSubscriptionSnapshot> + 'a {
        self.subs
            .iter()
            .filter(move |(key, _)| key.multiple_context() == Some(context))
            .map(|(_, sub)| sub)
    }

    /// Whether an exact subscription identity is present.
    pub fn contains(&self, key: &CovSubscriptionKey) -> bool {
        self.subs.contains_key(key)
    }

    fn remove_internal(&mut self, key: &CovSubscriptionKey, was_cancelled: bool) -> bool {
        if let Some(sub) = self.subs.remove(key) {
            self.timed.lock().remove(key);
            self.revisits.forget(key);
            let peer = sub.recipient();
            if let Some(count) = self.peer_counts.get_mut(&peer) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.peer_counts.remove(&peer);
                }
            }
            if sub.expires_at.is_none() {
                if let Some(count) = self.peer_indefinite_counts.get_mut(&peer) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        self.peer_indefinite_counts.remove(&peer);
                    }
                }
            }
            if was_cancelled {
                self.counters
                    .subscriptions_cancelled
                    .fetch_add(1, Ordering::Relaxed);
            }
            self.counters
                .subscriptions_active
                .store(self.subs.len() as u64, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Cancel only the exact family/form/property/index identity.
    pub fn unsubscribe(&mut self, key: &CovSubscriptionKey) -> bool {
        self.remove_internal(key, true)
    }

    /// Cancel every reference in the exact Multiple context.
    pub fn unsubscribe_cov_multiple_context(&mut self, context: &MultipleContextKey) {
        let keys: Vec<_> = self
            .subs
            .keys()
            .filter(|key| key.multiple_context() == Some(context))
            .cloned()
            .collect();
        for key in keys {
            self.remove_internal(&key, true);
        }
    }

    /// Remove all subscriptions for a given object (used on DeleteObject).
    pub fn remove_for_object(&mut self, oid: ObjectIdentifier) {
        let to_remove: Vec<_> = self
            .subs
            .iter()
            .filter(|(k, _)| k.object() == oid)
            .map(|(k, _)| k.clone())
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
            .iter()
            .filter(|(_, entry)| entry.endpoint() == target)
            .map(|(k, _)| k.clone())
            .collect();
        let count = to_remove.len();
        for key in to_remove {
            self.remove_internal(&key, true);
        }
        count
    }

    /// Remove all expired subscriptions. Returns the number removed.
    pub fn purge_expired(&mut self) -> usize {
        let now = runtime_clock::now();
        let mut purged_count = 0;
        let mut to_remove = Vec::new();
        for (k, sub) in &self.subs {
            if sub.expires_at.is_some_and(|exp| exp <= now) {
                to_remove.push(k.clone());
            }
        }
        for key in to_remove {
            purged_count += usize::from(self.remove_internal(&key, false));
        }
        if purged_count > 0 {
            self.counters
                .subscriptions_purged
                .fetch_add(purged_count as u64, Ordering::Relaxed);
            self.counters
                .subscriptions_active
                .store(self.subs.len() as u64, Ordering::Relaxed);
        }
        purged_count
    }

    /// Test fixture: let every subscription's lifetime run out now, without
    /// waiting for the clock that lifetimes use to reach it.
    #[cfg(test)]
    pub(crate) fn expire_all_for_test(&mut self) {
        let now = runtime_clock::now();
        for entry in self.subs.values_mut() {
            entry.subscription.expires_at = Some(now);
        }
    }

    /// Get all active (non-expired) subscriptions for a given object.
    pub fn subscriptions_for(&mut self, oid: &ObjectIdentifier) -> Vec<&CovSubscriptionSnapshot> {
        self.purge_expired();
        self.subs
            .values()
            .filter(|sub| sub.monitored_object_identifier == *oid)
            .collect()
    }

    /// The finest COV increment each object's live subscriptions to
    /// `property` ask for (#1510): SubscribeCOVProperty subscriptions and
    /// Multiple references that give one. A negative increment reports any
    /// change, as zero does, so it counts as zero; a NaN or infinite one
    /// reports nothing finer, so it's left out.
    pub(crate) fn finest_increments(
        &self,
        property: PropertyIdentifier,
    ) -> HashMap<ObjectIdentifier, f64> {
        let now = runtime_clock::now();
        let mut finest = HashMap::new();
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

    /// Whether a snapshot still owns a live entry in this table.
    pub fn is_current(&self, snapshot: &CovSubscriptionSnapshot) -> bool {
        self.remaining_lifetime(snapshot, runtime_clock::now())
            .and_then(CovTimeRemaining::wire_seconds)
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

    /// Number of active subscriptions.
    pub fn len(&self) -> usize {
        self.subs.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.subs.is_empty()
    }
}
