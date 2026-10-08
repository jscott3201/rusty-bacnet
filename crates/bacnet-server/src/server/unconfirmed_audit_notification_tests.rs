use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex as StdMutex};

use bacnet_objects::device::DeviceConfig;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::enums::AuditOperation;

use super::audit_notification_tests::{
    count, database, database_with_device, notification, oid, request_bytes, MemoryPersistence,
};
use super::*;
use crate::server::test_transport::{SendLog, TestTransport};

fn received(
    source_mac: &[u8],
    source_network: Option<NpduAddress>,
) -> bacnet_network::layer::ReceivedApdu {
    bacnet_network::layer::ReceivedApdu {
        direct_response: None,
        apdu: Bytes::new(),
        source_mac: MacAddr::from_slice(source_mac),
        ingress_network: None,
        source_network,
        link_layer_group: false,
        is_group: false,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

/// Dispatch one request through a fresh link and return that link's log of
/// every unicast and broadcast send.
async fn dispatch_unconfirmed(
    db: &Arc<RwLock<ObjectDatabase>>,
    config: &ServerConfig,
    comm_state: &Arc<CommState>,
    source_mac: &[u8],
    source_network: Option<NpduAddress>,
    service_request: Bytes,
) -> SendLog {
    let transport = TestTransport::builder().local_mac(&[0]).build();
    let sends = transport.sent();
    let network = Arc::new(NetworkLayer::new(transport));
    BACnetServer::<TestTransport>::handle_unconfirmed_request(
        &UnconfirmedServices {
            db: Arc::clone(db),
            comm_state: Arc::clone(comm_state),
            ..UnconfirmedServices::for_test(Arc::clone(&network), config.clone())
        },
        UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION,
            service_request,
        },
        &received(source_mac, source_network),
    )
    .await;
    sends
}

async fn assert_silent_drop(
    db: &Arc<RwLock<ObjectDatabase>>,
    sink: ObjectIdentifier,
    config: &ServerConfig,
    service_request: Bytes,
) {
    let before = count(db, sink).await;
    let sends =
        dispatch_unconfirmed(db, config, &Arc::default(), &[1], None, service_request).await;
    assert_eq!(count(db, sink).await, before);
    assert_eq!(sends.len(), 0, "a silently dropped request must not send");
}

#[tokio::test]
async fn accepted_direct_and_routed_requests_commit_atomically_without_output() {
    let persistence = Arc::new(MemoryPersistence::default());
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let db = database(Arc::clone(&persistence), 7);
    let contexts = Arc::new(StdMutex::new(Vec::new()));
    let observed = Arc::clone(&contexts);
    let config = ServerConfig {
        audit_notification_sink: Some(sink),
        unconfirmed_audit_notification_authorizer: Some(Arc::new(move |context| {
            observed.lock().unwrap().push(context.clone());
            true
        })),
        ..ServerConfig::default()
    };
    let routed = NpduAddress {
        network: 55,
        mac_address: MacAddr::from_slice(&[0xaa]),
    };
    let comm_state = Arc::new(CommState::default());

    let source = notification(AuditOperation::WRITE);
    let mut target = source.clone();
    target.source_timestamp = None;
    target.target_timestamp = source.source_timestamp.clone();
    let routed_sends = dispatch_unconfirmed(
        &db,
        &config,
        &comm_state,
        &[0x10],
        Some(routed.clone()),
        request_bytes(vec![source, target]),
    )
    .await;
    assert_eq!(count(&db, sink).await, (1, 1));
    assert_eq!(
        persistence
            .snapshot
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .records
            .len(),
        1
    );

    let direct_sends = dispatch_unconfirmed(
        &db,
        &config,
        &comm_state,
        &[0x20],
        None,
        request_bytes(vec![notification(AuditOperation::READ)]),
    )
    .await;
    assert_eq!(count(&db, sink).await, (2, 2));
    assert_eq!(routed_sends.len(), 0, "the routed request must not send");
    assert_eq!(direct_sends.len(), 0, "the direct request must not send");
    assert!(persistence
        .snapshot
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .completed_receipts
        .is_empty());

    let contexts = contexts.lock().unwrap();
    assert_eq!(contexts[0].source_mac, MacAddr::from_slice(&[0x10]));
    assert_eq!(contexts[0].source_network, Some(routed));
    assert_eq!(contexts[0].audit_log_sink, sink);
    assert_eq!(contexts[1].source_mac, MacAddr::from_slice(&[0x20]));
    assert_eq!(contexts[1].source_network, None);
}

#[tokio::test]
async fn every_precommit_failure_is_silent_and_nonmutating() {
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let valid = request_bytes(vec![notification(AuditOperation::WRITE)]);

    for authorizer in [
        None,
        Some(
            Arc::new(|_: &UnconfirmedAuditNotificationAuthorizationContext| false)
                as UnconfirmedAuditNotificationAuthorizer,
        ),
        Some(Arc::new(
            |_: &UnconfirmedAuditNotificationAuthorizationContext| -> bool { panic!("denied") },
        ) as UnconfirmedAuditNotificationAuthorizer),
    ] {
        let db = database(Arc::new(MemoryPersistence::default()), 7);
        let config = ServerConfig {
            audit_notification_sink: Some(sink),
            unconfirmed_audit_notification_authorizer: authorizer,
            ..ServerConfig::default()
        };
        assert_silent_drop(&db, sink, &config, valid.clone()).await;
    }

    let authorized = ServerConfig {
        audit_notification_sink: Some(sink),
        unconfirmed_audit_notification_authorizer: Some(Arc::new(|_| true)),
        ..ServerConfig::default()
    };
    let db = database(Arc::new(MemoryPersistence::default()), 7);
    let mut trailing = BytesMut::from(valid.as_ref());
    trailing.extend_from_slice(&[0xff]);
    for malformed in [
        trailing.freeze(),
        Bytes::from(vec![0; MAX_AUDIT_NOTIFICATION_BYTES + 1]),
    ] {
        assert_silent_drop(&db, sink, &authorized, malformed).await;
    }
    let too_many = request_bytes(
        (0..=MAX_AUDIT_NOTIFICATIONS)
            .map(|_| notification(AuditOperation::WRITE))
            .collect(),
    );
    assert!(too_many.len() <= MAX_AUDIT_NOTIFICATION_BYTES);
    assert_silent_drop(&db, sink, &authorized, too_many).await;

    for configured_sink in [
        None,
        Some(oid(ObjectType::ANALOG_INPUT, 7)),
        Some(oid(ObjectType::AUDIT_LOG, 99)),
    ] {
        let config = ServerConfig {
            audit_notification_sink: configured_sink,
            unconfirmed_audit_notification_authorizer: Some(Arc::new(|_| true)),
            ..ServerConfig::default()
        };
        assert_silent_drop(&db, sink, &config, valid.clone()).await;
    }

    let disabled = database(Arc::new(MemoryPersistence::default()), 7);
    disabled
        .write()
        .await
        .get_mut(&sink)
        .unwrap()
        .write_property(
            PropertyIdentifier::LOG_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    assert_silent_drop(&disabled, sink, &authorized, valid.clone()).await;

    for device in [
        None,
        Some(DeviceConfig {
            apdu_segment_timeout: 5000,
            apdu_timeout: 0,
            ..DeviceConfig::default()
        }),
    ] {
        let db = database_with_device(Arc::new(MemoryPersistence::default()), 7, device);
        assert_silent_drop(&db, sink, &authorized, valid.clone()).await;
    }

    let persistence = Arc::new(MemoryPersistence::default());
    let db = database(Arc::clone(&persistence), 7);
    persistence.fail.store(true, Ordering::Release);
    assert_silent_drop(&db, sink, &authorized, valid.clone()).await;
    assert!(persistence
        .snapshot
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .records
        .is_empty());

    let persistence = Arc::new(MemoryPersistence::default());
    let db = database(Arc::clone(&persistence), 7);
    db.write().await.set_clock_reader(None);
    assert_silent_drop(&db, sink, &authorized, valid).await;
    assert!(persistence
        .snapshot
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .records
        .is_empty());
}
