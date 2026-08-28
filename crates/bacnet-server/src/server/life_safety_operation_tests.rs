use super::*;

use std::sync::Mutex as StdMutex;

use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::life_safety::LifeSafetyPointObject;
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_types::enums::{LifeSafetyOperation, SilencedState};

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
        invoke_id: 0x51,
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

    let npdu = decode_npdu(rx.await.expect("reply_tx should receive response")).unwrap();
    decode_apdu(npdu.payload).unwrap()
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
    let mut objects = ObjectDatabase::new();
    objects
        .add(Box::new(LifeSafetyPointObject::new(1, "point").unwrap()))
        .unwrap();
    let mut server = BACnetServer::<BipTransport>::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(objects)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();

    for operation in [LifeSafetyOperation::SILENCE, LifeSafetyOperation::UNSILENCE] {
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

    let db = server.db.read().await;
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::SILENCED, None)
            .unwrap(),
        PropertyValue::Enumerated(SilencedState::UNSILENCED.to_raw())
    );
    drop(db);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn life_safety_operation_default_policy_denies_without_mutation() {
    let oid = point_oid(1);
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_operation_expected(LifeSafetyOperation::SILENCE);
    let mut objects = ObjectDatabase::new();
    objects.add(Box::new(point)).unwrap();
    let db = Arc::new(RwLock::new(objects));

    let apdu = dispatch_life_safety_operation(
        Arc::clone(&db),
        ServerConfig::default(),
        MacAddr::from_slice(&[1, 2, 3]),
        None,
        request(LifeSafetyOperation::SILENCE, Some(oid)),
    )
    .await;

    assert_error(
        apdu,
        ErrorClass::SERVICES,
        ErrorCode::SERVICE_REQUEST_DENIED,
    );
    let guard = db.read().await;
    assert_eq!(
        guard
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::SILENCED, None)
            .unwrap(),
        PropertyValue::Enumerated(SilencedState::UNSILENCED.to_raw())
    );
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
        request(LifeSafetyOperation::SILENCE, Some(analog_oid)),
    )
    .await;
    assert_error(
        apdu,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );

    let oid = point_oid(1);
    let mut objects = ObjectDatabase::new();
    objects
        .add(Box::new(LifeSafetyPointObject::new(1, "point").unwrap()))
        .unwrap();
    let db = Arc::new(RwLock::new(objects));
    let apdu = dispatch_life_safety_operation(
        Arc::clone(&db),
        allow(),
        source.clone(),
        None,
        request(LifeSafetyOperation::RESET, Some(oid)),
    )
    .await;
    assert_error(apdu, ErrorClass::OBJECT, ErrorCode::VALUE_OUT_OF_RANGE);

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
async fn life_safety_operation_targetless_reset_attempt_returns_simple_ack_without_mutation() {
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
