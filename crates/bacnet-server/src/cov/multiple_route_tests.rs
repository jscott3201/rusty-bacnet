use super::*;

fn routed_proposal(router: u8, index: Option<u32>) -> CovSubscription {
    let mut sub = proposal(index, false);
    sub.subscriber_mac = MacAddr::from_slice(&[router]);
    sub.subscriber_network = Some(NpduAddress {
        network: 10,
        mac_address: MacAddr::from_slice(&[4]),
    });
    sub
}

#[test]
fn cov_multiple_route_empty_renewal_fences_all_old_snapshots_without_new_generations() {
    let mut table = CovSubscriptionTable::new();
    let a = routed_proposal(1, None);
    let old = [
        table.admit_for_test(a.clone(), 1).unwrap(),
        table
            .admit_for_test(routed_proposal(1, Some(0)), 1)
            .unwrap(),
    ];
    let context = context(&a);
    let expiry = Instant::now() + Duration::from_secs(600);
    table.generation = u64::MAX;
    table
        .subscribe_multiple(&context, &a.endpoint(), expiry, 2, None, vec![])
        .unwrap();
    assert!(
        old.iter().all(|snapshot| table.is_current(snapshot)),
        "same-route renewal preserves authority"
    );
    let b = routed_proposal(2, None);
    assert!(
        table
            .subscribe_multiple(&context, &b.endpoint(), expiry, 3, None, vec![])
            .unwrap()
            .is_empty()
    );
    assert_eq!(table.generation, u64::MAX);
    for snapshot in &old {
        assert!(!table.is_current(snapshot));
        assert!(!table.complete_for_test(
            snapshot,
            snapshot.last_notified_observation.clone().unwrap()
        ));
        let live = table.get_subscription(snapshot.key()).unwrap();
        assert_eq!(live.generation, snapshot.generation);
        assert_eq!(
            live.last_notified_observation,
            snapshot.last_notified_observation
        );
        assert_eq!(live.endpoint(), b.endpoint());
        assert_eq!(live.max_notification_delay(), Some(3));
        assert_eq!(live.expires_at, Some(expiry));
        assert_eq!(
            table.remaining_lifetime(live, expiry),
            Some(CovTimeRemaining::Expired)
        );
    }
    assert_eq!(
        table.remove_peer_subscriptions(&a.subscriber_mac, a.subscriber_network.as_ref()),
        0
    );
    table
        .subscribe_multiple(&context, &a.endpoint(), expiry, 4, None, vec![])
        .unwrap();
    assert!(
        old.iter().all(|snapshot| !table.is_current(snapshot)),
        "A-B-A cannot revive an old token"
    );
    assert_eq!(
        table.remove_peer_subscriptions(&a.subscriber_mac, a.subscriber_network.as_ref()),
        2
    );
    assert!(table.is_empty());
}

#[test]
fn cov_multiple_route_rejected_generation_or_identity_preserves_live_target() {
    let mut table = CovSubscriptionTable::new();
    let a = routed_proposal(1, None);
    let before = table.admit_for_test(a.clone(), 1).unwrap();
    let context = context(&a);
    let mut b = routed_proposal(2, Some(0));
    let expiry = b.expires_at.unwrap();
    assert!(
        matches!(
            table.subscribe_multiple(
                &context,
                &b.endpoint(),
                a.expires_at.unwrap(),
                9,
                None,
                vec![a.clone()]
            ),
            Err(MultipleRefusal {
                error: Error::Encoding(_),
                refused: None,
                ..
            })
        ),
        "a proposal through A cannot publish on a separately supplied B route"
    );
    table.generation = u64::MAX;
    let exhausted = table
        .subscribe_multiple(&context, &b.endpoint(), expiry, 9, None, vec![b.clone()])
        .unwrap_err();
    assert!(exhausted.refused.is_none() && exhausted.committed.is_empty());
    resource_error(exhausted.error);
    let mut wrong_route = b.endpoint();
    wrong_route.network.as_mut().unwrap().network = 11;
    assert!(
        table
            .subscribe_multiple(&context, &wrong_route, expiry, 9, None, vec![])
            .is_err()
    );
    b.subscriber_process_identifier += 1;
    assert!(
        table
            .subscribe_multiple(&context, &b.endpoint(), expiry, 9, None, vec![b.clone()])
            .is_err()
    );
    assert_eq!(table.len(), 1);
    assert!(table.is_current(&before));
    let live = table.get_subscription(before.key()).unwrap();
    assert_eq!(live.endpoint(), a.endpoint());
    assert_eq!(live.expires_at, before.expires_at);
    assert_eq!(live.max_notification_delay(), Some(1));
    assert_eq!(
        live.last_notified_observation,
        before.last_notified_observation
    );
}

#[test]
fn cov_multiple_route_distinct_contexts_and_cleanup_remain_independent() {
    let mut table = CovSubscriptionTable::new();
    let base = routed_proposal(1, None);
    let first = table.admit_for_test(base.clone(), 0).unwrap();
    let mut variations = Vec::new();
    let mut other = base.clone();
    other.subscriber_network.as_mut().unwrap().network += 1;
    variations.push(other);
    let mut other = base.clone();
    other.subscriber_network.as_mut().unwrap().mac_address = MacAddr::from_slice(&[5]);
    variations.push(other);
    let mut other = base.clone();
    other.subscriber_process_identifier += 1;
    variations.push(other);
    let mut other = base.clone();
    other.issue_confirmed_notifications = true;
    variations.push(other);
    let mut other = base.clone();
    other.subscriber_network = None;
    variations.push(other);
    for variation in variations {
        table.admit_for_test(variation, 0).unwrap();
    }
    let b = routed_proposal(2, None);
    table
        .subscribe_multiple(
            &context(&base),
            &b.endpoint(),
            b.expires_at.unwrap(),
            0,
            None,
            vec![],
        )
        .unwrap();
    assert_eq!(table.len(), 6);
    assert_eq!(
        table.remove_peer_subscriptions(&b.subscriber_mac, b.subscriber_network.as_ref()),
        1
    );
    assert!(table.get_subscription(first.key()).is_none());
    assert_eq!(table.len(), 5);
    table.remove_for_object(base.monitored_object_identifier);
    assert!(table.is_empty());
    table.admit_for_test(base.clone(), 0).unwrap();
    table
        .subscribe_multiple(
            &context(&base),
            &b.endpoint(),
            Instant::now(),
            0,
            None,
            vec![],
        )
        .unwrap();
    assert_eq!(table.purge_expired(), 1);
    assert_eq!(table.peer_subscription_count(&base.recipient()), 0);
}

#[test]
fn cov_ordinary_and_single_share_recipient_with_current_route_cleanup() {
    let mut table = CovSubscriptionTable::new();
    for property in [None, Some(PropertyIdentifier::PRESENT_VALUE)] {
        for router in [1, 2] {
            let mut sub = routed_proposal(router, None);
            sub.notification_kind = CovNotificationKind::Single;
            sub.monitored_property = property;
            table.subscribe(sub).unwrap();
        }
    }
    assert_eq!(table.len(), 2);
    let a = routed_proposal(1, None);
    assert_eq!(
        table.remove_peer_subscriptions(&a.subscriber_mac, a.subscriber_network.as_ref()),
        0
    );
    assert_eq!(table.len(), 2);
}
