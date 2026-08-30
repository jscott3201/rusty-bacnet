use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot};
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_types::constructed::{BACnetAuditNotification, BACnetRecipient};
use bacnet_types::enums::{AuditOperation, ServiceSupported};
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, PropertyValue, Time};

use super::*;

#[derive(Default)]
pub(super) struct MemoryPersistence {
    pub(super) snapshot: StdMutex<Option<AuditLogSnapshot>>,
    pub(super) fail: AtomicBool,
}

impl AuditLogPersistence for MemoryPersistence {
    fn load(&self, _expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.snapshot.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        if self.fail.load(Ordering::Acquire) {
            return Err(Error::Transport(std::io::Error::other(
                "injected persistence failure",
            )));
        }
        *self.snapshot.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 124,
                month: 2,
                day: 29,
                day_of_week: 4,
            },
            local_time: Time {
                hour: 12,
                minute: 0,
                second: 30,
                hundredths: 0,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

pub(super) fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

pub(super) fn notification(operation: AuditOperation) -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: Some(BACnetTimeStamp::Time(Time {
            hour: 12,
            minute: 0,
            second: 0,
            hundredths: 0,
        })),
        target_timestamp: None,
        // Deliberately conflicts with transport provenance. The server must
        // preserve it as peer-reported payload, never rewrite or authenticate it.
        source_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 999)),
        source_object: None,
        operation,
        source_comment: None,
        target_comment: None,
        invoke_id: Some(200),
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 2)),
        target_object: None,
        target_property: None,
        target_priority: None,
        target_value: None,
        current_value: None,
        result: None,
    }
}

pub(super) fn request_bytes(notifications: Vec<BACnetAuditNotification>) -> Bytes {
    let mut bytes = BytesMut::new();
    AuditNotificationRequest { notifications }
        .try_encode(&mut bytes)
        .unwrap();
    bytes.freeze()
}

pub(super) fn database(
    persistence: Arc<MemoryPersistence>,
    sink_instance: u32,
) -> Arc<RwLock<ObjectDatabase>> {
    database_with_device(persistence, sink_instance, Some(DeviceConfig::default()))
}

pub(super) fn database_with_device(
    persistence: Arc<MemoryPersistence>,
    sink_instance: u32,
    device_config: Option<DeviceConfig>,
) -> Arc<RwLock<ObjectDatabase>> {
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(Arc::new(FixedClock)));
    if let Some(device_config) = device_config {
        db.add(Box::new(DeviceObject::new(device_config).unwrap()))
            .unwrap();
    }
    db.add(Box::new(
        AuditLogObject::new(sink_instance, "audit", 16, persistence).unwrap(),
    ))
    .unwrap();
    Arc::new(RwLock::new(db))
}

async fn dispatch(
    db: &Arc<RwLock<ObjectDatabase>>,
    config: &ServerConfig,
    notification_transactions: &Arc<NotificationTransactions>,
    invoke_id: u8,
    source_mac: &[u8],
    source_network: Option<NpduAddress>,
    service_request: Bytes,
) -> Result<Apdu, tokio::sync::oneshot::error::RecvError> {
    let network = Arc::new(NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    )));
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let cov_in_flight = Arc::new(Semaphore::new(1));
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
    let device_bindings = Arc::new(RwLock::new(DeviceBindingTable::new()));
    let comm_state = Arc::new(AtomicU8::new(0));
    let dcc_timer = Arc::new(Mutex::new(None::<JoinHandle<()>>));
    let confirmed = ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        service_request,
    };
    let (tx, rx) = oneshot::channel();
    BACnetServer::<BipTransport>::handle_confirmed_request(
        db,
        &network,
        &cov_table,
        &seg_ack_senders,
        &seg_send_permits,
        &cov_in_flight,
        &server_tsm,
        notification_transactions,
        &device_bindings,
        &comm_state,
        &dcc_timer,
        config,
        source_mac,
        source_network,
        confirmed,
        Some(tx),
    )
    .await;
    rx.await
        .map(|bytes| decode_apdu(decode_npdu(bytes).unwrap().payload).unwrap())
}

pub(super) async fn count(db: &Arc<RwLock<ObjectDatabase>>, sink: ObjectIdentifier) -> (u64, u64) {
    let db = db.read().await;
    let object = db.get(&sink).unwrap();
    let PropertyValue::Unsigned(records) = object
        .read_property(PropertyIdentifier::RECORD_COUNT, None)
        .unwrap()
    else {
        unreachable!()
    };
    let PropertyValue::Unsigned(total) = object
        .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
        .unwrap()
    else {
        unreachable!()
    };
    (records, total)
}

#[tokio::test]
async fn authorized_routed_request_preserves_provenance_and_duplicate_is_silent() {
    let persistence = Arc::new(MemoryPersistence::default());
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let db = database(persistence.clone(), 7);
    let contexts = Arc::new(StdMutex::new(Vec::new()));
    let observed = contexts.clone();
    let mut config = ServerConfig {
        audit_notification_sink: Some(sink),
        ..ServerConfig::default()
    };
    config.audit_notification_authorizer = Some(Arc::new(move |context| {
        observed.lock().unwrap().push(context.clone());
        true
    }));
    let notification_transactions = NotificationTransactions::new();
    let routed = NpduAddress {
        network: 55,
        mac_address: MacAddr::from_slice(&[0xaa]),
    };
    let source_report = notification(AuditOperation::WRITE);
    let mut target_report = source_report.clone();
    target_report.source_timestamp = None;
    target_report.target_timestamp = Some(BACnetTimeStamp::Time(Time {
        hour: 12,
        minute: 0,
        second: 4,
        hundredths: 0,
    }));
    let bytes = request_bytes(vec![source_report, target_report]);

    let response = dispatch(
        &db,
        &config,
        &notification_transactions,
        9,
        &[0x10],
        Some(routed.clone()),
        bytes.clone(),
    )
    .await
    .unwrap();
    assert!(matches!(response, Apdu::SimpleAck(_)));
    assert_eq!(count(&db, sink).await, (1, 1));
    let context = contexts.lock().unwrap().first().unwrap().clone();
    assert_eq!(context.source_mac, MacAddr::from_slice(&[0x10]));
    assert_eq!(context.source_network, Some(routed));
    assert_eq!(context.invoke_id, 9);
    assert_eq!(context.audit_log_sink, sink);
    assert_eq!(
        context.request.notifications[0].source_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 999))
    );

    assert!(dispatch(
        &db,
        &config,
        &notification_transactions,
        9,
        &[0x10],
        context.source_network,
        bytes
    )
    .await
    .is_err());
    assert_eq!(count(&db, sink).await, (1, 1));
    assert_eq!(contexts.lock().unwrap().len(), 1);

    let response = dispatch(
        &db,
        &config,
        &notification_transactions,
        9,
        &[0x10],
        None,
        request_bytes(vec![notification(AuditOperation::READ)]),
    )
    .await
    .unwrap();
    assert!(matches!(response, Apdu::SimpleAck(_)));
    assert_eq!(count(&db, sink).await, (2, 2));
    let contexts = contexts.lock().unwrap();
    assert_eq!(
        contexts.last().unwrap().source_mac,
        MacAddr::from_slice(&[0x10])
    );
    assert_eq!(contexts.last().unwrap().source_network, None);
}

#[tokio::test]
async fn policy_decode_bounds_sink_and_persistence_fail_before_success_ack() {
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    for authorizer in [
        None,
        Some(Arc::new(|_: &AuditNotificationAuthorizationContext| false)
            as AuditNotificationAuthorizer),
        Some(
            Arc::new(|_: &AuditNotificationAuthorizationContext| -> bool { panic!("denied panic") })
                as AuditNotificationAuthorizer,
        ),
    ] {
        let persistence = Arc::new(MemoryPersistence::default());
        let db = database(persistence, 7);
        let config = ServerConfig {
            audit_notification_sink: Some(sink),
            audit_notification_authorizer: authorizer,
            ..ServerConfig::default()
        };
        let response = dispatch(
            &db,
            &config,
            &NotificationTransactions::new(),
            1,
            &[1],
            None,
            request_bytes(vec![notification(AuditOperation::WRITE)]),
        )
        .await
        .unwrap();
        let Apdu::Error(error) = response else {
            panic!("expected error")
        };
        assert_eq!(error.error_class, ErrorClass::SERVICES);
        assert_eq!(error.error_code, ErrorCode::SERVICE_REQUEST_DENIED);
        assert_eq!(count(&db, sink).await, (0, 0));
    }

    let persistence = Arc::new(MemoryPersistence::default());
    let db = database(persistence.clone(), 7);
    persistence.fail.store(true, Ordering::Release);
    let config = ServerConfig {
        audit_notification_sink: Some(sink),
        audit_notification_authorizer: Some(Arc::new(|_| true)),
        ..ServerConfig::default()
    };
    let response = dispatch(
        &db,
        &config,
        &NotificationTransactions::new(),
        2,
        &[1],
        None,
        request_bytes(vec![notification(AuditOperation::WRITE)]),
    )
    .await
    .unwrap();
    assert!(matches!(response, Apdu::Error(_)));
    assert_eq!(count(&db, sink).await, (0, 0));

    for malformed in [
        Bytes::from_static(b"bad"),
        Bytes::from(vec![0; MAX_AUDIT_NOTIFICATION_BYTES + 1]),
    ] {
        let response = dispatch(
            &db,
            &config,
            &NotificationTransactions::new(),
            3,
            &[1],
            None,
            malformed,
        )
        .await
        .unwrap();
        assert!(matches!(response, Apdu::Error(_)));
        assert_eq!(count(&db, sink).await, (0, 0));
    }

    let too_many = request_bytes(
        (0..=MAX_AUDIT_NOTIFICATIONS)
            .map(|_| notification(AuditOperation::WRITE))
            .collect(),
    );
    let response = dispatch(
        &db,
        &config,
        &NotificationTransactions::new(),
        4,
        &[1],
        None,
        too_many,
    )
    .await
    .unwrap();
    assert!(matches!(response, Apdu::Error(_)));
    assert_eq!(count(&db, sink).await, (0, 0));

    for configured_sink in [
        None,
        Some(oid(ObjectType::ANALOG_INPUT, 7)),
        Some(oid(ObjectType::AUDIT_LOG, 99)),
    ] {
        let config = ServerConfig {
            audit_notification_sink: configured_sink,
            audit_notification_authorizer: Some(Arc::new(|_| true)),
            ..ServerConfig::default()
        };
        let response = dispatch(
            &db,
            &config,
            &NotificationTransactions::new(),
            5,
            &[1],
            None,
            request_bytes(vec![notification(AuditOperation::READ)]),
        )
        .await
        .unwrap();
        let Apdu::Error(error) = response else {
            panic!("expected error")
        };
        assert_eq!(error.error_class, ErrorClass::SERVICES);
        assert_eq!(error.error_code, ErrorCode::SERVICE_REQUEST_DENIED);
        assert_eq!(count(&db, sink).await, (0, 0));
    }
}

#[tokio::test]
async fn disabled_sink_returns_service_request_denied_without_receiver_mutation() {
    let persistence = Arc::new(MemoryPersistence::default());
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let db = database(persistence, 7);
    {
        let mut db = db.write().await;
        db.get_mut(&sink)
            .unwrap()
            .write_property(
                PropertyIdentifier::LOG_ENABLE,
                None,
                PropertyValue::Boolean(false),
                None,
            )
            .unwrap();
    }
    let before = count(&db, sink).await;
    let config = ServerConfig {
        audit_notification_sink: Some(sink),
        audit_notification_authorizer: Some(Arc::new(|_| true)),
        ..ServerConfig::default()
    };
    let response = dispatch(
        &db,
        &config,
        &NotificationTransactions::new(),
        6,
        &[1],
        None,
        request_bytes(vec![notification(AuditOperation::WRITE)]),
    )
    .await
    .unwrap();
    let Apdu::Error(error) = response else {
        panic!("expected error")
    };
    assert_eq!(error.error_class, ErrorClass::SERVICES);
    assert_eq!(error.error_code, ErrorCode::SERVICE_REQUEST_DENIED);
    assert_eq!(count(&db, sink).await, before);
}

#[tokio::test]
async fn missing_or_invalid_device_apdu_timeout_is_operational_problem_without_mutation() {
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    for device_config in [
        None,
        Some(DeviceConfig {
            apdu_timeout: 0,
            ..DeviceConfig::default()
        }),
    ] {
        let persistence = Arc::new(MemoryPersistence::default());
        let db = database_with_device(persistence, 7, device_config);
        let config = ServerConfig {
            audit_notification_sink: Some(sink),
            audit_notification_authorizer: Some(Arc::new(|_| true)),
            ..ServerConfig::default()
        };
        let response = dispatch(
            &db,
            &config,
            &NotificationTransactions::new(),
            7,
            &[1],
            None,
            request_bytes(vec![notification(AuditOperation::WRITE)]),
        )
        .await
        .unwrap();
        let Apdu::Error(error) = response else {
            panic!("expected error")
        };
        assert_eq!(error.error_class, ErrorClass::DEVICE);
        assert_eq!(error.error_code, ErrorCode::OPERATIONAL_PROBLEM);
        assert_eq!(count(&db, sink).await, (0, 0));
    }
}

#[test]
fn executed_service_truth_includes_confirmed_and_unconfirmed_receipt() {
    assert!(EXECUTED_CONFIRMED.contains(&ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION));
    assert!(
        EXECUTED_UNCONFIRMED.contains(&UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION)
    );
    assert!(bacnet_objects::device::EXECUTED_SERVICES
        .contains(&ServiceSupported::CONFIRMED_AUDIT_NOTIFICATION));
    assert!(bacnet_objects::device::EXECUTED_SERVICES
        .contains(&ServiceSupported::UNCONFIRMED_AUDIT_NOTIFICATION));
}

#[test]
fn generic_and_bip_builders_store_the_same_explicit_receiver_policy() {
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let generic = BACnetServer::<BipTransport>::generic_builder()
        .audit_notification_sink(sink)
        .audit_notification_authorizer(|_| true)
        .unconfirmed_audit_notification_authorizer(|_| true);
    assert_eq!(generic.config.audit_notification_sink, Some(sink));
    assert!(generic.config.audit_notification_authorizer.is_some());
    assert!(generic
        .config
        .unconfirmed_audit_notification_authorizer
        .is_some());

    let bip = BACnetServer::<BipTransport>::bip_builder()
        .audit_notification_sink(sink)
        .audit_notification_authorizer(|_| true)
        .unconfirmed_audit_notification_authorizer(|_| true);
    assert_eq!(bip.config.audit_notification_sink, Some(sink));
    assert!(bip.config.audit_notification_authorizer.is_some());
    assert!(bip
        .config
        .unconfirmed_audit_notification_authorizer
        .is_some());
}

#[cfg(feature = "sc-tls")]
#[test]
fn sc_builder_exposes_the_same_explicit_receiver_policy() {
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let sc = BACnetServer::<
        bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>,
    >::sc_builder()
    .audit_notification_sink(sink)
    .audit_notification_authorizer(|_| true)
    .unconfirmed_audit_notification_authorizer(|_| true);
    assert_eq!(sc.config.audit_notification_sink, Some(sink));
    assert!(sc.config.audit_notification_authorizer.is_some());
    assert!(sc
        .config
        .unconfirmed_audit_notification_authorizer
        .is_some());
}
