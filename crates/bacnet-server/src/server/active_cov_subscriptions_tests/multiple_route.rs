use super::*;

fn router_b() -> Peer {
    Peer {
        mac: vec![0x0A, 0, 0, 9, 0xBA, 0xC0],
        ..routed()
    }
}

#[tokio::test(start_paused = true)]
async fn multiple_two_routers_share_one_live_recipient_context() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    simple_ack(
        wire.send(
            &routed(),
            subscribe_cov_property_multiple(
                83,
                false,
                Some((300, 1)),
                vec![(
                    av(1),
                    vec![plain(PV), plain(PropertyIdentifier::STATUS_FLAGS)],
                )],
            ),
        )
        .await,
    );
    initials_complete(&wire, 2).await;
    simple_ack(
        wire.send(
            &router_b(),
            subscribe_cov_property_multiple(
                83,
                false,
                Some((600, 2)),
                vec![(av(2), vec![plain(PV)])],
            ),
        )
        .await,
    );
    initials_complete(&wire, 3).await;
    let contexts = wire.multiple().await;
    assert_eq!(
        contexts.len(),
        1,
        "one remote BACnet address/process/form through either router"
    );
    assert_eq!(contexts[0].recipient.recipient, address(&routed()));
    assert_eq!(contexts[0].max_notification_delay, 2);
    assert_eq!(contexts[0].list_of_cov_subscription_specifications.len(), 2);
    assert_eq!(contexts[0].time_remaining, 600);
    changed_value_uses(&wire, &router_b(), av(1), 21.0).await;

    // Empty finite renewal retargets all retained references without resetting
    // their paired selected-value/flags observations.
    let before = observations(&wire).await;
    simple_ack(
        wire.send(
            &routed(),
            subscribe_cov_property_multiple(83, false, Some((900, 3)), vec![]),
        )
        .await,
    );
    assert_eq!(observations(&wire).await, before);
    changed_value_uses(&wire, &routed(), av(2), 22.0).await;
    let contexts = wire.multiple().await;
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0].max_notification_delay, 3);

    // Cancellation through the other router removes canonical references but
    // keeps the remaining context on A.
    simple_ack(
        wire.send(
            &router_b(),
            subscribe_cov_property_multiple(83, false, None, vec![(av(2), vec![plain(PV)])]),
        )
        .await,
    );
    changed_value_uses(&wire, &routed(), av(1), 23.0).await;
    assert_eq!(
        wire.multiple().await[0]
            .list_of_cov_subscription_specifications
            .len(),
        1
    );
    simple_ack(
        wire.send(
            &router_b(),
            subscribe_cov_property_multiple(83, false, None, vec![]),
        )
        .await,
    );
    assert!(wire.multiple().await.is_empty());
    simple_ack(
        wire.send(
            &routed(),
            subscribe_cov_property_multiple(83, false, Some((900, 3)), vec![]),
        )
        .await,
    );
    assert!(
        wire.multiple().await.is_empty(),
        "empty finite request creates no context"
    );
    wire.server.stop().await.unwrap();
}

async fn observations(wire: &Wire) -> Vec<(u32, u32, Option<crate::cov::CovObservation>)> {
    let mut table = wire.server.cov_table.write().await;
    let mut result = Vec::new();
    for object in [av(1), av(2)] {
        for entry in table.subscriptions_for(&object) {
            result.push((
                object.instance_number(),
                entry.monitored_property.unwrap().to_raw(),
                entry.last_notified_observation.clone(),
            ));
        }
    }
    result.sort_by_key(|(object, property, _)| (*object, *property));
    result
}

async fn initials_complete(wire: &Wire, expected: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let values = observations(wire).await;
            if values.len() == expected
                && values
                    .iter()
                    .all(|(_, _, observation)| observation.is_some())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("initial sends completed their baselines");
}

async fn changed_value_uses(wire: &Wire, route: &Peer, object: ObjectIdentifier, value: f32) {
    use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
    use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
    wire.sent.clear();
    wire.server
        .write_local(
            &object,
            PV,
            None,
            PropertyValue::Real(value),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    let frames = wire.sent.frames();
    assert_eq!(
        frames.len(),
        1,
        "one grouped notification for the canonical context"
    );
    assert_eq!(frames[0].mac.as_slice(), route.mac.as_slice());
    let npdu = decode_npdu(frames[0].npdu.clone()).unwrap();
    assert_eq!(npdu.destination, route.network);
    let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
        panic!("unconfirmed Multiple notification");
    };
    assert_eq!(
        request.service_choice,
        UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
    );
    let notification = COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
    assert_eq!(notification.subscriber_process_identifier, 83);
    assert_eq!(notification.list_of_cov_notifications.len(), 1);
    assert_eq!(
        notification.list_of_cov_notifications[0].monitored_object_identifier,
        object
    );
}

#[tokio::test]
async fn multiple_route_rejected_wire_renewals_preserve_live_context() {
    let mut wire = Wire::start(ServerConfig {
        cov_policy: CovPolicy {
            max_subscriptions_per_peer: 1,
            ..Default::default()
        },
        ..Default::default()
    })
    .await;
    simple_ack(
        wire.send(
            &routed(),
            subscribe_cov_property_multiple(
                83,
                false,
                Some((300, 1)),
                vec![(av(1), vec![plain(PV)])],
            ),
        )
        .await,
    );
    initials_complete(&wire, 1).await;
    let before = wire
        .server
        .cov_table
        .write()
        .await
        .subscriptions_for(&av(1))
        .remove(0)
        .clone();
    // Each refuses the whole request or its first reference, which leaves
    // nothing processed and the route where it was (#1058).
    let rejected = [
        (
            subscribe_cov_property_multiple(
                83,
                false,
                Some((900, 99)),
                vec![
                    (av(99), vec![plain(PV)]),
                    (av(1), vec![plain(PropertyIdentifier::STATUS_FLAGS)]),
                ],
            ),
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ),
        (
            subscribe_cov_property_multiple(
                83,
                false,
                Some((900, 99)),
                vec![(av(1), vec![plain(MULTIPLE)])],
            ),
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ),
        (
            subscribe_cov_property_multiple(
                83,
                false,
                Some((900, 99)),
                vec![
                    (av(1), vec![plain(PropertyIdentifier::STATUS_FLAGS)]),
                    (av(2), vec![plain(PV)]),
                ],
            ),
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
        ),
        (
            out_of_range(83, 0, 0),
            ErrorClass::SERVICES,
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            out_of_range(83, 900, 900),
            ErrorClass::SERVICES,
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ];
    for (request, class, code) in rejected {
        error(wire.send(&router_b(), request).await, class, code);
        assert_live_unchanged(&wire, &before).await;
    }
    // A tag header cut short: the request is rejected as missing a
    // parameter (#1446).
    let response = wire
        .send(
            &router_b(),
            (
                ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
                BytesMut::from(&[0xff][..]),
            ),
        )
        .await;
    assert!(
        matches!(&response, Apdu::Reject(reject)
            if reject.reject_reason == RejectReason::MISSING_REQUIRED_PARAMETER),
        "{response:?}"
    );
    assert_live_unchanged(&wire, &before).await;
    changed_value_uses(&wire, &routed(), av(1), 21.0).await;
    wire.server.stop().await.unwrap();
}

async fn assert_live_unchanged(wire: &Wire, before: &crate::cov::CovSubscriptionSnapshot) {
    let table = wire.server.cov_table.read().await;
    assert!(table.is_current(before));
    assert_eq!(table.len(), 1);
    let entry = table.get_subscription(before.key()).unwrap();
    assert_eq!(entry.endpoint(), before.endpoint());
    assert_eq!(entry.expires_at, before.expires_at);
    assert_eq!(
        entry.max_notification_delay(),
        before.max_notification_delay()
    );
    assert_eq!(
        entry.last_notified_observation,
        before.last_notified_observation
    );
}

#[tokio::test]
async fn multiple_route_wire_delete_expiry_and_current_peer_cleanup_remove_migrated_refs() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    simple_ack(
        wire.send(
            &routed(),
            subscribe_cov_property_multiple(
                83,
                false,
                Some((300, 1)),
                vec![(av(1), vec![plain(PV)]), (av(2), vec![plain(PV)])],
            ),
        )
        .await,
    );
    initials_complete(&wire, 2).await;
    simple_ack(
        wire.send(
            &router_b(),
            subscribe_cov_property_multiple(83, false, Some((600, 2)), vec![]),
        )
        .await,
    );
    assert_eq!(
        wire.server
            .remove_peer_subscriptions(&routed().mac, routed().network.as_ref())
            .await,
        0
    );
    let mut delete = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: av(2),
    }
    .encode(&mut delete);
    simple_ack(
        wire.send(&direct(), (ConfirmedServiceChoice::DELETE_OBJECT, delete))
            .await,
    );
    let listed = wire.multiple().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].list_of_cov_subscription_specifications.len(), 1);
    changed_value_uses(&wire, &router_b(), av(1), 21.0).await;
    assert_eq!(
        wire.server
            .remove_peer_subscriptions(&router_b().mac, router_b().network.as_ref())
            .await,
        1
    );
    assert!(wire.multiple().await.is_empty());
    simple_ack(
        wire.send(
            &routed(),
            subscribe_cov_property_multiple(
                83,
                false,
                Some((300, 1)),
                vec![(av(1), vec![plain(PV)])],
            ),
        )
        .await,
    );
    initials_complete(&wire, 1).await;
    {
        let mut table = wire.server.cov_table.write().await;
        let old = table.subscriptions_for(&av(1))[0].clone();
        // Supply an already reached deadline through the table seam: actual
        // network reads must omit the migrated reference before periodic purge.
        table
            .subscribe_multiple(
                old.key().multiple_context().unwrap(),
                &crate::cov::SubscriberEndpoint::new(&router_b().mac, router_b().network.as_ref()),
                Instant::now(),
                0,
                None,
                vec![],
            )
            .unwrap();
    }
    assert!(wire.multiple().await.is_empty());
    wire.server.cov_table.write().await.purge_expired();
    assert!(wire.server.cov_table.read().await.is_empty());
    wire.server.stop().await.unwrap();
}
