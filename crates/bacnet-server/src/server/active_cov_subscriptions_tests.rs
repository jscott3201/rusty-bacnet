//! Wire evidence for the live Device `Active_COV_Subscriptions` projection
//! (Clause 12.11; #813). Every read goes through the running server's
//! ReadProperty/ReadPropertyMultiple dispatch unless it names `read_local`.
use super::*;
use bacnet_services::object_mgmt::DeleteObjectRequest;
use bacnet_services::write_property::WritePropertyRequest;
use support::*;

mod support;

#[tokio::test]
async fn active_cov_wire_lists_accepted_ordinary_and_single_subscriptions() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    assert_eq!(wire.active().await, Vec::new(), "initial list is empty");

    simple_ack(
        wire.send(&direct(), subscribe_cov(11, av(1), Some(false), Some(300)))
            .await,
    );
    let single = (12, PV, None, Some(1.5), Some(true), Some(600));
    simple_ack(
        wire.send(&routed(), subscribe_cov_property(av(1), single))
            .await,
    );
    let element = (
        13,
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(8),
        None,
        Some(false),
        Some(900),
    );
    simple_ack(
        wire.send(&direct(), subscribe_cov_property(av(1), element))
            .await,
    );

    let listed = wire.active().await;
    assert_eq!(processes(&listed), vec![11, 12, 13], "{listed:?}");
    let ordinary = find(&listed, 11);
    assert_eq!(ordinary.recipient.recipient, address(&direct()));
    // Whole-object entries name the Clause 13.1 monitored Present_Value.
    assert_eq!(
        ordinary.monitored_property_reference,
        BACnetObjectPropertyReference::new(av(1), PV.to_raw())
    );
    assert!(!ordinary.issue_confirmed_notifications);
    assert!((299..=300).contains(&ordinary.time_remaining));
    // Ordinary numeric Present_Value reports the object's COV_Increment in use.
    assert_eq!(ordinary.cov_increment, Some(0.0));

    let single = find(&listed, 12);
    // Routed subscribers report the remote NPDU source, never the router MAC.
    assert_eq!(single.recipient.recipient, address(&routed()));
    assert_eq!(
        single.monitored_property_reference,
        BACnetObjectPropertyReference::new(av(1), PV.to_raw())
    );
    assert!(single.issue_confirmed_notifications);
    assert!((599..=600).contains(&single.time_remaining));
    assert_eq!(single.cov_increment, Some(1.5));

    let element = find(&listed, 13);
    assert_eq!(
        element.monitored_property_reference,
        BACnetObjectPropertyReference::new_indexed(
            av(1),
            PropertyIdentifier::PRIORITY_ARRAY.to_raw(),
            8
        )
    );
    // A non-numeric selected element carries no increment.
    assert_eq!(element.cov_increment, None);

    // The increment in use follows the object without a second cache; an
    // explicit override is unaffected.
    wire.server
        .write_local(
            &av(1),
            PropertyIdentifier::COV_INCREMENT,
            None,
            PropertyValue::Real(2.5),
            None,
        )
        .await
        .unwrap();
    let listed = wire.active().await;
    assert_eq!(find(&listed, 11).cov_increment, Some(2.5));
    assert_eq!(find(&listed, 12).cov_increment, Some(1.5));
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn active_cov_wire_renewal_cancellation_and_indefinite_lifetime() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    simple_ack(
        wire.send(&direct(), subscribe_cov(21, av(1), Some(false), Some(300)))
            .await,
    );
    let single = (21, PV, None, None, Some(false), Some(60));
    simple_ack(
        wire.send(&direct(), subscribe_cov_property(av(1), single))
            .await,
    );
    // Renewal of the ordinary identity replaces its mutable form and lifetime.
    simple_ack(
        wire.send(&direct(), subscribe_cov(21, av(1), Some(true), None))
            .await,
    );
    // Same process/object, two families: the renewed ordinary entry (indefinite
    // is wire zero, confirmed) and the untouched finite unconfirmed Single entry.
    let forms = |listed: &[BACnetCOVSubscription]| {
        let mut forms: Vec<_> = listed
            .iter()
            .map(|entry| {
                assert_eq!(entry.recipient.process_identifier, 21);
                assert_eq!(
                    entry.monitored_property_reference,
                    BACnetObjectPropertyReference::new(av(1), PV.to_raw())
                );
                (
                    entry.issue_confirmed_notifications,
                    entry.time_remaining > 0,
                )
            })
            .collect();
        forms.sort_unstable();
        forms
    };
    let listed = wire.active().await;
    assert_eq!(
        forms(&listed),
        vec![(false, true), (true, false)],
        "{listed:?}"
    );
    assert!(listed.iter().all(|entry| entry.time_remaining <= 60));

    // Exact cancellation removes only the ordinary family entry.
    simple_ack(
        wire.send(&direct(), subscribe_cov(21, av(1), None, None))
            .await,
    );
    assert_eq!(forms(&wire.active().await), vec![(false, true)]);
    let cancel_single = (21, PV, None, None, None, None);
    simple_ack(
        wire.send(&direct(), subscribe_cov_property(av(1), cancel_single))
            .await,
    );
    assert_eq!(wire.active().await, Vec::new());

    // Indefinite ordinary entry; the vector is hand-assembled from Clause 20/21.
    simple_ack(
        wire.send(&direct(), subscribe_cov(22, av(1), Some(true), None))
            .await,
    );
    #[rustfmt::skip]
    let expected = [
        0x0E, 0x0E, 0x1E, 0x21, 0x00, 0x65, 0x06, 0x0A, 0x00, 0x00, 0x05, 0xBA, 0xC0, 0x1F,
        0x0F, 0x19, 0x16, 0x0F, 0x1E, 0x0C, 0x00, 0x80, 0x00, 0x01, 0x19, 0x55, 0x1F, 0x29,
        0x01, 0x39, 0x00, 0x4C, 0x00, 0x00, 0x00, 0x00,
    ];
    assert_eq!(wire.active_bytes().await, expected);
    // Cancelling an absent identity succeeds and changes nothing.
    simple_ack(
        wire.send(&direct(), subscribe_cov(21, av(1), None, None))
            .await,
    );
    assert_eq!(wire.active_bytes().await, expected);
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn active_cov_wire_expiry_delete_object_and_peer_cleanup_remove_entries() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    simple_ack(
        wire.send(&direct(), subscribe_cov(31, av(2), Some(false), Some(300)))
            .await,
    );
    let single = (32, PV, None, None, Some(false), Some(300));
    simple_ack(
        wire.send(&direct(), subscribe_cov_property(av(1), single))
            .await,
    );
    let routed_single = (33, PV, None, None, Some(false), Some(300));
    simple_ack(
        wire.send(&routed(), subscribe_cov_property(av(1), routed_single))
            .await,
    );
    // A sub-second lifetime is table-only; the wire minimum is one second.
    wire.server
        .cov_table
        .write()
        .await
        .subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&direct().mac),
            subscriber_network: None,
            subscriber_process_identifier: 34,
            monitored_object_identifier: av(1),
            issue_confirmed_notifications: false,
            expires_at: Some(Instant::now() + Duration::from_millis(300)),
            last_notified_observation: None,
            monitored_property: None,
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        })
        .unwrap();
    let listed = wire.active().await;
    assert_eq!(processes(&listed), vec![31, 32, 33, 34]);
    assert_eq!(
        find(&listed, 34).time_remaining,
        1,
        "positive fractions round up"
    );

    // Expired at the sampled instant: omitted before the periodic purge runs.
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(processes(&wire.active().await), vec![31, 32, 33]);
    assert_eq!(wire.server.cov_table.read().await.len(), 4, "not purged");

    let mut delete = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: av(2),
    }
    .encode(&mut delete);
    simple_ack(
        wire.send(&direct(), (ConfirmedServiceChoice::DELETE_OBJECT, delete))
            .await,
    );
    assert_eq!(processes(&wire.active().await), vec![32, 33]);

    // Peer cleanup is exact: another router carrying the same source keeps it.
    let source = routed().network;
    assert_eq!(
        wire.server
            .remove_peer_subscriptions(&[0xFF], source.as_ref())
            .await,
        0
    );
    assert_eq!(processes(&wire.active().await), vec![32, 33]);
    assert_eq!(
        wire.server
            .remove_peer_subscriptions(&routed().mac, source.as_ref())
            .await,
        1
    );
    let listed = wire.active().await;
    assert_eq!(processes(&listed), vec![32]);
    assert_eq!(find(&listed, 32).recipient.recipient, address(&direct()));
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn active_cov_wire_rejected_requests_leave_the_list_unchanged() {
    let config = ServerConfig {
        cov_policy: CovPolicy {
            max_subscriptions_per_peer: 2,
            ..CovPolicy::default()
        },
        ..ServerConfig::default()
    };
    let mut wire = Wire::start(config).await;
    simple_ack(
        wire.send(&direct(), subscribe_cov(41, av(1), Some(false), None))
            .await,
    );
    simple_ack(
        wire.send(&direct(), subscribe_cov(42, av(2), Some(true), None))
            .await,
    );
    let baseline = wire.active_bytes().await;
    assert_eq!(processes(&decode_subscriptions(&baseline)), vec![41, 42]);

    let rejected = [
        (
            subscribe_cov(43, av(1), Some(false), Some(300)),
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
        ),
        (
            subscribe_cov(44, device(), Some(false), Some(300)),
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        ),
        (
            subscribe_cov(45, av(99), Some(false), Some(300)),
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ),
        (
            subscribe_cov_property(av(1), (46, ACTIVE, None, None, Some(false), Some(300))),
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ),
    ];
    for (request, class, code) in rejected {
        error(wire.send(&direct(), request).await, class, code);
        assert_eq!(wire.active_bytes().await, baseline);
    }
    // Renewal at quota is admitted and still leaves exactly one entry.
    simple_ack(
        wire.send(&direct(), subscribe_cov(41, av(1), Some(false), None))
            .await,
    );
    assert_eq!(wire.active_bytes().await, baseline);
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn active_cov_wire_rpm_rows_share_one_request_snapshot_and_agree_with_rp() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    simple_ack(
        wire.send(&direct(), subscribe_cov(51, av(1), Some(true), None))
            .await,
    );
    let single = (52, PV, None, Some(0.25), Some(false), Some(300));
    simple_ack(
        wire.send(&routed(), subscribe_cov_property(av(1), single))
            .await,
    );

    let ack = wire
        .rpm(vec![
            (
                device(),
                vec![
                    (ACTIVE, None),
                    (ACTIVE, None),
                    (PropertyIdentifier::ALL, None),
                    (PropertyIdentifier::OPTIONAL, None),
                ],
            ),
            (wildcard(), vec![(ACTIVE, None)]),
            (av(1), vec![(PV, None)]),
        ])
        .await;
    let rows = active_rows(&ack);
    assert_eq!(
        rows.len(),
        5,
        "explicit x2, ALL, OPTIONAL and wildcard rows"
    );
    assert!(rows.iter().all(|row| row == &rows[0]), "one snapshot");
    let rpm = decode_subscriptions(&rows[0]);
    let rp = wire.active().await;
    assert_eq!(processes(&rpm), vec![51, 52]);
    let without_time = |entries: &[BACnetCOVSubscription]| {
        entries
            .iter()
            .map(|entry| BACnetCOVSubscription {
                time_remaining: entry.time_remaining.min(1),
                ..entry.clone()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(without_time(&rpm), without_time(&rp), "RP and RPM agree");
    assert_eq!(find(&rpm, 52).cov_increment, Some(0.25));

    // Renewal/cancellation racing repeated references never splits one response.
    let toggler = {
        let tx = wire.tx.clone();
        tokio::spawn(async move {
            for invoke_id in 100..160u8 {
                let request = if invoke_id % 2 == 0 {
                    subscribe_cov(99, av(2), Some(false), Some(300))
                } else {
                    subscribe_cov(99, av(2), None, None)
                };
                simple_ack(exchange(&tx, &routed(), invoke_id, request).await);
            }
        })
    };
    for _ in 0..30 {
        let ack = wire
            .rpm(vec![
                (
                    device(),
                    vec![
                        (ACTIVE, None),
                        (ACTIVE, None),
                        (PropertyIdentifier::ALL, None),
                    ],
                ),
                (wildcard(), vec![(ACTIVE, None)]),
            ])
            .await;
        let rows = active_rows(&ack);
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|row| row == &rows[0]), "one snapshot");
    }
    toggler.await.unwrap();
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn active_cov_read_only_list_scope_local_read_and_stop() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    simple_ack(
        wire.send(&direct(), subscribe_cov(61, av(1), Some(false), None))
            .await,
    );
    let live = wire.active_bytes().await;
    assert_eq!(processes(&decode_subscriptions(&live)), vec![61]);

    let mut write = BytesMut::new();
    WritePropertyRequest {
        object_identifier: device(),
        property_identifier: ACTIVE,
        property_array_index: None,
        property_value: vec![0x00],
        priority: None,
    }
    .encode(&mut write)
    .unwrap();
    let write = (ConfirmedServiceChoice::WRITE_PROPERTY, write);
    let response = wire.send(&direct(), write).await;
    error(
        response,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(wire.active_bytes().await, live);

    let not_array = (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY);
    assert_eq!(wire.read(device(), ACTIVE, Some(1)).await, Err(not_array));
    let ack = wire.rpm(vec![(device(), vec![(ACTIVE, Some(0))])]).await;
    let row = &ack.list_of_read_access_results[0].list_of_results[0];
    assert_eq!(
        (row.error, row.property_array_index),
        (Some(not_array), None)
    );

    // Only the selected local Device is live; another Device keeps its
    // standalone default list.
    let other = ObjectIdentifier::new(ObjectType::DEVICE, 900).unwrap();
    let config = DeviceConfig {
        instance: 900,
        name: "Other Device".into(),
        ..DeviceConfig::default()
    };
    wire.server
        .database()
        .write()
        .await
        .add(Box::new(DeviceObject::new(config).unwrap()))
        .unwrap();
    let selected =
        handlers::resolve_device_wildcard(&*wire.server.database().read().await, &wildcard());
    let (live_device, standalone) = if selected == device() {
        (device(), other)
    } else {
        (other, device())
    };
    assert_eq!(wire.read(live_device, ACTIVE, None).await.unwrap(), live);
    assert_eq!(wire.read(wildcard(), ACTIVE, None).await.unwrap(), live);
    assert_eq!(
        wire.read(standalone, ACTIVE, None).await.unwrap(),
        Vec::<u8>::new()
    );

    // The local entrypoint shares the evaluator; raw object reads stay standalone.
    let local = PropertyValue::ApplicationData(live.clone());
    for oid in [live_device, wildcard()] {
        assert_eq!(
            wire.server.read_local(&oid, ACTIVE, None).await.unwrap(),
            local
        );
    }
    let indexed = wire.server.read_local(&live_device, ACTIVE, Some(1)).await;
    assert!(matches!(indexed, Err(Error::Protocol { code, .. })
        if code == ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32));
    let raw = wire
        .server
        .database()
        .read()
        .await
        .get(&live_device)
        .unwrap()
        .read_property(ACTIVE, None);
    assert_eq!(raw.unwrap(), PropertyValue::ApplicationData(Vec::new()));

    // A stopped server services no subscription and reports none; the table
    // is not consulted as a stale fallback.
    wire.server.stop().await.unwrap();
    assert_eq!(wire.server.cov_table.read().await.len(), 1);
    assert_eq!(
        wire.server
            .read_local(&live_device, ACTIVE, None)
            .await
            .unwrap(),
        PropertyValue::ApplicationData(Vec::new())
    );
    assert_eq!(
        wire.server
            .read_local(&live_device, PropertyIdentifier::OBJECT_IDENTIFIER, None)
            .await
            .unwrap(),
        PropertyValue::ObjectIdentifier(live_device)
    );
}
