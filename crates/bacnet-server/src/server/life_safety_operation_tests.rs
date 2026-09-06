use super::*;

use std::sync::atomic::AtomicUsize;
use std::sync::Mutex as StdMutex;

use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyPointResetCommit};
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_types::enums::{LifeSafetyOperation, LifeSafetyState, SilencedState};

fn point_oid(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, instance).unwrap()
}

fn request(
    operation: LifeSafetyOperation,
    oid: Option<ObjectIdentifier>,
) -> LifeSafetyOperationRequest {
    LifeSafetyOperationRequest {
        requesting_process_identifier: 41,
        requesting_source: "operator label".into(),
        request: operation,
        object_identifier: oid,
    }
}

async fn dispatch_life_safety_operation(
    db: Arc<RwLock<ObjectDatabase>>,
    config: ServerConfig,
    source_mac: MacAddr,
    source_network: Option<NpduAddress>,
    request: LifeSafetyOperationRequest,
) -> Apdu {
    dispatch_life_safety_operation_with_tracker(
        db,
        config,
        &Arc::new(ConfirmedRequestTracker::default()),
        source_mac,
        source_network,
        0x51,
        request,
    )
    .await
    .expect("first request should receive a response")
}

async fn dispatch_life_safety_operation_with_tracker(
    db: Arc<RwLock<ObjectDatabase>>,
    config: ServerConfig,
    confirmed_request_tracker: &Arc<ConfirmedRequestTracker>,
    source_mac: MacAddr,
    source_network: Option<NpduAddress>,
    invoke_id: u8,
    request: LifeSafetyOperationRequest,
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
    let notification_transactions = NotificationTransactions::new();
    let device_bindings = Arc::new(RwLock::new(DeviceBindingTable::new()));
    let comm_state = Arc::new(AtomicU8::new(0));
    let dcc_timer = Arc::new(Mutex::new(None::<JoinHandle<()>>));
    let mut service_request = BytesMut::new();
    request.encode(&mut service_request).unwrap();
    let confirmed = ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
        service_request: service_request.freeze(),
    };
    let (tx, rx) = oneshot::channel();

    BACnetServer::<BipTransport>::handle_confirmed_request(
        &db,
        &network,
        &cov_table,
        &seg_ack_senders,
        &seg_send_permits,
        &cov_in_flight,
        &server_tsm,
        &notification_transactions,
        confirmed_request_tracker,
        &device_bindings,
        &comm_state,
        &dcc_timer,
        &config,
        &source_mac,
        source_network,
        confirmed,
        Some(tx),
    )
    .await;

    rx.await.map(|bytes| {
        let npdu = decode_npdu(bytes).unwrap();
        decode_apdu(npdu.payload).unwrap()
    })
}

fn assert_error(apdu: Apdu, class: ErrorClass, code: ErrorCode) {
    match apdu {
        Apdu::Error(error) => {
            assert_eq!(error.invoke_id, 0x51);
            assert_eq!(
                error.service_choice,
                ConfirmedServiceChoice::LIFE_SAFETY_OPERATION
            );
            assert_eq!(error.error_class, class);
            assert_eq!(error.error_code, code);
        }
        other => panic!("expected Error PDU, got {other:?}"),
    }
}

fn assert_simple_ack(apdu: Apdu) {
    match apdu {
        Apdu::SimpleAck(ack) => {
            assert_eq!(ack.invoke_id, 0x51);
            assert_eq!(
                ack.service_choice,
                ConfirmedServiceChoice::LIFE_SAFETY_OPERATION
            );
        }
        other => panic!("expected SimpleACK, got {other:?}"),
    }
}

#[tokio::test]
async fn local_rearm_api_supports_two_operation_cycles() {
    let oid = point_oid(1);
    let executions = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&executions);
    let point = LifeSafetyPointObject::new(1, "point")
        .unwrap()
        .with_reset_executor(Arc::new(move |_| {
            observed.fetch_add(1, Ordering::AcqRel);
            Ok(LifeSafetyPointResetCommit::default())
        }));
    let mut objects = ObjectDatabase::new();
    objects.add(Box::new(point)).unwrap();
    let mut server = BACnetServer::<BipTransport>::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(objects)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();

    for operation in [LifeSafetyOperation::RESET, LifeSafetyOperation::RESET_FAULT] {
        server
            .set_life_safety_operation_expected_local(&oid, operation)
            .await
            .unwrap();
        let mut db = server.db.write().await;
        let changed =
            handlers::handle_life_safety_operation(&mut db, &request(operation, Some(oid)))
                .unwrap();
        assert_eq!(changed, vec![oid]);
    }

    assert_eq!(executions.load(Ordering::Acquire), 2);
    let db = server.db.read().await;
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::OPERATION_EXPECTED, None)
            .unwrap(),
        PropertyValue::Enumerated(LifeSafetyOperation::NONE.to_raw())
    );
    drop(db);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn life_safety_operation_default_policy_denies_without_mutation() {
    let oid = point_oid(1);
    let executions = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&executions);
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_present_value(LifeSafetyState::ALARM.to_raw());
    point.set_operation_expected(LifeSafetyOperation::RESET);
    point.set_reset_executor(Arc::new(move |_| {
        observed.fetch_add(1, Ordering::AcqRel);
        Ok(LifeSafetyPointResetCommit {
            present_value: Some(LifeSafetyState::QUIET),
            ..Default::default()
        })
    }));
    let mut objects = ObjectDatabase::new();
    objects.add(Box::new(point)).unwrap();
    let db = Arc::new(RwLock::new(objects));

    let apdu = dispatch_life_safety_operation(
        Arc::clone(&db),
        ServerConfig::default(),
        MacAddr::from_slice(&[1, 2, 3]),
        None,
        request(LifeSafetyOperation::RESET, Some(oid)),
    )
    .await;

    assert_error(
        apdu,
        ErrorClass::SERVICES,
        ErrorCode::SERVICE_REQUEST_DENIED,
    );
    assert_eq!(executions.load(Ordering::Acquire), 0);
    let guard = db.read().await;
    assert_eq!(
        guard
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(LifeSafetyState::ALARM.to_raw())
    );
    assert_eq!(
        guard
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::OPERATION_EXPECTED, None)
            .unwrap(),
        PropertyValue::Enumerated(LifeSafetyOperation::RESET.to_raw())
    );
}

#[tokio::test]
async fn unknown_target_is_rejected_before_authorization() {
    let authorizations = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&authorizations);
    let config = ServerConfig {
        life_safety_operation_authorizer: Some(Arc::new(move |_| {
            observed.fetch_add(1, Ordering::AcqRel);
            true
        })),
        ..ServerConfig::default()
    };

    let apdu = dispatch_life_safety_operation(
        Arc::new(RwLock::new(ObjectDatabase::new())),
        config,
        MacAddr::from_slice(&[1]),
        None,
        request(LifeSafetyOperation::RESET, Some(point_oid(99))),
    )
    .await;

    assert_error(apdu, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);
    assert_eq!(authorizations.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn life_safety_operation_authorizer_receives_routed_identity_before_success() {
    let oid = point_oid(1);
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_operation_expected(LifeSafetyOperation::SILENCE);
    let mut objects = ObjectDatabase::new();
    objects.add(Box::new(point)).unwrap();
    let db = Arc::new(RwLock::new(objects));
    let seen = Arc::new(StdMutex::new(None));
    let seen_by_policy = Arc::clone(&seen);
    let db_by_policy = Arc::clone(&db);
    let source_mac = MacAddr::from_slice(&[10, 11]);
    let routed_source = NpduAddress {
        network: 222,
        mac_address: MacAddr::from_slice(&[12, 13]),
    };
    let config = ServerConfig {
        life_safety_operation_authorizer: Some(Arc::new(move |context| {
            assert!(
                db_by_policy.try_write().is_ok(),
                "authorization must run outside the database lock"
            );
            *seen_by_policy.lock().unwrap() = Some(context.clone());
            true
        })),
        ..ServerConfig::default()
    };

    let apdu = dispatch_life_safety_operation(
        Arc::clone(&db),
        config,
        source_mac.clone(),
        Some(routed_source.clone()),
        request(LifeSafetyOperation::SILENCE, Some(oid)),
    )
    .await;

    assert_simple_ack(apdu);
    let context = seen.lock().unwrap().clone().expect("policy was invoked");
    assert_eq!(context.source_mac, source_mac);
    assert_eq!(context.source_network, Some(routed_source));
    assert_eq!(context.invoke_id, 0x51);
    assert_eq!(context.request.object_identifier, Some(oid));
    let guard = db.read().await;
    assert_eq!(
        guard
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::SILENCED, None)
            .unwrap(),
        PropertyValue::Enumerated(SilencedState::ALL_SILENCED.to_raw())
    );
}

#[tokio::test]
async fn life_safety_operation_dispatch_preserves_all_targeted_error_rows() {
    let allow = || ServerConfig {
        life_safety_operation_authorizer: Some(Arc::new(|_| true)),
        ..ServerConfig::default()
    };
    let source = MacAddr::from_slice(&[1]);

    let missing_db = Arc::new(RwLock::new(ObjectDatabase::new()));
    let missing = point_oid(99);
    let apdu = dispatch_life_safety_operation(
        missing_db,
        allow(),
        source.clone(),
        None,
        request(LifeSafetyOperation::SILENCE, Some(missing)),
    )
    .await;
    assert_error(apdu, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);

    let apdu = dispatch_life_safety_operation(
        Arc::new(RwLock::new(ObjectDatabase::new())),
        allow(),
        source.clone(),
        None,
        request(LifeSafetyOperation::RESET, Some(missing)),
    )
    .await;
    assert_error(apdu, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);

    let analog_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let mut objects = ObjectDatabase::new();
    objects
        .add(Box::new(AnalogInputObject::new(1, "analog", 62).unwrap()))
        .unwrap();
    let apdu = dispatch_life_safety_operation(
        Arc::new(RwLock::new(objects)),
        allow(),
        source.clone(),
        None,
        request(LifeSafetyOperation::RESET, Some(analog_oid)),
    )
    .await;
    assert_error(
        apdu,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );

    let oid = point_oid(1);
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_operation_expected(LifeSafetyOperation::RESET);
    let mut objects = ObjectDatabase::new();
    objects.add(Box::new(point)).unwrap();
    let db = Arc::new(RwLock::new(objects));
    let apdu = dispatch_life_safety_operation(
        Arc::clone(&db),
        allow(),
        source.clone(),
        None,
        request(LifeSafetyOperation::RESET, Some(oid)),
    )
    .await;
    assert_error(
        apdu,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );

    let apdu = dispatch_life_safety_operation(
        db,
        allow(),
        source,
        None,
        request(LifeSafetyOperation::SILENCE, Some(oid)),
    )
    .await;
    assert_error(
        apdu,
        ErrorClass::OBJECT,
        ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
    );
}

#[tokio::test]
async fn life_safety_operation_targetless_missing_executor_returns_simple_ack_without_mutation() {
    let oid = point_oid(1);
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_operation_expected(LifeSafetyOperation::RESET);
    let mut objects = ObjectDatabase::new();
    objects.add(Box::new(point)).unwrap();
    let db = Arc::new(RwLock::new(objects));
    let config = ServerConfig {
        life_safety_operation_authorizer: Some(Arc::new(|_| true)),
        ..ServerConfig::default()
    };

    let apdu = dispatch_life_safety_operation(
        Arc::clone(&db),
        config,
        MacAddr::from_slice(&[1]),
        None,
        request(LifeSafetyOperation::RESET, None),
    )
    .await;

    assert_simple_ack(apdu);
    let guard = db.read().await;
    assert_eq!(
        guard
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::OPERATION_EXPECTED, None)
            .unwrap(),
        PropertyValue::Enumerated(LifeSafetyOperation::RESET.to_raw())
    );
}

#[tokio::test]
async fn life_safety_operation_panicking_authorizer_fails_closed() {
    let oid = point_oid(1);
    let mut objects = ObjectDatabase::new();
    objects
        .add(Box::new(LifeSafetyPointObject::new(1, "point").unwrap()))
        .unwrap();
    let config = ServerConfig {
        life_safety_operation_authorizer: Some(Arc::new(|_| panic!("policy failure"))),
        ..ServerConfig::default()
    };

    let apdu = dispatch_life_safety_operation(
        Arc::new(RwLock::new(objects)),
        config,
        MacAddr::from_slice(&[1]),
        None,
        request(LifeSafetyOperation::SILENCE, Some(oid)),
    )
    .await;

    assert_error(
        apdu,
        ErrorClass::SERVICES,
        ErrorCode::SERVICE_REQUEST_DENIED,
    );
}

#[tokio::test]
async fn exact_success_duplicate_is_silent_and_changed_reuse_executes_normally() {
    let oid = point_oid(1);
    let executions = Arc::new(AtomicUsize::new(0));
    let observed_executions = Arc::clone(&executions);
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_present_value(LifeSafetyState::ALARM.to_raw());
    point.set_operation_expected(LifeSafetyOperation::RESET);
    point.set_reset_executor(Arc::new(move |context| {
        observed_executions.fetch_add(1, Ordering::AcqRel);
        let present_value = if context.operation == LifeSafetyOperation::RESET {
            LifeSafetyState::QUIET
        } else {
            LifeSafetyState::ACTIVE
        };
        Ok(LifeSafetyPointResetCommit {
            present_value: Some(present_value),
            ..Default::default()
        })
    }));
    let mut objects = ObjectDatabase::new();
    objects.add(Box::new(point)).unwrap();
    let db = Arc::new(RwLock::new(objects));
    let authorizations = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&authorizations);
    let config = ServerConfig {
        life_safety_operation_authorizer: Some(Arc::new(move |_| {
            observed.fetch_add(1, Ordering::AcqRel);
            true
        })),
        ..ServerConfig::default()
    };
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let source = MacAddr::from_slice(&[1, 2, 3]);
    let reset = request(LifeSafetyOperation::RESET, Some(oid));

    let first = dispatch_life_safety_operation_with_tracker(
        Arc::clone(&db),
        config.clone(),
        &tracker,
        source.clone(),
        None,
        0x51,
        reset.clone(),
    )
    .await
    .unwrap();
    assert_simple_ack(first);
    assert!(dispatch_life_safety_operation_with_tracker(
        Arc::clone(&db),
        config.clone(),
        &tracker,
        source.clone(),
        None,
        0x51,
        reset,
    )
    .await
    .is_err());
    assert_eq!(authorizations.load(Ordering::Acquire), 1);
    assert_eq!(executions.load(Ordering::Acquire), 1);
    assert_eq!(
        db.read()
            .await
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(LifeSafetyState::QUIET.to_raw())
    );

    db.write()
        .await
        .get_mut(&oid)
        .unwrap()
        .set_life_safety_operation_expected_internal(LifeSafetyOperation::RESET_ALARM)
        .unwrap();
    let changed = dispatch_life_safety_operation_with_tracker(
        Arc::clone(&db),
        config,
        &tracker,
        source,
        None,
        0x51,
        request(LifeSafetyOperation::RESET_ALARM, Some(oid)),
    )
    .await
    .unwrap();
    assert_simple_ack(changed);
    assert_eq!(authorizations.load(Ordering::Acquire), 2);
    assert_eq!(executions.load(Ordering::Acquire), 2);
    assert_eq!(
        db.read()
            .await
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(LifeSafetyState::ACTIVE.to_raw())
    );
}

#[tokio::test]
async fn exact_denied_duplicate_is_silent_without_second_authorization() {
    let oid = point_oid(1);
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_operation_expected(LifeSafetyOperation::SILENCE);
    let mut objects = ObjectDatabase::new();
    objects.add(Box::new(point)).unwrap();
    let db = Arc::new(RwLock::new(objects));
    let authorizations = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&authorizations);
    let config = ServerConfig {
        life_safety_operation_authorizer: Some(Arc::new(move |_| {
            observed.fetch_add(1, Ordering::AcqRel);
            false
        })),
        ..ServerConfig::default()
    };
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let source = MacAddr::from_slice(&[4, 5, 6]);
    let denied = request(LifeSafetyOperation::SILENCE, Some(oid));

    let first = dispatch_life_safety_operation_with_tracker(
        Arc::clone(&db),
        config.clone(),
        &tracker,
        source.clone(),
        None,
        0x51,
        denied.clone(),
    )
    .await
    .unwrap();
    assert_error(
        first,
        ErrorClass::SERVICES,
        ErrorCode::SERVICE_REQUEST_DENIED,
    );
    assert!(dispatch_life_safety_operation_with_tracker(
        Arc::clone(&db),
        config,
        &tracker,
        source,
        None,
        0x51,
        denied,
    )
    .await
    .is_err());
    assert_eq!(authorizations.load(Ordering::Acquire), 1);
    assert_eq!(
        db.read()
            .await
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::SILENCED, None)
            .unwrap(),
        PropertyValue::Enumerated(SilencedState::UNSILENCED.to_raw())
    );
}
