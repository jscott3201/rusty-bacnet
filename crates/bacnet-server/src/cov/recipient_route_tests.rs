use super::*;
use bacnet_device_core::cov::CovNotificationKind;
use bacnet_types::MacAddr;

fn routed_single(property: Option<PropertyIdentifier>) -> CovSubscription {
    let mut sub = proposal(None, false);
    sub.notification_kind = CovNotificationKind::Single;
    sub.monitored_property = property;
    sub.subscriber_network = Some(NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&[4]),
    });
    sub
}

#[test]
fn cov_recipient_renewal_retargets_terms_and_uses_existing_generation_fence() {
    for property in [None, Some(PropertyIdentifier::PRESENT_VALUE)] {
        let mut table = CovSubscriptionTable::with_policy(
            CovPolicy {
                max_subscriptions_per_peer: 1,
                ..Default::default()
            },
            Arc::new(AtomicCovCounters::default()),
        );
        let mut sub = routed_single(property);
        let old = table.subscribe(sub.clone()).unwrap();
        sub.subscriber_mac = MacAddr::from_slice(&[2]);
        sub.issue_confirmed_notifications = true;
        sub.cov_increment = Some(2.5);
        sub.expires_at =
            property.map(|_| crate::runtime_clock::core_now() + Duration::from_secs(600));
        // Real handler proposals reset ordinary/Single observations on renewal.
        sub.last_notified_observation = None;
        let current = table.subscribe(sub.clone()).unwrap();
        assert_eq!(old.key(), current.key());
        assert_eq!(old.recipient(), current.recipient());
        assert_eq!(table.len(), 1);
        assert_eq!(table.peer_subscription_count(&sub.recipient()), 1);
        assert_eq!(table.counters.snapshot().subscriptions_created, 1);
        assert_eq!(current.endpoint(), sub.endpoint());
        assert_eq!(current.expires_at, sub.expires_at);
        assert!(current.issue_confirmed_notifications);
        assert_eq!(current.cov_increment, Some(2.5));
        assert_eq!(current.last_notified_observation, None);
        assert!(!table.is_current(&old));
        assert!(!table.complete_for_test(&old, old.last_notified_observation.clone().unwrap()));
        assert_eq!(
            table.remove_peer_subscriptions(&[1], sub.subscriber_network.as_ref()),
            0
        );
        assert_eq!(
            table.remove_peer_subscriptions(&[2], sub.subscriber_network.as_ref()),
            1
        );
        assert_eq!(table.peer_subscription_count(&sub.recipient()), 0);
    }
}

#[test]
fn cov_recipient_failed_renewal_preserves_current_route_terms_and_observation() {
    for property in [None, Some(PropertyIdentifier::PRESENT_VALUE)] {
        let mut table = CovSubscriptionTable::new();
        let mut sub = routed_single(property);
        let old = table.subscribe(sub.clone()).unwrap();
        table.core.set_generation_for_test(u64::MAX);
        sub.subscriber_mac = MacAddr::from_slice(&[2]);
        sub.issue_confirmed_notifications = true;
        sub.expires_at = Some(crate::runtime_clock::core_now() + Duration::from_secs(600));
        sub.cov_increment = Some(2.5);
        sub.last_notified_observation = None;
        resource_error(table.subscribe(sub).unwrap_err());
        let current = table.get_subscription(old.key()).unwrap();
        assert!(table.is_current(&old));
        assert_eq!(current.endpoint(), old.endpoint());
        assert_eq!(current.expires_at, old.expires_at);
        assert_eq!(
            current.issue_confirmed_notifications,
            old.issue_confirmed_notifications
        );
        assert_eq!(current.cov_increment, old.cov_increment);
        assert_eq!(
            current.last_notified_observation,
            old.last_notified_observation
        );
        assert_eq!(table.peer_subscription_count(&old.recipient()), 1);
    }
}

#[test]
fn cov_recipient_identity_keeps_families_targets_indexes_and_clients_distinct() {
    let ordinary = routed_single(None);
    let mut proposals = vec![ordinary.clone()];
    for index in [None, Some(0), Some(1)] {
        let mut sub = routed_single(Some(PropertyIdentifier::PRIORITY_ARRAY));
        sub.monitored_property_array_index = index;
        proposals.push(sub);
    }
    proposals.push(routed_single(Some(PropertyIdentifier::PRESENT_VALUE)));
    for variant in 0..5 {
        let mut sub = ordinary.clone();
        match variant {
            0 => sub.subscriber_process_identifier += 1,
            1 => {
                sub.monitored_object_identifier =
                    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap()
            }
            2 => sub.subscriber_network.as_mut().unwrap().network += 1,
            3 => sub.subscriber_network.as_mut().unwrap().mac_address = MacAddr::from_slice(&[5]),
            _ => sub.subscriber_network = None,
        }
        proposals.push(sub);
    }
    let mut other_direct = ordinary.clone();
    other_direct.subscriber_network = None;
    other_direct.subscriber_mac = MacAddr::from_slice(&[2]);
    proposals.push(other_direct);
    let mut table = CovSubscriptionTable::new();
    for sub in &proposals {
        table.subscribe(sub.clone()).unwrap();
    }
    assert_eq!(table.len(), 11);
    // Multiple's two forms remain independent of ordinary/Single identity.
    for confirmed in [false, true] {
        let mut sub = routed_single(Some(PropertyIdentifier::PRESENT_VALUE));
        sub.notification_kind = CovNotificationKind::Multiple;
        sub.issue_confirmed_notifications = confirmed;
        table.admit_for_test(sub, 0).unwrap();
    }
    assert_eq!(table.len(), 13);
    for mut sub in proposals {
        let old_key = sub.key().unwrap();
        if sub.subscriber_network.is_some() {
            sub.subscriber_mac = MacAddr::from_slice(&[9]);
        }
        sub.issue_confirmed_notifications = true;
        let current = table.subscribe(sub).unwrap();
        assert_eq!(*current.key(), old_key);
    }
    assert_eq!(table.len(), 13);
}

#[test]
fn cov_recipient_full_peer_renewal_and_indefinite_refusal_preserve_accounting() {
    let mut table = CovSubscriptionTable::with_policy(
        CovPolicy {
            max_subscriptions_global: 4,
            max_subscriptions_per_peer: 2,
            max_indefinite_per_peer: 1,
            reserved_capacity: 0,
            ..Default::default()
        },
        Arc::new(AtomicCovCounters::default()),
    );
    let finite = routed_single(None);
    let old = table.subscribe(finite.clone()).unwrap();
    let mut indefinite = finite.clone();
    indefinite.subscriber_process_identifier += 1;
    indefinite.expires_at = None;
    table.subscribe(indefinite.clone()).unwrap();
    let mut other = finite.clone();
    other.subscriber_mac = MacAddr::from_slice(&[2]);
    other.subscriber_network.as_mut().unwrap().mac_address = MacAddr::from_slice(&[5]);
    table.subscribe(other.clone()).unwrap();
    other.subscriber_process_identifier += 1;
    table.subscribe(other.clone()).unwrap();
    assert_eq!(table.len(), 4);
    let mut refused = finite.clone();
    refused.subscriber_mac = MacAddr::from_slice(&[2]);
    refused.expires_at = None;
    refused.issue_confirmed_notifications = true;
    resource_error(table.subscribe(refused).unwrap_err());
    let current = table.get_subscription(old.key()).unwrap();
    assert!(table.is_current(&old));
    assert_eq!(current.endpoint(), old.endpoint());
    assert_eq!(current.expires_at, old.expires_at);
    assert_eq!(
        current.last_notified_observation,
        old.last_notified_observation
    );
    assert!(!current.issue_confirmed_notifications);
    for mut sub in [finite.clone(), indefinite] {
        sub.subscriber_mac = MacAddr::from_slice(&[2]);
        table.subscribe(sub).unwrap();
    }
    assert_eq!(table.len(), 4);
    assert_eq!(table.peer_subscription_count(&finite.recipient()), 2);
    assert_eq!(table.peer_indefinite_count(&finite.recipient()), 1);
    assert_eq!(table.peer_subscription_count(&other.recipient()), 2);
    assert_eq!(table.counters.snapshot().subscriptions_created, 4);
    assert_eq!(
        table.remove_peer_subscriptions(&[1], finite.subscriber_network.as_ref()),
        0
    );
    assert_eq!(
        table.remove_peer_subscriptions(&[2], finite.subscriber_network.as_ref()),
        2
    );
    assert_eq!(table.peer_subscription_count(&finite.recipient()), 0);
    assert_eq!(table.peer_indefinite_count(&finite.recipient()), 0);
    assert_eq!(table.peer_subscription_count(&other.recipient()), 2);
}

#[test]
fn cov_recipient_empty_routed_source_is_refused_before_any_table_effect() {
    for family in 0..3 {
        let mut table = CovSubscriptionTable::new();
        let mut original =
            routed_single((family != 0).then_some(PropertyIdentifier::PRESENT_VALUE));
        if family == 2 {
            original.notification_kind = CovNotificationKind::Multiple;
        }
        let before = table.admit_for_test(original.clone(), 0).unwrap();
        // Keep an expired unrelated row to prove malformed admission does not
        // even perform the normal valid-request expiry purge.
        let mut expired = routed_single(None);
        expired.subscriber_process_identifier += 1;
        expired.expires_at = Some(crate::runtime_clock::core_now());
        table.subscribe(expired).unwrap();
        let counters = table.counters.snapshot();
        let mut malformed = original.clone();
        malformed.subscriber_mac = MacAddr::from_slice(&[2]);
        malformed
            .subscriber_network
            .as_mut()
            .unwrap()
            .mac_address
            .clear();
        let invalid_recipient = malformed.recipient();
        assert!(
            matches!(invalid_recipient, CovRecipient::Routed(_)),
            "invalid routed input is never reinterpreted as direct"
        );
        if family == 2 {
            let invalid_context = MultipleContextKey {
                recipient: invalid_recipient.clone(),
                process_id: original.subscriber_process_identifier,
                confirmed: false,
            };
            for subscriptions in [vec![malformed.clone()], vec![]] {
                assert!(matches!(
                    table.subscribe_multiple(
                        &invalid_context,
                        &malformed.endpoint(),
                        original.expires_at.unwrap(),
                        0,
                        None,
                        subscriptions
                    ),
                    Err(MultipleRefusal {
                        error: Error::Encoding(_),
                        refused: None,
                        ..
                    })
                ));
            }
            assert!(matches!(
                table.subscribe_multiple(
                    before.key().multiple_context().unwrap(),
                    &malformed.endpoint(),
                    original.expires_at.unwrap(),
                    0,
                    None,
                    vec![]
                ),
                Err(MultipleRefusal {
                    error: Error::Encoding(_),
                    ..
                })
            ));
            table.unsubscribe_cov_multiple_context(&invalid_context);
        } else {
            assert!(matches!(
                table.subscribe(malformed),
                Err(Error::Encoding(_))
            ));
        }
        let mut invalid_key = before.key().clone();
        match &mut invalid_key {
            CovSubscriptionKey::Object { recipient, .. }
            | CovSubscriptionKey::Property { recipient, .. } => *recipient = invalid_recipient,
            CovSubscriptionKey::Multiple { context, .. } => context.recipient = invalid_recipient,
        }
        assert!(!table.unsubscribe(&invalid_key));
        assert_eq!(table.len(), 2);
        assert_eq!(table.counters.snapshot(), counters);
        assert!(table.is_current(&before));
        let current = table.get_subscription(before.key()).unwrap();
        assert_eq!(current.endpoint(), before.endpoint());
        assert_eq!(current.expires_at, before.expires_at);
        assert_eq!(
            current.last_notified_observation,
            before.last_notified_observation
        );
        assert_eq!(table.peer_subscription_count(&original.recipient()), 2);
    }
}
