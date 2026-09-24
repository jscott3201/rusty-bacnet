use super::*;
use bacnet_types::enums::ObjectType;
use std::time::Duration;

fn proposal(index: Option<u32>, confirmed: bool) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[1]),
        subscriber_network: None,
        subscriber_process_identifier: 9,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        issue_confirmed_notifications: confirmed,
        expires_at: Some(Instant::now() + Duration::from_secs(60)),
        last_notified_sample: Some(
            crate::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(1.0))
                .unwrap(),
        ),
        monitored_property: Some(PropertyIdentifier::PRIORITY_ARRAY),
        monitored_property_array_index: index,
        cov_increment: Some(0.5),
        notification_kind: CovNotificationKind::Multiple,
        timestamped: false,
    }
}

fn context(sub: &CovSubscription) -> MultipleContextKey {
    sub.key().unwrap().multiple_context().unwrap().clone()
}

fn resource_error(error: Error) {
    assert!(matches!(error, Error::Protocol { class, code }
        if class == ErrorClass::RESOURCES.to_raw() as u32
        && code == ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32));
}

#[test]
fn cov_identity_generations_never_reuse_across_renew_cancel_or_foreign_table() {
    let mut table = CovSubscriptionTable::new();
    let sub = proposal(None, false);
    let first = table.subscribe(sub.clone()).unwrap();
    let second = table.subscribe(sub.clone()).unwrap();
    assert!(!table.set_last_notified_sample(
        &first,
        crate::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(7.0)).unwrap()
    ));
    assert!(table.set_last_notified_sample(
        &second,
        crate::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(8.0)).unwrap()
    ));
    assert!(table.unsubscribe(second.key()));
    assert!(!table.set_last_notified_sample(
        &second,
        crate::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(9.0)).unwrap()
    ));
    let third = table.subscribe(sub.clone()).unwrap();
    assert!(!table.set_last_notified_sample(
        &second,
        crate::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(9.0)).unwrap()
    ));
    assert_eq!(
        table
            .get_subscription(third.key())
            .unwrap()
            .last_notified_sample,
        Some(
            crate::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(1.0))
                .unwrap()
        )
    );
    let foreign = CovSubscriptionTable::new().subscribe(sub).unwrap();
    assert!(!table.set_last_notified_sample(
        &foreign,
        crate::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(99.0)).unwrap()
    ));
    assert!(first.generation < second.generation && second.generation < third.generation);
}

#[test]
fn cov_identity_batch_reserves_only_final_duplicates_and_exhaustion_is_atomic() {
    let mut table = CovSubscriptionTable::new();
    let existing = proposal(None, false);
    let before = table.subscribe(existing.clone()).unwrap();
    let peer = existing.peer_key();
    let context = context(&existing);
    let expiry = Instant::now() + Duration::from_secs(600);
    let mut replacement = existing.clone();
    replacement.expires_at = Some(expiry);
    replacement.cov_increment = Some(2.0);
    let mut added = replacement.clone();
    added.monitored_property_array_index = Some(0);
    table.generation = u64::MAX - 1;
    let counters = table.counters.snapshot();
    resource_error(
        table
            .subscribe_multiple(&context, expiry, vec![replacement.clone(), added])
            .unwrap_err(),
    );
    assert_eq!(table.generation, u64::MAX - 1);
    assert_eq!(table.len(), 1);
    assert_eq!(
        table.get_subscription(before.key()).unwrap().expires_at,
        before.expires_at
    );
    assert_eq!(
        table.get_subscription(before.key()).unwrap().cov_increment,
        before.cov_increment
    );
    assert_eq!(table.peer_subscription_count(&peer), 1);
    assert_eq!(
        table.counters.snapshot().subscriptions_created,
        counters.subscriptions_created
    );
    assert_eq!(
        table.counters.snapshot().subscriptions_active,
        counters.subscriptions_active
    );
    // A large duplicate list consumes only one final generation, and last options win.
    let mut final_options = replacement.clone();
    final_options.cov_increment = Some(3.0);
    let accepted = table
        .subscribe_multiple(
            &context,
            expiry,
            vec![replacement; 64]
                .into_iter()
                .chain([final_options])
                .collect(),
        )
        .unwrap();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].generation, u64::MAX);
    assert_eq!(accepted[0].cov_increment, Some(3.0));
    assert!(!table.set_last_notified_sample(
        &before,
        crate::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(100.0)).unwrap()
    ));
    resource_error(table.subscribe(existing).unwrap_err());
    assert_eq!(
        table
            .get_subscription(accepted[0].key())
            .unwrap()
            .generation,
        u64::MAX
    );
    assert!(table.unsubscribe(accepted[0].key()));
    assert!(!table.unsubscribe(accepted[0].key()));
    table.unsubscribe_cov_multiple_context(&context);
    assert!(table.is_empty());
    assert_eq!(table.peer_subscription_count(&peer), 0);
}

#[test]
fn cov_identity_indexes_forms_quota_and_exact_router_cleanup() {
    let mut table = CovSubscriptionTable::with_policy(
        CovPolicy {
            max_subscriptions_per_peer: 8,
            reserved_capacity: 0,
            ..Default::default()
        },
        Arc::new(AtomicCovCounters::default()),
    );
    let remote = NpduAddress {
        network: 10,
        mac_address: MacAddr::from_slice(&[4]),
    };
    for router in [1, 2] {
        for index in [None, Some(0), Some(1), Some(2)] {
            let mut sub = proposal(index, false);
            sub.subscriber_mac = MacAddr::from_slice(&[router]);
            sub.subscriber_network = Some(remote.clone());
            table.subscribe(sub).unwrap();
        }
    }
    let quota = CovPeerKey::from_endpoint(&MacAddr::from_slice(&[1]), Some(&remote));
    assert_eq!(table.peer_subscription_count(&quota), 8);
    let mut opposite = proposal(None, true);
    opposite.subscriber_network = Some(remote.clone());
    resource_error(table.subscribe(opposite.clone()).unwrap_err());
    assert_eq!(table.remove_peer_subscriptions(&[1], Some(&remote)), 4);
    assert_eq!(table.peer_subscription_count(&quota), 4);
    table.subscribe(opposite).unwrap();
    assert_eq!(table.peer_subscription_count(&quota), 5);
    assert_eq!(table.remove_peer_subscriptions(&[9], Some(&remote)), 0);
}

#[test]
fn cov_identity_ordinary_and_single_mode_renew_in_place() {
    let mut table = CovSubscriptionTable::new();
    for property in [None, Some(PropertyIdentifier::PRESENT_VALUE)] {
        let mut sub = proposal(None, false);
        sub.notification_kind = CovNotificationKind::Single;
        sub.monitored_property = property;
        let first = table.subscribe(sub.clone()).unwrap();
        sub.issue_confirmed_notifications = true;
        let renewed = table.subscribe(sub).unwrap();
        assert_eq!(first.key(), renewed.key());
        assert!(!table.is_current(&first));
        assert!(renewed.issue_confirmed_notifications);
    }
    assert_eq!(table.len(), 2);
    assert_eq!(table.counters.snapshot().subscriptions_created, 2);
}
