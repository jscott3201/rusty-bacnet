use bacnet_types::error::RemoteError;
use std::time::{Duration, Instant};

use bytes::BytesMut;

use bacnet_encoding::npdu::NpduAddress;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_types::MacAddr;
use bacnet_types::constructed::PropertyReference;
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::cov_notifications_tests::recording_transport;
use super::*;
use crate::cov::{CovNotificationKind, CovPolicy, CovSubscription};
use crate::handlers::{handle_subscribe_cov, handle_subscribe_cov_property_multiple_with_initial};
use crate::server::test_transport::TestTransport;

fn ai(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn create_database_with_ais(count: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let dev = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(100, 0)
            .expect("valid Device identity")
            .with_name("Device-100".to_string()),
        ..Default::default()
    })
    .unwrap();
    db.add(Box::new(dev)).unwrap();
    for i in 1..=count {
        let ai_obj = bacnet_objects::analog::AnalogInputObject::new(
            i,
            format!("AI-{}", i),
            EngineeringUnits::NO_UNITS,
        )
        .unwrap();
        db.add(Box::new(ai_obj)).unwrap();
    }
    db
}

// ---------------------------------------------------------------------------
// 1. One peer reaching its quota does not prevent another peer from subscribing
// ---------------------------------------------------------------------------
#[tokio::test]
async fn peer_quota_isolation() {
    let db = create_database_with_ais(10);
    let policy = CovPolicy {
        max_subscriptions_per_peer: 2,
        max_subscriptions_global: 10,
        reserved_capacity: 0,
        ..Default::default()
    };

    let server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(100, 0).unwrap())
        .database(db)
        .transport(recording_transport().0)
        .cov_policy(policy)
        .build()
        .await
        .unwrap();

    let peer_a = vec![10, 0, 0, 1, 0xBA, 0xC0];
    let peer_b = vec![10, 0, 0, 2, 0xBA, 0xC0];

    // Peer A subscribes to AI:1 and AI:2 (reaches its quota of 2)
    for i in 1..=2 {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: ai(i),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server.cov_table.write().await;
        let db = server.db.read().await;
        handle_subscribe_cov(&mut table, &db, &peer_a, &buf).unwrap();
    }

    // Peer A attempts to subscribe to AI:3 -> rejected by quota
    {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: ai(3),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server.cov_table.write().await;
        let db = server.db.read().await;
        let err = handle_subscribe_cov(&mut table, &db, &peer_a, &buf).unwrap_err();
        match err {
            Error::Remote(RemoteError::Error { class, code }) => {
                assert_eq!(class, ErrorClass::RESOURCES);
                assert_eq!(code, ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT);
            }
            other => panic!("expected RESOURCES / NO_SPACE_TO_ADD_LIST_ELEMENT, got {other:?}"),
        }
    }

    // Telemetry: subscriptions_rejected_quota should be 1
    assert_eq!(server.cov_counters().subscriptions_rejected_quota, 1);

    // Peer B subscribes to AI:1 -> SUCCEEDS (not starved by Peer A reaching quota)
    {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: ai(1),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server.cov_table.write().await;
        let db = server.db.read().await;
        handle_subscribe_cov(&mut table, &db, &peer_b, &buf).unwrap();
    }

    let counters = server.cov_counters();
    assert_eq!(counters.subscriptions_active, 3);
    assert_eq!(counters.subscriptions_created, 3);
    assert_eq!(counters.subscriptions_rejected_quota, 1);
}

// ---------------------------------------------------------------------------
// 2. Reserved capacity is preserved: unreserved cannot consume; reserved can
// ---------------------------------------------------------------------------
#[tokio::test]
async fn reserved_capacity_preservation() {
    let db = create_database_with_ais(10);
    let reserved_mac = MacAddr::from_slice(&[192, 168, 1, 99]);
    let policy = CovPolicy {
        max_subscriptions_global: 5,
        reserved_capacity: 2,
        max_subscriptions_per_peer: 10,
        reserved_peers: vec![reserved_mac.clone()],
        ..Default::default()
    };

    let server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(100, 0).unwrap())
        .database(db)
        .transport(recording_transport().0)
        .cov_policy(policy)
        .build()
        .await
        .unwrap();

    let unreserved_peer = vec![10, 0, 0, 1, 0xBA, 0xC0];

    // Unreserved ceiling is 5 - 2 = 3. Fill the 3 unreserved slots.
    for i in 1..=3 {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: i,
            monitored_object_identifier: ai(i),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server.cov_table.write().await;
        let db = server.db.read().await;
        handle_subscribe_cov(&mut table, &db, &unreserved_peer, &buf).unwrap();
    }

    // 4th subscription from unreserved peer is rejected (preserves reserved headroom)
    {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 4,
            monitored_object_identifier: ai(4),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server.cov_table.write().await;
        let db = server.db.read().await;
        let err = handle_subscribe_cov(&mut table, &db, &unreserved_peer, &buf).unwrap_err();
        match err {
            Error::Remote(RemoteError::Error { class, code }) => {
                assert_eq!(class, ErrorClass::RESOURCES);
                assert_eq!(code, ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT);
            }
            other => panic!("expected RESOURCES / NO_SPACE_TO_ADD_LIST_ELEMENT, got {other:?}"),
        }
    }
    assert_eq!(server.cov_counters().subscriptions_rejected_capacity, 1);

    // Reserved peer CAN subscribe into the reserved headroom (slot 4 and slot 5)
    for i in 4..=5 {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: i,
            monitored_object_identifier: ai(i),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server.cov_table.write().await;
        let db = server.db.read().await;
        handle_subscribe_cov(&mut table, &db, reserved_mac.as_slice(), &buf).unwrap();
    }
    assert_eq!(server.cov_counters().subscriptions_active, 5);

    // Reserved peer attempts slot 6 -> rejected by global capacity
    {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 6,
            monitored_object_identifier: ai(6),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server.cov_table.write().await;
        let db = server.db.read().await;
        let err = handle_subscribe_cov(&mut table, &db, reserved_mac.as_slice(), &buf).unwrap_err();
        match err {
            Error::Remote(RemoteError::Error { class, code }) => {
                assert_eq!(class, ErrorClass::RESOURCES);
                assert_eq!(code, ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT);
            }
            other => panic!("expected RESOURCES / NO_SPACE_TO_ADD_LIST_ELEMENT, got {other:?}"),
        }
    }
    assert_eq!(server.cov_counters().subscriptions_rejected_capacity, 2);
}

// ---------------------------------------------------------------------------
// 3. Indefinite subscriptions follow explicit configured policy
// ---------------------------------------------------------------------------
#[tokio::test]
async fn indefinite_subscription_policy() {
    let db = create_database_with_ais(10);
    let policy = CovPolicy {
        allow_indefinite_subscriptions: false,
        ..Default::default()
    };

    let server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(100, 0).unwrap())
        .database(db)
        .transport(recording_transport().0)
        .cov_policy(policy)
        .build()
        .await
        .unwrap();

    let peer = vec![10, 0, 0, 1, 0xBA, 0xC0];

    // Indefinite subscription (lifetime: None) rejected when disallowed
    {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: ai(1),
            issue_confirmed_notifications: Some(false),
            lifetime: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server.cov_table.write().await;
        let db = server.db.read().await;
        let err = handle_subscribe_cov(&mut table, &db, &peer, &buf).unwrap_err();
        match err {
            Error::Remote(RemoteError::Error { class, code }) => {
                assert_eq!(class, ErrorClass::SERVICES);
                assert_eq!(code, ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED);
            }
            other => {
                panic!("expected SERVICES / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, got {other:?}")
            }
        }
    }
    assert_eq!(server.cov_counters().subscriptions_rejected_indefinite, 1);

    // Definite subscription succeeds when indefinite is disallowed
    {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: ai(1),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server.cov_table.write().await;
        let db = server.db.read().await;
        handle_subscribe_cov(&mut table, &db, &peer, &buf).unwrap();
    }
    assert_eq!(server.cov_counters().subscriptions_active, 1);

    // Now test max_indefinite_per_peer when allow_indefinite is true
    let db2 = create_database_with_ais(10);
    let policy2 = CovPolicy {
        allow_indefinite_subscriptions: true,
        max_indefinite_per_peer: 1,
        ..Default::default()
    };
    let server2 = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(100, 0).unwrap())
        .database(db2)
        .transport(recording_transport().0)
        .cov_policy(policy2)
        .build()
        .await
        .unwrap();

    // 1st indefinite succeeds
    {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: ai(1),
            issue_confirmed_notifications: Some(false),
            lifetime: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server2.cov_table.write().await;
        let db = server2.db.read().await;
        handle_subscribe_cov(&mut table, &db, &peer, &buf).unwrap();
    }

    // 2nd indefinite from same peer is rejected by indefinite quota
    {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 2,
            monitored_object_identifier: ai(2),
            issue_confirmed_notifications: Some(false),
            lifetime: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server2.cov_table.write().await;
        let db = server2.db.read().await;
        let err = handle_subscribe_cov(&mut table, &db, &peer, &buf).unwrap_err();
        match err {
            Error::Remote(RemoteError::Error { class, code }) => {
                assert_eq!(class, ErrorClass::RESOURCES);
                assert_eq!(code, ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT);
            }
            other => panic!("expected RESOURCES / NO_SPACE_TO_ADD_LIST_ELEMENT, got {other:?}"),
        }
    }
    assert_eq!(server2.cov_counters().subscriptions_rejected_indefinite, 1);
}

// ---------------------------------------------------------------------------
// 4. Disconnect/expiry cleanup releases quota deterministically
// ---------------------------------------------------------------------------
#[tokio::test]
async fn disconnect_and_expiry_cleanup_releases_quota() {
    let db = create_database_with_ais(10);
    let policy = CovPolicy {
        max_subscriptions_per_peer: 2,
        ..Default::default()
    };

    let server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(100, 0).unwrap())
        .database(db)
        .transport(recording_transport().0)
        .cov_policy(policy)
        .build()
        .await
        .unwrap();

    let peer = vec![10, 0, 0, 1, 0xBA, 0xC0];

    // Peer creates 2 subscriptions (reaches quota)
    for i in 1..=2 {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: i,
            monitored_object_identifier: ai(i),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server.cov_table.write().await;
        let db = server.db.read().await;
        handle_subscribe_cov(&mut table, &db, &peer, &buf).unwrap();
    }
    assert_eq!(server.cov_counters().subscriptions_active, 2);

    // 3rd subscription rejected
    {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 3,
            monitored_object_identifier: ai(3),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server.cov_table.write().await;
        let db = server.db.read().await;
        assert!(handle_subscribe_cov(&mut table, &db, &peer, &buf).is_err());
    }

    // Explicit disconnect cleanup releases all subscriptions for the peer
    let removed = server.remove_peer_subscriptions(&peer, None).await;
    assert_eq!(removed, 2);
    assert_eq!(server.cov_counters().subscriptions_active, 0);
    assert_eq!(server.cov_counters().subscriptions_cancelled, 2);

    // Peer can immediately subscribe again
    {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: ai(1),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let mut table = server.cov_table.write().await;
        let db = server.db.read().await;
        handle_subscribe_cov(&mut table, &db, &peer, &buf).unwrap();
    }
    assert_eq!(server.cov_counters().subscriptions_active, 1);

    // Test routed peer isolation in remove_peer_subscriptions
    let router_mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let routed_a = NpduAddress {
        network: 10,
        mac_address: MacAddr::from_slice(&[1]),
    };
    let routed_b = NpduAddress {
        network: 10,
        mac_address: MacAddr::from_slice(&[2]),
    };

    {
        let mut table = server.cov_table.write().await;
        table
            .subscribe(CovSubscription {
                subscriber_mac: MacAddr::from_slice(&router_mac),
                subscriber_network: Some(routed_a.clone()),
                subscriber_process_identifier: 1,
                monitored_object_identifier: ai(1),
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
            .subscribe(CovSubscription {
                subscriber_mac: MacAddr::from_slice(&router_mac),
                subscriber_network: Some(routed_b.clone()),
                subscriber_process_identifier: 1,
                monitored_object_identifier: ai(1),
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

    // Removing routed_a removes only routed_a, preserving routed_b
    let removed_routed = server
        .remove_peer_subscriptions(&router_mac, Some(&routed_a))
        .await;
    assert_eq!(removed_routed, 1);

    // Verify routed_b remains active
    {
        let table = server.cov_table.read().await;
        assert!(table.contains(&crate::cov::CovSubscriptionKey::Object {
            recipient: crate::cov::CovRecipient::from_endpoint(
                &MacAddr::from_slice(&router_mac),
                Some(&routed_b)
            ),
            process_id: 1,
            object: ai(1)
        }));
    }

    // Test expiry cleanup via purge_expired
    {
        let mut table = server.cov_table.write().await;
        let sub = CovSubscription {
            subscriber_mac: MacAddr::from_slice(&[10, 0, 0, 99]),
            subscriber_network: None,
            subscriber_process_identifier: 1,
            monitored_object_identifier: ai(5),
            issue_confirmed_notifications: false,
            expires_at: Some(Instant::now() - Duration::from_secs(5)),
            last_notified_observation: None,
            monitored_property: None,
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        };
        table.subscribe(sub).unwrap();
        let purged = table.purge_expired();
        assert_eq!(purged, 1);
        assert_eq!(server.cov_counters().subscriptions_purged, 1);
    }
}

// ---------------------------------------------------------------------------
// 5. Maximum-fanout object change respects work/fanout budgets
// ---------------------------------------------------------------------------
#[tokio::test]
async fn fanout_and_work_budgets_enforced() {
    let db = create_database_with_ais(2);
    let policy = CovPolicy {
        max_notifications_per_event: 3,
        max_notification_bytes_per_event: 65_536,
        max_confirmed_in_flight_per_peer: 16,
        ..Default::default()
    };

    let server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(100, 0).unwrap())
        .database(db)
        .transport(recording_transport().0)
        .cov_policy(policy)
        .build()
        .await
        .unwrap();

    // Register 6 subscriptions on AI:1 from different peers
    {
        let mut table = server.cov_table.write().await;
        for i in 1..=6 {
            table
                .subscribe(CovSubscription {
                    subscriber_mac: MacAddr::from_slice(&[10, 0, 0, i, 0xBA, 0xC0]),
                    subscriber_network: None,
                    subscriber_process_identifier: 1,
                    monitored_object_identifier: ai(1),
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
    }

    // Fire notifications for AI:1
    BACnetServer::<TestTransport>::fire_cov_notifications(
        &crate::server::cov_notify_context::CovNotifyContext {
            db: &server.db,
            network: server.test_network(),
            cov_table: &server.cov_table,
            cov_in_flight: &server.cov_in_flight,
            notification_transactions: &server.notification_transactions,
            comm_state: &server.comm_state,
            config: &server.config,
        },
        &ai(1),
    )
    .await;

    let counters = server.cov_counters();
    // Exactly 3 notifications emitted (fanout budget), 3 throttled
    assert_eq!(counters.notifications_sent, 3);
    assert_eq!(counters.notifications_unconfirmed, 3);
    assert_eq!(counters.notifications_throttled_fanout, 3);
    assert!(counters.notification_bytes_sent > 0);
}

// ---------------------------------------------------------------------------
// 6. Confirmed notification in-flight per peer throttled
// ---------------------------------------------------------------------------
#[tokio::test]
async fn in_flight_confirmed_per_peer_throttled() {
    let db = create_database_with_ais(2);
    let policy = CovPolicy {
        max_notifications_per_event: 64,
        max_confirmed_in_flight_per_peer: 2,
        ..Default::default()
    };

    let server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(100, 0).unwrap())
        .database(db)
        .transport(recording_transport().0)
        .cov_policy(policy)
        .build()
        .await
        .unwrap();

    let peer = MacAddr::from_slice(&[10, 0, 0, 1, 0xBA, 0xC0]);

    // 4 confirmed subscriptions for the same peer on AI:1 (distinct process IDs)
    {
        let mut table = server.cov_table.write().await;
        for i in 1..=4 {
            table
                .subscribe(CovSubscription {
                    subscriber_mac: peer.clone(),
                    subscriber_network: None,
                    subscriber_process_identifier: i,
                    monitored_object_identifier: ai(1),
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

    BACnetServer::<TestTransport>::fire_cov_notifications(
        &crate::server::cov_notify_context::CovNotifyContext {
            db: &server.db,
            network: server.test_network(),
            cov_table: &server.cov_table,
            cov_in_flight: &server.cov_in_flight,
            notification_transactions: &server.notification_transactions,
            comm_state: &server.comm_state,
            config: &server.config,
        },
        &ai(1),
    )
    .await;

    let counters = server.cov_counters();
    // In-flight per peer was 2, so 2 spawned and 2 throttled due to peer in-flight limit
    assert_eq!(counters.notifications_confirmed, 2);
    assert_eq!(counters.notifications_throttled_peer, 2);
}

// ---------------------------------------------------------------------------
// 7. Multi-property admission stops at the reference past the peer quota
// ---------------------------------------------------------------------------
#[tokio::test]
async fn subscribe_cov_property_multiple_quota_names_the_overflowing_reference() {
    let db = create_database_with_ais(5);
    let policy = CovPolicy {
        max_subscriptions_per_peer: 3,
        max_subscriptions_global: 10,
        ..Default::default()
    };

    let server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(100, 0).unwrap())
        .database(db)
        .transport(recording_transport().0)
        .cov_policy(policy)
        .build()
        .await
        .unwrap();

    let peer = vec![10, 0, 0, 1, 0xBA, 0xC0];

    // Pre-populate 2 subscriptions for this peer
    {
        let mut table = server.cov_table.write().await;
        for i in 1..=2 {
            table
                .admit_for_test(
                    CovSubscription {
                        subscriber_mac: MacAddr::from_slice(&peer),
                        subscriber_network: None,
                        subscriber_process_identifier: 1,
                        monitored_object_identifier: ai(i),
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

    // Now request 2 more properties in a batch: the first takes the peer's
    // third and last slot, and the second is past max_subscriptions_per_peer.
    let request = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: false,
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: ai(3),
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
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let mut table = server.cov_table.write().await;
    let db = server.db.read().await;
    let refusal = handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &peer, &buf)
        .unwrap_err();

    match refusal.error {
        Error::Structured {
            class,
            code,
            detail,
        } => {
            assert_eq!(class, ErrorClass::RESOURCES);
            assert_eq!(code, ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT);
            assert_eq!(
                *detail,
                bacnet_types::error::ErrorDetail::FirstFailedSubscription(
                    bacnet_types::constructed::BACnetObjectPropertyReference::new(
                        ai(3),
                        PropertyIdentifier::STATUS_FLAGS.to_raw()
                    )
                )
            );
        }
        other => panic!("expected RESOURCES / NO_SPACE_TO_ADD_LIST_ELEMENT, got {other:?}"),
    }

    // Present_Value, admitted before the quota ran out, stays (#1058, #1059).
    assert_eq!(refusal.refused, Some(1));
    assert_eq!(refusal.committed.len(), 1);
    assert_eq!(table.len(), 3);
    assert_eq!(table.counters().snapshot().subscriptions_rejected_quota, 1);
}
