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
        subscriber_network: None,
        subscriber_process_identifier: process_id,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: false,
        expires_at: None,
        last_notified_value: None,
        monitored_property: None,
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
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
    table.set_last_notified_value(&[1, 2, 3], None, 1, ai1(), None, 72.5);

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
fn cov_multiple_context_lifetime_refreshes_and_expires() {
    let mut table = CovSubscriptionTable::new();
    let original_expiry = Instant::now() + Duration::from_secs(30);
    let refreshed_expiry = Instant::now() + Duration::from_secs(60);

    let mut present_value = make_sub(&[1, 2, 3], 1, ai1());
    present_value.notification_kind = CovNotificationKind::Multiple;
    present_value.monitored_property = Some(PropertyIdentifier::PRESENT_VALUE);
    present_value.expires_at = Some(original_expiry);
    table.subscribe(present_value);

    let mut status_flags = make_sub(&[1, 2, 3], 1, ai1());
    status_flags.notification_kind = CovNotificationKind::Multiple;
    status_flags.monitored_property = Some(PropertyIdentifier::STATUS_FLAGS);
    status_flags.expires_at = Some(original_expiry);
    table.subscribe(status_flags);

    let mut single = make_sub(&[1, 2, 3], 1, ai1());
    single.expires_at = Some(original_expiry);
    table.subscribe(single);

    table.refresh_cov_multiple_context_lifetime(&[1, 2, 3], None, 1, false, Some(refreshed_expiry));

    let multiple_expiries: Vec<_> = table
        .subs
        .values()
        .filter(|sub| sub.notification_kind == CovNotificationKind::Multiple)
        .map(|sub| sub.expires_at)
        .collect();
    assert_eq!(
        multiple_expiries,
        vec![Some(refreshed_expiry), Some(refreshed_expiry)]
    );
    assert!(table
        .subs
        .values()
        .any(|sub| sub.notification_kind == CovNotificationKind::Single
            && sub.expires_at == Some(original_expiry)));

    table.refresh_cov_multiple_context_lifetime(
        &[1, 2, 3],
        None,
        1,
        false,
        Some(Instant::now() - Duration::from_secs(1)),
    );

    assert_eq!(table.purge_expired(), 2);
    assert_eq!(table.len(), 1);
    assert!(table.subs.values().all(|sub| {
        sub.notification_kind == CovNotificationKind::Single
            && sub.expires_at == Some(original_expiry)
    }));
}

#[test]
fn purge_expired_returns_zero_when_none_expired() {
    let mut table = CovSubscriptionTable::new();
    table.subscribe(make_sub(&[1, 2, 3], 1, ai1()));
    let purged = table.purge_expired();
    assert_eq!(purged, 0);
    assert_eq!(table.len(), 1);
}

#[test]
fn default_policy_allows_1024th_subscription() {
    let mut table = CovSubscriptionTable::new();
    let oid = ai1();
    // 16 peers * 64 subscriptions each = 1024 subscriptions
    for peer_idx in 0..16u8 {
        let mac = [192, 168, 1, peer_idx];
        let peer_key = CovPeerKey::direct(MacAddr::from_slice(&mac));
        for proc_id in 0..64u32 {
            table
                .check_admission(&peer_key, false, None)
                .expect("subscription admitted");
            let mut sub = make_sub(&mac, proc_id, oid);
            sub.expires_at = Some(Instant::now() + Duration::from_secs(300));
            table.subscribe(sub);
        }
    }
    assert_eq!(table.len(), 1024);

    // 1025th subscription from a 17th peer fails due to global capacity
    let mac17 = [192, 168, 1, 17];
    let peer17 = CovPeerKey::direct(MacAddr::from_slice(&mac17));
    assert!(table.check_admission(&peer17, false, None).is_err());
}

#[test]
fn effective_unreserved_capacity_respects_reserved_peers() {
    let mut policy = CovPolicy {
        max_subscriptions_global: 100,
        reserved_capacity: 20,
        reserved_peers: Vec::new(),
        ..Default::default()
    };
    // No reserved peers -> full global capacity available
    assert_eq!(policy.effective_unreserved_capacity(), 100);

    // With reserved peers -> reserved capacity is deducted
    policy.reserved_peers.push(MacAddr::from_slice(&[1, 2, 3]));
    assert_eq!(policy.effective_unreserved_capacity(), 80);

    // If reserved_capacity is 0 -> full global capacity
    policy.reserved_capacity = 0;
    assert_eq!(policy.effective_unreserved_capacity(), 100);
}

#[test]
fn in_flight_tracker_does_not_leak_zero_count_entries_on_failure() {
    let tracker = Arc::new(CovInFlightTracker::default());
    let semaphore = Arc::new(tokio::sync::Semaphore::new(0));
    let peer = CovPeerKey::direct(MacAddr::from_slice(&[1, 2, 3, 4]));

    // Acquisition fails due to global pool exhausted
    let err = tracker
        .try_acquire(peer.clone(), 10, &semaphore)
        .unwrap_err();
    assert_eq!(err, InFlightAcquireError::GlobalPoolExhausted);
    assert_eq!(tracker.active_peer_count(), 0);

    // Acquisition fails due to peer limit exceeded (max_per_peer = 0)
    let semaphore2 = Arc::new(tokio::sync::Semaphore::new(10));
    let peer2 = CovPeerKey::direct(MacAddr::from_slice(&[5, 6, 7, 8]));
    let err2 = tracker.try_acquire(peer2, 0, &semaphore2).unwrap_err();
    assert_eq!(err2, InFlightAcquireError::PeerLimitExceeded);
    assert_eq!(tracker.active_peer_count(), 0);
}

#[test]
fn expired_subscriptions_immediately_release_quota_on_admission() {
    let policy = CovPolicy {
        max_subscriptions_per_peer: 1,
        ..Default::default()
    };
    let mut table =
        CovSubscriptionTable::with_policy(policy, Arc::new(AtomicCovCounters::default()));
    let peer = CovPeerKey::direct(MacAddr::from_slice(&[1, 2, 3]));

    // Create a subscription with an expiry in the past
    let mut sub = make_sub(&[1, 2, 3], 1, ai1());
    sub.expires_at = Some(Instant::now() - Duration::from_secs(5));
    table.subscribe(sub);
    assert_eq!(table.len(), 1);

    // Admitting a new subscription from the same peer immediately purges the expired subscription
    // and succeeds, rather than being rejected by per-peer quota!
    assert!(table.check_admission(&peer, false, None).is_ok());
    assert_eq!(table.len(), 0);
}
