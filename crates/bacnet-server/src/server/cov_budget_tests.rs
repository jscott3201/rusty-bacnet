use super::*;
use crate::cov::AtomicCovCounters;
use crate::server::cov_notifications_tests::recording_transport;
use crate::server::test_transport::TestTransport;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::life_safety::LifeSafetyPointObject;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_types::constructed::PropertyReference;
use bacnet_types::enums::ObjectType;

fn test_db_with_ao() -> (Arc<RwLock<ObjectDatabase>>, ObjectIdentifier) {
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    let mut db = ObjectDatabase::new();
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "COV-Test".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    device.set_object_list(vec![device_oid, ao_oid]);
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(AnalogOutputObject::new(1, "AO-1", 62).unwrap()))
        .unwrap();
    (Arc::new(RwLock::new(db)), ao_oid)
}

#[tokio::test]
async fn in_flight_failure_does_not_consume_event_budget() {
    let (transport, _sent) = recording_transport();
    let (db, ao_oid) = test_db_with_ao();
    let policy = CovPolicy {
        max_notifications_per_event: 2,
        max_confirmed_in_flight_per_peer: 1,
        ..Default::default()
    };
    let config = Arc::new(ServerConfig {
        cov_policy: policy,
        ..ServerConfig::default()
    });
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::with_policy(
        config.cov_policy.clone(),
        Arc::new(AtomicCovCounters::default()),
    )));
    let cov_in_flight = Arc::new(Semaphore::new(255));
    let transactions = NotificationTransactions::new();

    let peer1_mac = MacAddr::from_slice(&[10, 0, 0, 1]);

    {
        let mut table = cov_table.write().await;
        // Peer 1 has 3 confirmed subscriptions (max in-flight per peer is 1)
        for i in 1..=3 {
            table
                .subscribe(CovSubscription {
                    subscriber_mac: peer1_mac.clone(),
                    subscriber_network: None,
                    subscriber_process_identifier: i,
                    monitored_object_identifier: ao_oid,
                    issue_confirmed_notifications: true,
                    expires_at: None,
                    last_notified_observation: None,
                    monitored_property: None,
                    monitored_property_array_index: None,
                    cov_increment: None,
                    notification_kind: CovNotificationKind::Single,
                    timestamped: false,
                })
                .unwrap();
        }
    }

    let network = Arc::new(NetworkLayer::new(transport));

    BACnetServer::<TestTransport>::fire_cov_notifications(
        &crate::server::cov_notify_context::CovNotifyContext {
            db: &db,
            network: &network,
            cov_table: &cov_table,
            cov_in_flight: &cov_in_flight,
            notification_transactions: &transactions,
            comm_state: &Arc::new(CommState::default()),
            config: &config,
        },
        &ao_oid,
    )
    .await;

    let counters = cov_table.read().await.counters().snapshot();
    // 1 confirmed sent, 2 throttled due to in-flight peer limit.
    // In-flight failures did NOT burn event budget (max 2), so fanout throttled is 0!
    assert_eq!(counters.notifications_confirmed, 1);
    assert_eq!(counters.notifications_throttled_peer, 2);
    assert_eq!(counters.notifications_sent, 1);
    assert_eq!(counters.notifications_throttled_fanout, 0);
}

#[tokio::test]
async fn fair_distribution_between_single_and_multiple_notifications() {
    let (transport, sent) = recording_transport();
    let (db, ao_oid) = test_db_with_ao();
    let policy = CovPolicy {
        max_notifications_per_event: 2,
        ..Default::default()
    };
    let config = Arc::new(ServerConfig {
        cov_policy: policy,
        ..ServerConfig::default()
    });
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::with_policy(
        config.cov_policy.clone(),
        Arc::new(AtomicCovCounters::default()),
    )));
    let cov_in_flight = Arc::new(Semaphore::new(255));
    let transactions = NotificationTransactions::new();

    {
        let mut table = cov_table.write().await;
        // 2 Single subscriptions
        for i in 1..=2 {
            table
                .subscribe(CovSubscription {
                    subscriber_mac: MacAddr::from_slice(&[10, 0, 0, i]),
                    subscriber_network: None,
                    subscriber_process_identifier: i as u32,
                    monitored_object_identifier: ao_oid,
                    issue_confirmed_notifications: false,
                    expires_at: None,
                    last_notified_observation: None,
                    monitored_property: None,
                    monitored_property_array_index: None,
                    cov_increment: None,
                    notification_kind: CovNotificationKind::Single,
                    timestamped: false,
                })
                .unwrap();
        }
        // 2 Multiple subscriptions for different peers (2 groups)
        for i in 3..=4 {
            table
                .admit_for_test(
                    CovSubscription {
                        subscriber_mac: MacAddr::from_slice(&[10, 0, 0, i]),
                        subscriber_network: None,
                        subscriber_process_identifier: i as u32,
                        monitored_object_identifier: ao_oid,
                        issue_confirmed_notifications: false,
                        expires_at: Some(Instant::now() + Duration::from_secs(3600)),
                        last_notified_observation: None,
                        monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
                        monitored_property_array_index: None,
                        cov_increment: None,
                        notification_kind: CovNotificationKind::Multiple,
                        timestamped: false,
                    },
                    0,
                )
                .unwrap();
        }
    }

    let network = Arc::new(NetworkLayer::new(transport));

    BACnetServer::<TestTransport>::fire_cov_notifications(
        &crate::server::cov_notify_context::CovNotifyContext {
            db: &db,
            network: &network,
            cov_table: &cov_table,
            cov_in_flight: &cov_in_flight,
            notification_transactions: &transactions,
            comm_state: &Arc::new(CommState::default()),
            config: &config,
        },
        &ao_oid,
    )
    .await;

    let sent_frames = sent.lock();
    assert_eq!(sent_frames.len(), 2);
    let mut saw_single = false;
    let mut saw_multiple = false;
    for frame in sent_frames.iter() {
        let npdu = decode_npdu(frame.npdu.clone()).unwrap();
        if let Apdu::UnconfirmedRequest(req) = decode_apdu(npdu.payload).unwrap() {
            if req.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION {
                saw_single = true;
            } else if req.service_choice
                == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
            {
                saw_multiple = true;
            }
        }
    }
    assert!(saw_single, "single notifications must not starve multiples");
    assert!(saw_multiple, "multiples must receive fair budget share");
}

#[tokio::test]
async fn expired_subscription_releases_quota_on_handle_subscribe_cov() {
    let (transport, _sent) = recording_transport();
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    let mut db = ObjectDatabase::new();
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "COV-Test".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    device.set_object_list(vec![device_oid, ao_oid]);
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(AnalogOutputObject::new(1, "AO-1", 62).unwrap()))
        .unwrap();

    let policy = CovPolicy {
        max_subscriptions_per_peer: 1,
        ..Default::default()
    };
    let server = BACnetServer::generic_builder()
        .database(db)
        .transport(transport)
        .cov_policy(policy)
        .build()
        .await
        .unwrap();

    let peer = vec![10, 0, 0, 1, 0xBA, 0xC0];

    // Pre-populate 1 expired subscription for this peer
    {
        let mut table = server.cov_table.write().await;
        table
            .subscribe(CovSubscription {
                subscriber_mac: MacAddr::from_slice(&peer),
                subscriber_network: None,
                subscriber_process_identifier: 1,
                monitored_object_identifier: ao_oid,
                issue_confirmed_notifications: false,
                expires_at: Some(Instant::now() - Duration::from_secs(1)),
                last_notified_observation: None,
                monitored_property: None,
                monitored_property_array_index: None,
                cov_increment: None,
                notification_kind: CovNotificationKind::Single,
                timestamped: false,
            })
            .unwrap();
    }

    // Now send a subscribe request for a different process ID
    let req = bacnet_services::cov::SubscribeCOVRequest {
        subscriber_process_identifier: 2,
        monitored_object_identifier: ao_oid,
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    };
    let mut buf = bytes::BytesMut::new();
    req.encode(&mut buf).unwrap();
    let mut table = server.cov_table.write().await;
    let db = server.db.read().await;
    let res = crate::handlers::handle_subscribe_cov(&mut table, &db, &peer, &buf);
    assert!(
        res.is_ok(),
        "expected admission to succeed after purging expired, got {res:?}"
    );
    assert_eq!(table.len(), 1);
}

#[tokio::test]
async fn expired_subscription_purged_before_cov_property_multiple_admission() {
    let (transport, _sent) = recording_transport();
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    let mut db = ObjectDatabase::new();
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "COV-Test".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    device.set_object_list(vec![device_oid, ao_oid]);
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(AnalogOutputObject::new(1, "AO-1", 62).unwrap()))
        .unwrap();

    let policy = CovPolicy {
        max_subscriptions_per_peer: 1,
        ..Default::default()
    };
    let server = BACnetServer::generic_builder()
        .database(db)
        .transport(transport)
        .cov_policy(policy)
        .build()
        .await
        .unwrap();

    let peer = vec![10, 0, 0, 1, 0xBA, 0xC0];

    // Pre-populate 1 expired subscription for this peer matching PRESENT_VALUE
    {
        let mut table = server.cov_table.write().await;
        table
            .admit_for_test(
                CovSubscription {
                    subscriber_mac: MacAddr::from_slice(&peer),
                    subscriber_network: None,
                    subscriber_process_identifier: 1,
                    monitored_object_identifier: ao_oid,
                    issue_confirmed_notifications: false,
                    expires_at: Some(Instant::now() - Duration::from_secs(1)),
                    last_notified_observation: None,
                    monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
                    monitored_property_array_index: None,
                    cov_increment: None,
                    notification_kind: CovNotificationKind::Multiple,
                    timestamped: false,
                },
                0,
            )
            .unwrap();
    }

    // Send a SubscribeCOVPropertyMultiple request for 2 properties:
    // PRESENT_VALUE (matching the expired key) and STATUS_FLAGS.
    // Expired subscriptions must be purged before evaluating new keys, so
    // PRESENT_VALUE is new and takes the one slot of
    // max_subscriptions_per_peer, and STATUS_FLAGS is past it.
    let request = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: false,
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: ao_oid,
            list_of_cov_references: vec![
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    cov_increment: None,
                    timestamped: false,
                },
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::STATUS_FLAGS,
                        property_array_index: None,
                    },
                    cov_increment: None,
                    timestamped: false,
                },
            ],
        }],
    };
    let mut buf = bytes::BytesMut::new();
    request.encode(&mut buf).unwrap();

    let mut table = server.cov_table.write().await;
    let db = server.db.read().await;
    let res = crate::handlers::handle_subscribe_cov_property_multiple_with_initial(
        &mut table, &db, &peer, &buf,
    );
    let Err(refusal) = res else {
        panic!("expected admission rejection when new keys exceed peer quota, got {res:?}");
    };
    // The expired subscription was purged, so the request's Present_Value is
    // a new one that fits and Status_Flags is the reference refused.
    assert_eq!(refusal.refused, Some(1));
    assert_eq!(refusal.committed.len(), 1);
    assert_eq!(table.len(), 1);
    let counters = table.counters().snapshot();
    assert_eq!(counters.subscriptions_purged, 1);
    assert_eq!(counters.subscriptions_created, 2);
}

#[tokio::test]
async fn unlimited_policy_half_cap_computation_does_not_overflow() {
    let (transport, sent) = recording_transport();
    let (db, ao_oid) = test_db_with_ao();
    let config = Arc::new(ServerConfig {
        cov_policy: CovPolicy::unlimited(),
        ..ServerConfig::default()
    });
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::with_policy(
        config.cov_policy.clone(),
        Arc::new(AtomicCovCounters::default()),
    )));
    let cov_in_flight = Arc::new(Semaphore::new(255));
    let transactions = NotificationTransactions::new();

    {
        let mut table = cov_table.write().await;
        table
            .subscribe(CovSubscription {
                subscriber_mac: MacAddr::from_slice(&[10, 0, 0, 1]),
                subscriber_network: None,
                subscriber_process_identifier: 1,
                monitored_object_identifier: ao_oid,
                issue_confirmed_notifications: false,
                expires_at: None,
                last_notified_observation: None,
                monitored_property: None,
                monitored_property_array_index: None,
                cov_increment: None,
                notification_kind: CovNotificationKind::Single,
                timestamped: false,
            })
            .unwrap();
        table
            .admit_for_test(
                CovSubscription {
                    subscriber_mac: MacAddr::from_slice(&[10, 0, 0, 2]),
                    subscriber_network: None,
                    subscriber_process_identifier: 2,
                    monitored_object_identifier: ao_oid,
                    issue_confirmed_notifications: false,
                    expires_at: Some(Instant::now() + Duration::from_secs(3600)),
                    last_notified_observation: None,
                    monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
                    monitored_property_array_index: None,
                    cov_increment: None,
                    notification_kind: CovNotificationKind::Multiple,
                    timestamped: false,
                },
                0,
            )
            .unwrap();
    }

    let network = Arc::new(NetworkLayer::new(transport));

    // Under CovPolicy::unlimited(), remaining_notifications is usize::MAX.
    // The previous computation `(remaining_notifications + 1) / 2` panicked on overflow.
    // This must complete without panicking and dispatch both families.
    BACnetServer::<TestTransport>::fire_cov_notifications(
        &crate::server::cov_notify_context::CovNotifyContext {
            db: &db,
            network: &network,
            cov_table: &cov_table,
            cov_in_flight: &cov_in_flight,
            notification_transactions: &transactions,
            comm_state: &Arc::new(CommState::default()),
            config: &config,
        },
        &ao_oid,
    )
    .await;

    let sent_frames = sent.lock();
    assert_eq!(sent_frames.len(), 2);
    let mut saw_single = false;
    let mut saw_multiple = false;
    for frame in sent_frames.iter() {
        let npdu = decode_npdu(frame.npdu.clone()).unwrap();
        if let Apdu::UnconfirmedRequest(req) = decode_apdu(npdu.payload).unwrap() {
            if req.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION {
                saw_single = true;
            } else if req.service_choice
                == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
            {
                saw_multiple = true;
            }
        }
    }
    assert!(
        saw_single,
        "single notification must be dispatched under unlimited policy"
    );
    assert!(
        saw_multiple,
        "multiple notification must be dispatched under unlimited policy"
    );
}

#[tokio::test]
async fn life_safety_fair_budget_partitioning_between_single_and_multiple() {
    let (transport, sent) = recording_transport();
    let point_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let mut db = ObjectDatabase::new();
    let lsp = LifeSafetyPointObject::new(1, "point").unwrap();
    db.add(Box::new(lsp)).unwrap();
    let db = Arc::new(RwLock::new(db));

    let policy = CovPolicy {
        max_notifications_per_event: 2,
        ..Default::default()
    };
    let config = Arc::new(ServerConfig {
        cov_policy: policy,
        ..ServerConfig::default()
    });
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::with_policy(
        config.cov_policy.clone(),
        Arc::new(AtomicCovCounters::default()),
    )));
    let cov_in_flight = Arc::new(Semaphore::new(255));
    let transactions = NotificationTransactions::new();

    {
        let mut table = cov_table.write().await;
        // 2 Single subscriptions
        for i in 1..=2 {
            table
                .subscribe(CovSubscription {
                    subscriber_mac: MacAddr::from_slice(&[10, 0, 0, i]),
                    subscriber_network: None,
                    subscriber_process_identifier: i as u32,
                    monitored_object_identifier: point_oid,
                    issue_confirmed_notifications: false,
                    expires_at: None,
                    last_notified_observation: None,
                    monitored_property: None,
                    monitored_property_array_index: None,
                    cov_increment: None,
                    notification_kind: CovNotificationKind::Single,
                    timestamped: false,
                })
                .unwrap();
        }
        // 2 Multiple subscriptions for different peers
        for i in 3..=4 {
            table
                .admit_for_test(
                    CovSubscription {
                        subscriber_mac: MacAddr::from_slice(&[10, 0, 0, i]),
                        subscriber_network: None,
                        subscriber_process_identifier: i as u32,
                        monitored_object_identifier: point_oid,
                        issue_confirmed_notifications: false,
                        expires_at: Some(Instant::now() + Duration::from_secs(3600)),
                        last_notified_observation: None,
                        monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
                        monitored_property_array_index: None,
                        cov_increment: None,
                        notification_kind: CovNotificationKind::Multiple,
                        timestamped: false,
                    },
                    0,
                )
                .unwrap();
        }
    }

    let network = Arc::new(NetworkLayer::new(transport));

    BACnetServer::<TestTransport>::fire_life_safety_cov_notifications(
        &crate::server::cov_notify_context::CovNotifyContext {
            db: &db,
            network: &network,
            cov_table: &cov_table,
            cov_in_flight: &cov_in_flight,
            notification_transactions: &transactions,
            comm_state: &Arc::new(CommState::default()),
            config: &config,
        },
        &point_oid,
        &[PropertyIdentifier::PRESENT_VALUE],
    )
    .await;

    let sent_frames = sent.lock();
    assert_eq!(sent_frames.len(), 2);
    let mut saw_single = false;
    let mut saw_multiple = false;
    for frame in sent_frames.iter() {
        let npdu = decode_npdu(frame.npdu.clone()).unwrap();
        if let Apdu::UnconfirmedRequest(req) = decode_apdu(npdu.payload).unwrap() {
            if req.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION {
                saw_single = true;
            } else if req.service_choice
                == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
            {
                saw_multiple = true;
            }
        }
    }
    assert!(
        saw_single,
        "single notifications must not starve multiples in life safety"
    );
    assert!(
        saw_multiple,
        "multiples must receive fair share in life safety"
    );
}
