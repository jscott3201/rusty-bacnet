use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use bacnet_objects::device::DeviceConfig;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::AuditOperation;
use tokio::sync::mpsc;

use super::audit_notification_tests::{
    count, database, database_with_device, notification, oid, request_bytes, MemoryPersistence,
};
use super::*;

#[derive(Clone)]
struct CountingTransport {
    sends: Arc<AtomicUsize>,
}

impl TransportPort for CountingTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &[0]
    }
}

fn received(
    source_mac: &[u8],
    source_network: Option<NpduAddress>,
) -> bacnet_network::layer::ReceivedApdu {
    bacnet_network::layer::ReceivedApdu {
        apdu: Bytes::new(),
        source_mac: MacAddr::from_slice(source_mac),
        source_network,
        link_layer_group: false,
        is_group: false,
        data_attributes: Vec::new(),
        reply_tx: None,
    }
}

async fn dispatch_unconfirmed(
    db: &Arc<RwLock<ObjectDatabase>>,
    config: &ServerConfig,
    comm_state: &Arc<AtomicU8>,
    source_mac: &[u8],
    source_network: Option<NpduAddress>,
    service_request: Bytes,
    sends: Arc<AtomicUsize>,
) {
    let network = Arc::new(NetworkLayer::new(CountingTransport { sends }));
    let bindings = Arc::new(RwLock::new(DeviceBindingTable::new()));
    BACnetServer::<CountingTransport>::handle_unconfirmed_request(
        db,
        &network,
        config,
        None,
        comm_state,
        &bindings,
        UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION,
            service_request,
        },
        &received(source_mac, source_network),
    )
    .await;
}

async fn assert_silent_drop(
    db: &Arc<RwLock<ObjectDatabase>>,
    sink: ObjectIdentifier,
    config: &ServerConfig,
    service_request: Bytes,
    comm_state: u8,
) {
    let before = count(db, sink).await;
    let sends = Arc::new(AtomicUsize::new(0));
    dispatch_unconfirmed(
        db,
        config,
        &Arc::new(AtomicU8::new(comm_state)),
        &[1],
        None,
        service_request,
        Arc::clone(&sends),
    )
    .await;
    assert_eq!(count(db, sink).await, before);
    assert_eq!(sends.load(Ordering::SeqCst), 0);
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
    let sends = Arc::new(AtomicUsize::new(0));
    let comm_state = Arc::new(AtomicU8::new(0));

    let source = notification(AuditOperation::WRITE);
    let mut target = source.clone();
    target.source_timestamp = None;
    target.target_timestamp = source.source_timestamp.clone();
    dispatch_unconfirmed(
        &db,
        &config,
        &comm_state,
        &[0x10],
        Some(routed.clone()),
        request_bytes(vec![source, target]),
        Arc::clone(&sends),
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

    dispatch_unconfirmed(
        &db,
        &config,
        &comm_state,
        &[0x20],
        None,
        request_bytes(vec![notification(AuditOperation::READ)]),
        Arc::clone(&sends),
    )
    .await;
    assert_eq!(count(&db, sink).await, (2, 2));
    assert_eq!(sends.load(Ordering::SeqCst), 0);

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
        assert_silent_drop(&db, sink, &config, valid.clone(), 0).await;
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
        assert_silent_drop(&db, sink, &authorized, malformed, 0).await;
    }
    let too_many = request_bytes(
        (0..=MAX_AUDIT_NOTIFICATIONS)
            .map(|_| notification(AuditOperation::WRITE))
            .collect(),
    );
    assert!(too_many.len() <= MAX_AUDIT_NOTIFICATION_BYTES);
    assert_silent_drop(&db, sink, &authorized, too_many, 0).await;

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
        assert_silent_drop(&db, sink, &config, valid.clone(), 0).await;
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
    assert_silent_drop(&disabled, sink, &authorized, valid.clone(), 0).await;

    for device in [
        None,
        Some(DeviceConfig {
            apdu_timeout: 0,
            ..DeviceConfig::default()
        }),
    ] {
        let db = database_with_device(Arc::new(MemoryPersistence::default()), 7, device);
        assert_silent_drop(&db, sink, &authorized, valid.clone(), 0).await;
    }

    let persistence = Arc::new(MemoryPersistence::default());
    let db = database(Arc::clone(&persistence), 7);
    persistence.fail.store(true, Ordering::Release);
    assert_silent_drop(&db, sink, &authorized, valid.clone(), 0).await;
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
    assert_silent_drop(&db, sink, &authorized, valid.clone(), 0).await;
    assert!(persistence
        .snapshot
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .records
        .is_empty());

    let called = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&called);
    let config = ServerConfig {
        audit_notification_sink: Some(sink),
        unconfirmed_audit_notification_authorizer: Some(Arc::new(move |_| {
            observed.store(true, Ordering::Release);
            true
        })),
        ..ServerConfig::default()
    };
    assert_silent_drop(&db, sink, &config, valid, 1).await;
    assert!(!called.load(Ordering::Acquire));
}
