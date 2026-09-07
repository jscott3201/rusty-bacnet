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

mod policy;
pub use policy::*;

#[cfg(test)]
mod tests;

/// An active COV subscription.
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
    /// Whether to send ConfirmedCOVNotification (true) or Unconfirmed (false).
    pub issue_confirmed_notifications: bool,
    /// When this subscription expires (None = infinite lifetime).
    pub expires_at: Option<Instant>,
    /// Last present_value for which a COV notification was sent.
    /// Used with COV_Increment to decide whether to fire again.
    pub last_notified_value: Option<f32>,
    /// Property-level filter (SubscribeCOVProperty only).
    pub monitored_property: Option<PropertyIdentifier>,
    /// Array index within monitored property (SubscribeCOVProperty only).
    pub monitored_property_array_index: Option<u32>,
    /// COV increment override (SubscribeCOVProperty only).
    pub cov_increment: Option<f32>,
    /// Notification service family used for this subscription.
    pub notification_kind: CovNotificationKind,
    /// Whether COVNotificationMultiple values should include timeOfChange.
    pub timestamped: bool,
}

impl CovSubscription {
    /// Derive the canonical peer identity for this subscription.
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

/// Key for uniquely identifying a subscription:
/// (subscriber endpoint, process_id, monitored_object, monitored_property).
/// Including monitored_property ensures SubscribeCOV (whole-object) and
/// SubscribeCOVProperty (per-property) coexist as independent subscriptions.
type SubKey = (
    MacAddr,
    Option<NpduAddress>,
    u32,
    ObjectIdentifier,
    Option<PropertyIdentifier>,
);

/// Table of active COV subscriptions.
#[derive(Debug)]
pub struct CovSubscriptionTable {
    subs: HashMap<SubKey, CovSubscription>,
    peer_counts: HashMap<CovPeerKey, usize>,
    peer_indefinite_counts: HashMap<CovPeerKey, usize>,
    policy: CovPolicy,
    counters: Arc<AtomicCovCounters>,
    in_flight: Arc<CovInFlightTracker>,
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
            peer_counts: HashMap::new(),
            peer_indefinite_counts: HashMap::new(),
            policy: policy.sanitized(),
            counters,
            in_flight: Arc::new(CovInFlightTracker::default()),
        }
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

    /// Add or update a subscription.
    pub fn subscribe(&mut self, sub: CovSubscription) {
        let key = (
            sub.subscriber_mac.clone(),
            sub.subscriber_network.clone(),
            sub.subscriber_process_identifier,
            sub.monitored_object_identifier,
            sub.monitored_property,
        );
        let peer = sub.peer_key();
        let new_indefinite = sub.expires_at.is_none();
        if let Some(old) = self.subs.insert(key, sub) {
            let old_indefinite = old.expires_at.is_none();
            if old_indefinite != new_indefinite {
                if new_indefinite {
                    *self.peer_indefinite_counts.entry(peer).or_default() += 1;
                } else if let Some(count) = self.peer_indefinite_counts.get_mut(&peer) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        self.peer_indefinite_counts.remove(&peer);
                    }
                }
            }
        } else {
            *self.peer_counts.entry(peer.clone()).or_default() += 1;
            if new_indefinite {
                *self.peer_indefinite_counts.entry(peer).or_default() += 1;
            }
            self.counters
                .subscriptions_created
                .fetch_add(1, Ordering::Relaxed);
        }
        self.counters
            .subscriptions_active
            .store(self.subs.len() as u64, Ordering::Relaxed);
    }

    /// Get an active subscription by exact key if present.
    pub fn get_subscription(
        &self,
        mac: &MacAddr,
        network: Option<&NpduAddress>,
        process_id: u32,
        monitored_object: ObjectIdentifier,
        monitored_property: Option<PropertyIdentifier>,
    ) -> Option<&CovSubscription> {
        self.subs.get(&(
            mac.clone(),
            network.cloned(),
            process_id,
            monitored_object,
            monitored_property,
        ))
    }

    /// Whether a subscription key is already present.
    pub fn contains(
        &self,
        mac: &MacAddr,
        network: Option<&NpduAddress>,
        process_id: u32,
        monitored_object: ObjectIdentifier,
        monitored_property: Option<PropertyIdentifier>,
    ) -> bool {
        self.get_subscription(
            mac,
            network,
            process_id,
            monitored_object,
            monitored_property,
        )
        .is_some()
    }

    fn remove_internal(&mut self, key: &SubKey, was_cancelled: bool) -> bool {
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

    /// Remove a subscription by subscriber MAC, process identifier, and monitored object.
    pub fn unsubscribe(
        &mut self,
        mac: &[u8],
        process_id: u32,
        monitored_object: ObjectIdentifier,
    ) -> bool {
        self.unsubscribe_at(mac, None, process_id, monitored_object)
    }

    /// Remove a whole-object subscription for a specific subscriber endpoint.
    pub fn unsubscribe_at(
        &mut self,
        mac: &[u8],
        network: Option<&NpduAddress>,
        process_id: u32,
        monitored_object: ObjectIdentifier,
    ) -> bool {
        let key = (
            MacAddr::from_slice(mac),
            network.cloned(),
            process_id,
            monitored_object,
            None,
        );
        self.remove_internal(&key, true)
    }

    /// Unsubscribe a per-property subscription.
    pub fn unsubscribe_property(
        &mut self,
        mac: &[u8],
        process_id: u32,
        monitored_object: ObjectIdentifier,
        monitored_property: PropertyIdentifier,
    ) -> bool {
        self.unsubscribe_property_at(mac, None, process_id, monitored_object, monitored_property)
    }

    /// Unsubscribe a per-property subscription for a specific subscriber endpoint.
    pub fn unsubscribe_property_at(
        &mut self,
        mac: &[u8],
        network: Option<&NpduAddress>,
        process_id: u32,
        monitored_object: ObjectIdentifier,
        monitored_property: PropertyIdentifier,
    ) -> bool {
        let key = (
            MacAddr::from_slice(mac),
            network.cloned(),
            process_id,
            monitored_object,
            Some(monitored_property),
        );
        self.remove_internal(&key, true)
    }

    /// Remove every COV-multiple subscription in a subscriber context.
    pub fn unsubscribe_cov_multiple_context(
        &mut self,
        mac: &[u8],
        network: Option<&NpduAddress>,
        process_id: u32,
        confirmed: bool,
    ) {
        let mac = MacAddr::from_slice(mac);
        let to_remove: Vec<_> = self
            .subs
            .iter()
            .filter(|(_, sub)| {
                sub.notification_kind == CovNotificationKind::Multiple
                    && sub.subscriber_mac == mac
                    && sub.subscriber_network == network.cloned()
                    && sub.subscriber_process_identifier == process_id
                    && sub.issue_confirmed_notifications == confirmed
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in to_remove {
            self.remove_internal(&key, true);
        }
    }

    /// Remove one property from a COV-multiple subscriber context.
    pub fn unsubscribe_cov_multiple_property_at(
        &mut self,
        mac: &[u8],
        network: Option<&NpduAddress>,
        process_id: u32,
        confirmed: bool,
        monitored_object: ObjectIdentifier,
        monitored_property: PropertyIdentifier,
    ) {
        let mac = MacAddr::from_slice(mac);
        let to_remove: Vec<_> = self
            .subs
            .iter()
            .filter(|(_, sub)| {
                sub.notification_kind == CovNotificationKind::Multiple
                    && sub.subscriber_mac == mac
                    && sub.subscriber_network == network.cloned()
                    && sub.subscriber_process_identifier == process_id
                    && sub.issue_confirmed_notifications == confirmed
                    && sub.monitored_object_identifier == monitored_object
                    && sub.monitored_property == Some(monitored_property)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in to_remove {
            self.remove_internal(&key, true);
        }
    }

    /// Refresh the lifetime for an existing COV-multiple subscriber context.
    pub fn refresh_cov_multiple_context_lifetime(
        &mut self,
        mac: &[u8],
        network: Option<&NpduAddress>,
        process_id: u32,
        confirmed: bool,
        expires_at: Option<Instant>,
    ) {
        let mac = MacAddr::from_slice(mac);
        for sub in self.subs.values_mut() {
            if sub.notification_kind == CovNotificationKind::Multiple
                && sub.subscriber_mac == mac
                && sub.subscriber_network == network.cloned()
                && sub.subscriber_process_identifier == process_id
                && sub.issue_confirmed_notifications == confirmed
            {
                sub.expires_at = expires_at;
            }
        }
    }

    /// Remove all subscriptions for a given object (used on DeleteObject).
    pub fn remove_for_object(&mut self, oid: ObjectIdentifier) {
        let to_remove: Vec<_> = self
            .subs
            .iter()
            .filter(|(k, _)| k.3 == oid)
            .map(|(k, _)| k.clone())
            .collect();
        for key in to_remove {
            self.remove_internal(&key, false);
        }
    }

    /// Purge all subscriptions for a peer and deterministically release its quota.
    pub fn remove_peer_subscriptions(
        &mut self,
        mac: &[u8],
        network: Option<&NpduAddress>,
    ) -> usize {
        let target = CovPeerKey::from_endpoint(&MacAddr::from_slice(mac), network);
        let to_remove: Vec<_> = self
            .subs
            .iter()
            .filter(|(_, sub)| sub.peer_key() == target)
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
            if let Some(sub) = self.subs.remove(&key) {
                let peer = sub.peer_key();
                if let Some(count) = self.peer_counts.get_mut(&peer) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        self.peer_counts.remove(&peer);
                    }
                }
                purged_count += 1;
            }
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
    pub fn subscriptions_for(&mut self, oid: &ObjectIdentifier) -> Vec<&CovSubscription> {
        self.purge_expired();
        self.subs
            .values()
            .filter(|sub| sub.monitored_object_identifier == *oid)
            .collect()
    }

    /// Check admission for a single subscription request against policy quotas.
    pub fn check_admission(
        &self,
        peer: &CovPeerKey,
        is_indefinite: bool,
        existing: Option<&CovSubscription>,
    ) -> Result<(), Error> {
        if is_indefinite && !self.policy.allow_indefinite_subscriptions {
            self.counters
                .subscriptions_rejected_indefinite
                .fetch_add(1, Ordering::Relaxed);
            return Err(Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
            });
        }

        let (new_count, new_indefinite) = match existing {
            Some(sub) => {
                let was_indefinite = sub.expires_at.is_none();
                let add_indefinite = if is_indefinite && !was_indefinite {
                    1
                } else {
                    0
                };
                (0, add_indefinite)
            }
            None => {
                let add_indefinite = if is_indefinite { 1 } else { 0 };
                (1, add_indefinite)
            }
        };

        self.check_admission_multiple(peer, new_count, new_indefinite)
    }

    /// Check admission for a batch of subscriptions against policy quotas.
    pub fn check_admission_multiple(
        &self,
        peer: &CovPeerKey,
        new_count: usize,
        new_indefinite: usize,
    ) -> Result<(), Error> {
        if new_indefinite > 0 {
            if !self.policy.allow_indefinite_subscriptions {
                self.counters
                    .subscriptions_rejected_indefinite
                    .fetch_add(1, Ordering::Relaxed);
                return Err(Error::Protocol {
                    class: ErrorClass::SERVICES.to_raw() as u32,
                    code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
                });
            }
            let current_indefinite = self.peer_indefinite_counts.get(peer).copied().unwrap_or(0);
            if current_indefinite + new_indefinite > self.policy.max_indefinite_per_peer {
                self.counters
                    .subscriptions_rejected_indefinite
                    .fetch_add(1, Ordering::Relaxed);
                return Err(Error::Protocol {
                    class: ErrorClass::RESOURCES.to_raw() as u32,
                    code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
                });
            }
        }

        if new_count == 0 {
            return Ok(());
        }

        let current_peer = self.peer_counts.get(peer).copied().unwrap_or(0);
        if current_peer + new_count > self.policy.max_subscriptions_per_peer {
            self.counters
                .subscriptions_rejected_quota
                .fetch_add(1, Ordering::Relaxed);
            return Err(Error::Protocol {
                class: ErrorClass::RESOURCES.to_raw() as u32,
                code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
            });
        }

        if self.subs.len() + new_count > self.policy.max_subscriptions_global {
            self.counters
                .subscriptions_rejected_capacity
                .fetch_add(1, Ordering::Relaxed);
            return Err(Error::Protocol {
                class: ErrorClass::RESOURCES.to_raw() as u32,
                code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
            });
        }

        if !self.policy.is_peer_reserved(peer) {
            let unreserved_capacity = self
                .policy
                .max_subscriptions_global
                .saturating_sub(self.policy.reserved_capacity);
            if self.subs.len() + new_count > unreserved_capacity {
                self.counters
                    .subscriptions_rejected_capacity
                    .fetch_add(1, Ordering::Relaxed);
                return Err(Error::Protocol {
                    class: ErrorClass::RESOURCES.to_raw() as u32,
                    code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
                });
            }
        }

        Ok(())
    }

    /// Update the last-notified value for a subscription.
    pub fn set_last_notified_value(
        &mut self,
        mac: &[u8],
        network: Option<&NpduAddress>,
        process_id: u32,
        monitored_object: ObjectIdentifier,
        monitored_property: Option<PropertyIdentifier>,
        value: f32,
    ) {
        let key = (
            MacAddr::from_slice(mac),
            network.cloned(),
            process_id,
            monitored_object,
            monitored_property,
        );
        if let Some(sub) = self.subs.get_mut(&key) {
            sub.last_notified_value = Some(value);
        }
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
