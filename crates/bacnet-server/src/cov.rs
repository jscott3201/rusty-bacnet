//! COV subscription engine — tracks SubscribeCOV subscriptions and their lifetimes.

use std::collections::HashMap;
use std::time::Instant;

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;

/// An active COV subscription.
#[derive(Debug, Clone)]
pub struct CovSubscription {
    /// MAC address of the subscriber.
    pub subscriber_mac: MacAddr,
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
}

/// Key for uniquely identifying a subscription:
/// (subscriber_mac, process_id, monitored_object, monitored_property).
/// Including monitored_property ensures SubscribeCOV (whole-object) and
/// SubscribeCOVProperty (per-property) coexist as independent subscriptions.
type SubKey = (MacAddr, u32, ObjectIdentifier, Option<PropertyIdentifier>);

/// Table of active COV subscriptions.
#[derive(Debug, Default)]
pub struct CovSubscriptionTable {
    subs: HashMap<SubKey, CovSubscription>,
}

impl CovSubscriptionTable {
    pub fn new() -> Self {
        Self {
            subs: HashMap::new(),
        }
    }

    /// Add or update a subscription.
    pub fn subscribe(&mut self, sub: CovSubscription) {
        let key = (
            sub.subscriber_mac.clone(),
            sub.subscriber_process_identifier,
            sub.monitored_object_identifier,
            sub.monitored_property,
        );
        self.subs.insert(key, sub);
    }

    /// Remove a subscription by subscriber MAC, process identifier, and monitored object.
    pub fn unsubscribe(
        &mut self,
        mac: &[u8],
        process_id: u32,
        monitored_object: ObjectIdentifier,
    ) -> bool {
        let key = (MacAddr::from_slice(mac), process_id, monitored_object, None);
        self.subs.remove(&key).is_some()
    }

    /// Unsubscribe a per-property subscription.
    pub fn unsubscribe_property(
        &mut self,
        mac: &[u8],
        process_id: u32,
        monitored_object: ObjectIdentifier,
        monitored_property: PropertyIdentifier,
    ) -> bool {
        let key = (
            MacAddr::from_slice(mac),
            process_id,
            monitored_object,
            Some(monitored_property),
        );
        self.subs.remove(&key).is_some()
    }

    /// Remove all subscriptions for a given object (used on DeleteObject).
    pub fn remove_for_object(&mut self, oid: ObjectIdentifier) {
        self.subs.retain(|k, _| k.2 != oid);
    }

    /// Get all active (non-expired) subscriptions for a given object.
    pub fn subscriptions_for(&mut self, oid: &ObjectIdentifier) -> Vec<&CovSubscription> {
        let now = Instant::now();
        self.subs
            .retain(|_, sub| sub.expires_at.is_none_or(|exp| exp > now));
        self.subs
            .values()
            .filter(|sub| sub.monitored_object_identifier == *oid)
            .collect()
    }

    /// Update the last-notified value for a subscription.
    pub fn set_last_notified_value(
        &mut self,
        mac: &[u8],
        process_id: u32,
        monitored_object: ObjectIdentifier,
        monitored_property: Option<PropertyIdentifier>,
        value: f32,
    ) {
        let key = (
            MacAddr::from_slice(mac),
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

    /// Remove all expired subscriptions. Returns the number removed.
    pub fn purge_expired(&mut self) -> usize {
        let before = self.subs.len();
        let now = Instant::now();
        self.subs
            .retain(|_, sub| sub.expires_at.is_none_or(|exp| exp > now));
        before - self.subs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;
    use std::time::Duration;

    fn ai1() -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
    }

    fn ai2() -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap()
    }

    fn make_sub(mac: &[u8], process_id: u32, oid: ObjectIdentifier) -> CovSubscription {
        CovSubscription {
            subscriber_mac: MacAddr::from_slice(mac),
            subscriber_process_identifier: process_id,
            monitored_object_identifier: oid,
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_value: None,
            monitored_property: None,
            monitored_property_array_index: None,
            cov_increment: None,
        }
    }

    #[test]
    fn subscribe_and_lookup() {
        let mut table = CovSubscriptionTable::new();
        table.subscribe(make_sub(&[1, 2, 3], 1, ai1()));
        assert_eq!(table.len(), 1);
        assert_eq!(table.subscriptions_for(&ai1()).len(), 1);
        assert_eq!(table.subscriptions_for(&ai2()).len(), 0);
    }

    #[test]
    fn unsubscribe() {
        let mut table = CovSubscriptionTable::new();
        table.subscribe(make_sub(&[1, 2, 3], 1, ai1()));
        assert!(table.unsubscribe(&[1, 2, 3], 1, ai1()));
        assert!(!table.unsubscribe(&[1, 2, 3], 1, ai1())); // already removed
        assert!(table.is_empty());
    }

    #[test]
    fn expired_subscriptions_purged_on_lookup() {
        let mut table = CovSubscriptionTable::new();
        let mut sub = make_sub(&[1, 2, 3], 1, ai1());
        sub.expires_at = Some(Instant::now() - Duration::from_secs(1)); // already expired
        table.subscribe(sub);
        assert_eq!(table.subscriptions_for(&ai1()).len(), 0);
        assert!(table.is_empty());
    }

    #[test]
    fn multiple_subscribers_same_object() {
        let mut table = CovSubscriptionTable::new();
        table.subscribe(make_sub(&[1, 2, 3], 1, ai1()));
        table.subscribe(make_sub(&[4, 5, 6], 2, ai1()));
        assert_eq!(table.subscriptions_for(&ai1()).len(), 2);
    }

    #[test]
    fn should_notify_no_increment_always_fires() {
        let sub = make_sub(&[1, 2, 3], 1, ai1());
        // Binary/multi-state objects have no COV_Increment
        assert!(CovSubscriptionTable::should_notify(&sub, Some(1.0), None));
    }

    #[test]
    fn should_notify_first_notification_always_fires() {
        let sub = make_sub(&[1, 2, 3], 1, ai1());
        // First notification (last_notified_value = None)
        assert!(CovSubscriptionTable::should_notify(
            &sub,
            Some(72.5),
            Some(1.0)
        ));
    }

    #[test]
    fn should_notify_change_exceeds_increment() {
        let mut sub = make_sub(&[1, 2, 3], 1, ai1());
        sub.last_notified_value = Some(70.0);
        // Change of 2.5 >= increment of 1.0
        assert!(CovSubscriptionTable::should_notify(
            &sub,
            Some(72.5),
            Some(1.0)
        ));
    }

    #[test]
    fn should_notify_change_below_increment() {
        let mut sub = make_sub(&[1, 2, 3], 1, ai1());
        sub.last_notified_value = Some(72.0);
        // Change of 0.3 < increment of 1.0
        assert!(!CovSubscriptionTable::should_notify(
            &sub,
            Some(72.3),
            Some(1.0)
        ));
    }

    #[test]
    fn should_notify_exact_increment() {
        let mut sub = make_sub(&[1, 2, 3], 1, ai1());
        sub.last_notified_value = Some(70.0);
        // Change of exactly 1.0 == increment of 1.0 → fires
        assert!(CovSubscriptionTable::should_notify(
            &sub,
            Some(71.0),
            Some(1.0)
        ));
    }

    #[test]
    fn should_notify_zero_increment_always_fires() {
        let mut sub = make_sub(&[1, 2, 3], 1, ai1());
        sub.last_notified_value = Some(72.0);
        // COV_Increment = 0.0 means any change fires
        assert!(CovSubscriptionTable::should_notify(
            &sub,
            Some(72.001),
            Some(0.0)
        ));
    }

    #[test]
    fn set_last_notified_value_updates() {
        let mut table = CovSubscriptionTable::new();
        table.subscribe(make_sub(&[1, 2, 3], 1, ai1()));
        table.set_last_notified_value(&[1, 2, 3], 1, ai1(), None, 72.5);

        let subs = table.subscriptions_for(&ai1());
        assert_eq!(subs[0].last_notified_value, Some(72.5));
    }

    #[test]
    fn upsert_replaces_existing() {
        let mut table = CovSubscriptionTable::new();
        let mut sub = make_sub(&[1, 2, 3], 1, ai1());
        sub.issue_confirmed_notifications = false;
        table.subscribe(sub);
        // Same (mac, process_id, object) key — replaces the existing entry
        let mut sub2 = make_sub(&[1, 2, 3], 1, ai1());
        sub2.issue_confirmed_notifications = true;
        table.subscribe(sub2);
        assert_eq!(table.len(), 1);
        let subs = table.subscriptions_for(&ai1());
        assert!(subs[0].issue_confirmed_notifications);
    }

    #[test]
    fn same_subscriber_different_objects_both_exist() {
        let mut table = CovSubscriptionTable::new();
        // Same (mac, process_id) but different monitored objects
        table.subscribe(make_sub(&[1, 2, 3], 1, ai1()));
        table.subscribe(make_sub(&[1, 2, 3], 1, ai2()));
        assert_eq!(table.len(), 2);
        assert_eq!(table.subscriptions_for(&ai1()).len(), 1);
        assert_eq!(table.subscriptions_for(&ai2()).len(), 1);
    }

    #[test]
    fn purge_expired_removes_stale_subscriptions() {
        let mut table = CovSubscriptionTable::new();
        let mut sub1 = make_sub(&[1, 2, 3], 1, ai1());
        sub1.expires_at = Some(Instant::now() - Duration::from_secs(10));
        table.subscribe(sub1);

        let mut sub2 = make_sub(&[4, 5, 6], 2, ai1());
        sub2.expires_at = None; // infinite lifetime
        table.subscribe(sub2);

        let purged = table.purge_expired();
        assert_eq!(purged, 1);
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn purge_expired_returns_zero_when_none_expired() {
        let mut table = CovSubscriptionTable::new();
        table.subscribe(make_sub(&[1, 2, 3], 1, ai1()));
        let purged = table.purge_expired();
        assert_eq!(purged, 0);
        assert_eq!(table.len(), 1);
    }
}
