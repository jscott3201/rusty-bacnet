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

mod identity;
pub use identity::*;
mod admission;

mod policy;
pub use policy::*;

#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod tests;

/// Proposed COV subscription data. Table acceptance validates its canonical
/// identity and returns an immutable [`CovSubscriptionSnapshot`] for delivery.
#[derive(Debug, Clone)]
pub struct CovSubscription {
    /// MAC address of the subscriber.
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
    /// When this subscription expires (None = infinite lifetime).
    pub expires_at: Option<Instant>,
    /// Last present_value for which a COV notification was sent.
    /// Used with COV_Increment to decide whether to fire again.
    pub last_notified_value: Option<f32>,
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
    /// Derive the quota/rate group, not cancellation or replacement identity.
    pub fn peer_key(&self) -> CovPeerKey {
        CovPeerKey::from_endpoint(&self.subscriber_mac, self.subscriber_network.as_ref())
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
    owner: Arc<()>,
    peer_counts: HashMap<CovPeerKey, usize>,
    peer_indefinite_counts: HashMap<CovPeerKey, usize>,
    policy: CovPolicy,
    counters: Arc<AtomicCovCounters>,
    in_flight: Arc<CovInFlightTracker>,
    dispatch_turn: usize,
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
        Self {
            subs: HashMap::new(),
            generation: 0,
            owner: Arc::new(()),
            peer_counts: HashMap::new(),
            peer_indefinite_counts: HashMap::new(),
            policy: policy.sanitized(),
            counters,
            in_flight: Arc::new(CovInFlightTracker::default()),
            dispatch_turn: 0,
        }
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
    pub fn peer_subscription_count(&self, peer: &CovPeerKey) -> usize {
        self.peer_counts.get(peer).copied().unwrap_or(0)
    }

    /// Get the number of active indefinite subscriptions for a peer.
    pub fn peer_indefinite_count(&self, peer: &CovPeerKey) -> usize {
        self.peer_indefinite_counts.get(peer).copied().unwrap_or(0)
    }

    /// Get an accepted entry by its complete typed identity.
    pub fn get_subscription(&self, key: &CovSubscriptionKey) -> Option<&CovSubscriptionSnapshot> {
        self.subs.get(key)
    }

    /// Whether an exact subscription identity is present.
    pub fn contains(&self, key: &CovSubscriptionKey) -> bool {
        self.subs.contains_key(key)
    }

    fn remove_internal(&mut self, key: &CovSubscriptionKey, was_cancelled: bool) -> bool {
        if let Some(sub) = self.subs.remove(key) {
            let peer = sub.peer_key();
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

    /// Remove one exact endpoint and release its contribution to shared peer quotas.
    pub fn remove_peer_subscriptions(
        &mut self,
        mac: &[u8],
        network: Option<&NpduAddress>,
    ) -> usize {
        let target = SubscriberEndpoint::new(mac, network);
        let to_remove: Vec<_> = self
            .subs
            .iter()
            .filter(|(key, _)| key.endpoint() == &target)
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
        let now = Instant::now();
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

    /// Get all active (non-expired) subscriptions for a given object.
    pub fn subscriptions_for(&mut self, oid: &ObjectIdentifier) -> Vec<&CovSubscriptionSnapshot> {
        self.purge_expired();
        self.subs
            .values()
            .filter(|sub| sub.monitored_object_identifier == *oid)
            .collect()
    }

    /// Whether a snapshot still owns a live entry in this table.
    pub fn is_current(&self, snapshot: &CovSubscriptionSnapshot) -> bool {
        Arc::ptr_eq(&self.owner, &snapshot.owner)
            && self.subs.get(snapshot.key()).is_some_and(|entry| {
                entry.generation == snapshot.generation
                    && entry
                        .expires_at
                        .is_none_or(|expiry| expiry > Instant::now())
            })
    }

    /// Complete only the captured generation, never a renewal or recreated entry.
    pub fn set_last_notified_value(
        &mut self,
        snapshot: &CovSubscriptionSnapshot,
        value: f32,
    ) -> bool {
        if !self.is_current(snapshot) {
            return false;
        }
        self.subs
            .get_mut(snapshot.key())
            .unwrap()
            .subscription
            .last_notified_value = Some(value);
        true
    }

    /// Check if a COV notification should fire for a subscription given
    /// the current present_value and the object's COV_Increment.
    ///
    /// Returns `true` if:
    /// - No COV_Increment (binary/multi-state objects — always notify)
    /// - No previous notified value (first notification)
    /// - `|current - last_notified| >= cov_increment`
    pub fn should_notify(
        sub: &CovSubscription,
        current_value: Option<f32>,
        cov_increment: Option<f32>,
    ) -> bool {
        match (cov_increment, current_value) {
            (Some(increment), Some(current)) => {
                match sub.last_notified_value {
                    None => true, // First notification — always fire
                    Some(last) => (current - last).abs() >= increment,
                }
            }
            _ => true, // No increment or no numeric value — always notify
        }
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
