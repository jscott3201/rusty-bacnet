//! Server-side LSO replay follow-ups: pending discard, routing identity,
//! invoke/byte discrimination, and oversize fallback.
//!
//! Companion to `life_safety_operation_tests.rs` (kept under the 700 LOC cap
//! per file): the byte-identical success/error replay pins live there, while
//! the remaining PR-0801 contract points live here. Replay remains a local
//! service-specific extension confined to LifeSafetyOperation, not a Standard
//! mandate, and makes no physical-idempotency claim.

use super::cov_notifications_tests::RecordingTransport;
use super::*;

use std::sync::atomic::AtomicUsize;
use std::sync::Mutex as StdMutex;

use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::life_safety::{
    LifeSafetyPointObject, LifeSafetyPointResetCommit, LifeSafetyZoneObject,
    LifeSafetyZoneResetCommit,
};
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_types::enums::{ErrorClass, ErrorCode, LifeSafetyOperation, LifeSafetyState};
use bytes::Bytes;

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
    let seg_ack_senders = Arc::new(segmented_send::SegmentedSendRegistry::default());
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let cov_in_flight = Arc::new(Semaphore::new(1));
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
    let notification_transactions = NotificationTransactions::new();
    let device_bindings = Arc::new(RwLock::new(DeviceBindingTable::new()));
    let comm_state = Arc::new(AtomicU8::new(0));
    let dcc_timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));
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
        &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
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

async fn dispatch_raw_with_tracker(
    db: Arc<RwLock<ObjectDatabase>>,
    config: ServerConfig,
    confirmed_request_tracker: &Arc<ConfirmedRequestTracker>,
    source_mac: MacAddr,
    source_network: Option<NpduAddress>,
    invoke_id: u8,
    request: LifeSafetyOperationRequest,
) -> Result<Bytes, tokio::sync::oneshot::error::RecvError> {
    let mut service_request = BytesMut::new();
    request.encode(&mut service_request).unwrap();
    dispatch_confirmed_raw_with_tracker(
        db,
        config,
        confirmed_request_tracker,
        source_mac,
        source_network,
        ConfirmedRequestPdu {
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
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_confirmed_raw_with_tracker(
    db: Arc<RwLock<ObjectDatabase>>,
    config: ServerConfig,
    confirmed_request_tracker: &Arc<ConfirmedRequestTracker>,
    source_mac: MacAddr,
    source_network: Option<NpduAddress>,
    confirmed: ConfirmedRequestPdu,
    // Note: takes the full PDU so oversize/changed-byte cases can craft raw
    // service requests without going through the typed encoder.
) -> Result<Bytes, tokio::sync::oneshot::error::RecvError> {
    let network = Arc::new(NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    )));
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
    let seg_ack_senders = Arc::new(segmented_send::SegmentedSendRegistry::default());
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let cov_in_flight = Arc::new(Semaphore::new(1));
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
    let notification_transactions = NotificationTransactions::new();
    let device_bindings = Arc::new(RwLock::new(DeviceBindingTable::new()));
    let comm_state = Arc::new(AtomicU8::new(0));
    let dcc_timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));
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
        &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
        &source_mac,
        source_network,
        confirmed,
        Some(tx),
    )
    .await;

    rx.await
}

fn decode_raw(bytes: &Bytes) -> Apdu {
    let npdu = decode_npdu(bytes.clone()).unwrap();
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
async fn pending_in_flight_duplicate_discards_without_replay() {
    // (a)/(f): concurrent exact duplicate while the first has no response yet
    // preserves DISCARD — there are no bytes to replay, and the first
    // execution must not be duplicated.
    let oid = point_oid(1);
    let executions = Arc::new(AtomicUsize::new(0));
    let observed_executions = Arc::clone(&executions);
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_present_value(LifeSafetyState::ALARM.to_raw());
    point.set_operation_expected(LifeSafetyOperation::RESET);
    point.set_reset_executor(Arc::new(move |_| {
        observed_executions.fetch_add(1, Ordering::AcqRel);
        Ok(LifeSafetyPointResetCommit {
            present_value: Some(LifeSafetyState::QUIET),
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
    let source = MacAddr::from_slice(&[7, 8, 9]);
    let reset = request(LifeSafetyOperation::RESET, Some(oid));

    // Simulate the in-flight window deterministically: insert the exact pending
    // admission the first execution would hold, then drive the duplicate
    // through the real confirmed path.
    let mut service_request = BytesMut::new();
    reset.encode(&mut service_request).unwrap();
    let in_flight = match tracker.lso.begin(
        &source,
        None,
        bacnet_encoding::apdu::ConfirmedRequest {
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
        },
    ) {
        crate::server::lso_replay::LsoAdmission::New(pending) => pending,
        _ => panic!("first admission should be new"),
    };
    assert!(in_flight.is_tracked());

    assert!(
        dispatch_life_safety_operation_with_tracker(
            Arc::clone(&db),
            config.clone(),
            &tracker,
            source.clone(),
            None,
            0x51,
            reset.clone(),
        )
        .await
        .is_err(),
        "in-flight duplicate must discard (no replay available)"
    );
    assert_eq!(authorizations.load(Ordering::Acquire), 0);
    assert_eq!(executions.load(Ordering::Acquire), 0);
    drop(in_flight);

    // After the in-flight entry clears, the same request executes normally.
    let after = dispatch_life_safety_operation_with_tracker(
        Arc::clone(&db),
        config,
        &tracker,
        source,
        None,
        0x51,
        reset,
    )
    .await
    .unwrap();
    assert_simple_ack(after);
    assert_eq!(authorizations.load(Ordering::Acquire), 1);
    assert_eq!(executions.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn routed_same_origin_replays_while_other_origins_execute_independently() {
    // (c): routing identity follows the canonical requester — same routed
    // origin via a different router replays; a different network, a direct
    // peer, and a different direct MAC are independent executions.
    let config_allow = || ServerConfig {
        life_safety_operation_authorizer: Some(Arc::new(|_| true)),
        ..ServerConfig::default()
    };
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let origin = NpduAddress {
        network: 222,
        mac_address: MacAddr::from_slice(&[12, 13]),
    };

    let first_db = {
        let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
        point.set_operation_expected(LifeSafetyOperation::SILENCE);
        let mut objects = ObjectDatabase::new();
        objects.add(Box::new(point)).unwrap();
        Arc::new(RwLock::new(objects))
    };
    let first_raw = dispatch_raw_with_tracker(
        Arc::clone(&first_db),
        config_allow(),
        &tracker,
        MacAddr::from_slice(&[10, 11]),
        Some(origin.clone()),
        0x51,
        request(LifeSafetyOperation::SILENCE, Some(point_oid(1))),
    )
    .await
    .unwrap();
    assert_simple_ack(decode_raw(&first_raw));

    // Same routed origin through a different router: byte-identical replay,
    // no second execution (state already shows the first applied).
    let replay_raw = dispatch_raw_with_tracker(
        Arc::clone(&first_db),
        config_allow(),
        &tracker,
        MacAddr::from_slice(&[99, 100]),
        Some(origin.clone()),
        0x51,
        request(LifeSafetyOperation::SILENCE, Some(point_oid(1))),
    )
    .await
    .unwrap();
    assert_eq!(first_raw, replay_raw);

    // Different network: independent execution (fresh database with an armed
    // point so the second execution can succeed rather than replay).
    let other_db = {
        let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
        point.set_operation_expected(LifeSafetyOperation::SILENCE);
        let mut objects = ObjectDatabase::new();
        objects.add(Box::new(point)).unwrap();
        Arc::new(RwLock::new(objects))
    };
    let other_origin = NpduAddress {
        network: 223,
        mac_address: MacAddr::from_slice(&[12, 13]),
    };
    let other = dispatch_life_safety_operation_with_tracker(
        other_db,
        config_allow(),
        &tracker,
        MacAddr::from_slice(&[10, 11]),
        Some(other_origin),
        0x51,
        request(LifeSafetyOperation::SILENCE, Some(point_oid(1))),
    )
    .await
    .unwrap();
    assert_simple_ack(other);

    // Direct peers are keyed by immediate MAC: a new direct MAC executes.
    let direct_db = {
        let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
        point.set_operation_expected(LifeSafetyOperation::SILENCE);
        let mut objects = ObjectDatabase::new();
        objects.add(Box::new(point)).unwrap();
        Arc::new(RwLock::new(objects))
    };
    let direct = dispatch_life_safety_operation_with_tracker(
        direct_db,
        config_allow(),
        &tracker,
        MacAddr::from_slice(&[1]),
        None,
        0x51,
        request(LifeSafetyOperation::SILENCE, Some(point_oid(1))),
    )
    .await
    .unwrap();
    assert_simple_ack(direct);
}

#[tokio::test]
async fn invoke_reuse_with_changed_bytes_and_new_invoke_execute() {
    // (d): full key match required — a reused invoke ID with different bytes
    // and a new invoke ID with identical bytes both execute fresh; post-TTL
    // expiry is covered by the cache unit tests with a deterministic clock.
    // After the first SILENCE applies (expectation clears to NONE), any fresh
    // execution of the same operation fails with INVALID_OPERATION, while a
    // replay would have returned the original SimpleACK — the Error proves a
    // new execution happened.
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
            true
        })),
        ..ServerConfig::default()
    };
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let source = MacAddr::from_slice(&[3, 3, 3]);

    let first = dispatch_life_safety_operation_with_tracker(
        Arc::clone(&db),
        config.clone(),
        &tracker,
        source.clone(),
        None,
        0x51,
        request(LifeSafetyOperation::SILENCE, Some(oid)),
    )
    .await
    .unwrap();
    assert_simple_ack(first);
    assert_eq!(authorizations.load(Ordering::Acquire), 1);

    // Same invoke, changed Requesting Source bytes → fresh execution → Error.
    let mut changed_source = request(LifeSafetyOperation::SILENCE, Some(oid));
    changed_source.requesting_source = "different operator".into();
    let changed = dispatch_life_safety_operation_with_tracker(
        Arc::clone(&db),
        config.clone(),
        &tracker,
        source.clone(),
        None,
        0x51,
        changed_source,
    )
    .await
    .unwrap();
    assert_error(
        changed,
        ErrorClass::OBJECT,
        ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
    );
    assert_eq!(authorizations.load(Ordering::Acquire), 2);

    // New invoke ID, otherwise identical bytes → fresh execution → Error.
    let reused = dispatch_life_safety_operation_with_tracker(
        Arc::clone(&db),
        config,
        &tracker,
        source,
        None,
        0x52,
        request(LifeSafetyOperation::SILENCE, Some(oid)),
    )
    .await
    .unwrap();
    match reused {
        Apdu::Error(error) => {
            assert_eq!(error.invoke_id, 0x52);
            assert_eq!(
                error.service_choice,
                ConfirmedServiceChoice::LIFE_SAFETY_OPERATION
            );
            assert_eq!(error.error_class, ErrorClass::OBJECT);
            assert_eq!(error.error_code, ErrorCode::INVALID_OPERATION_IN_THIS_STATE);
        }
        other => panic!("expected Error PDU, got {other:?}"),
    }
    assert_eq!(authorizations.load(Ordering::Acquire), 3);
}

#[tokio::test]
async fn oversize_lso_request_executes_untracked_without_suppression() {
    // (e): >64 KiB service requests execute untracked — each retransmission
    // executes (here: deterministically rejected at decode) and never
    // suppresses the next one.
    let db = Arc::new(RwLock::new(ObjectDatabase::new()));
    let config = ServerConfig {
        life_safety_operation_authorizer: Some(Arc::new(|_| true)),
        ..ServerConfig::default()
    };
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let source = MacAddr::from_slice(&[5, 5, 5]);
    let oversized = ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 0x51,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
        service_request: Bytes::from(vec![0u8; 64 * 1024 + 1]),
    };

    for _ in 0..2 {
        let raw = dispatch_confirmed_raw_with_tracker(
            Arc::clone(&db),
            config.clone(),
            &tracker,
            source.clone(),
            None,
            oversized.clone(),
        )
        .await
        .expect("oversize LSO must always execute (never discard)");
        match decode_raw(&raw) {
            Apdu::Error(_) | Apdu::Reject(_) => {}
            other => panic!("expected deterministic reject for oversize garbage, got {other:?}"),
        }
    }
}

/// Recording dispatch for the targetless-reset replay pin.
///
/// Unlike the oneshot helpers above, the confirmed response AND any COV
/// notifications are captured as sent frames, so a duplicate replay must show
/// exactly one frame (the byte-identical SimpleACK) with no second COV. Only
/// the database, recording network, COV table, tracker, config, source, and
/// frame sink are shared; the remaining dispatch plumbing is rebuilt per call.
#[allow(clippy::too_many_arguments)]
async fn dispatch_lso_raw_recording(
    db: &Arc<RwLock<ObjectDatabase>>,
    network: &Arc<NetworkLayer<RecordingTransport>>,
    cov_table: &Arc<RwLock<CovSubscriptionTable>>,
    tracker: &Arc<ConfirmedRequestTracker>,
    config: &ServerConfig,
    source_mac: &MacAddr,
    sent: &Arc<StdMutex<Vec<(Bytes, MacAddr)>>>,
    invoke_id: u8,
    service_request: Bytes,
) -> Vec<Bytes> {
    let seg_ack_senders = Arc::new(segmented_send::SegmentedSendRegistry::default());
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let cov_in_flight = Arc::new(Semaphore::new(255));
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
    let notification_transactions = NotificationTransactions::new();
    let device_bindings = Arc::new(RwLock::new(DeviceBindingTable::new()));
    let comm_state = Arc::new(AtomicU8::new(0));
    let dcc_timer = Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default()));

    BACnetServer::<RecordingTransport>::handle_confirmed_request(
        db,
        network,
        cov_table,
        &seg_ack_senders,
        &seg_send_permits,
        &cov_in_flight,
        &server_tsm,
        &notification_transactions,
        tracker,
        &device_bindings,
        &comm_state,
        &dcc_timer,
        config,
        &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
        source_mac,
        None,
        ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 480,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
            service_request,
        },
        None,
    )
    .await;

    let frames: Vec<Bytes> = sent
        .lock()
        .unwrap()
        .iter()
        .map(|(frame, _)| frame.clone())
        .collect();
    sent.lock().unwrap().clear();
    frames
}

fn decode_frame(frame: &Bytes) -> Apdu {
    let npdu = decode_npdu(frame.clone()).unwrap();
    decode_apdu(npdu.payload).unwrap()
}

#[tokio::test]
async fn targetless_reset_duplicate_replays_identical_simple_ack_without_second_actuation() {
    // PR-0802 remainder: a targetless RESET applies to both an armed Point and
    // an armed Zone (single SimpleACK per §13.13), and an identical confirmed
    // retransmission within the window replays the byte-identical SimpleACK
    // with zero second actuations, zero second authorizations, and zero second
    // COV. This is a local service-specific extension, not a Standard mandate,
    // and makes no physical-idempotency claim.
    let point_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let zone_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 1).unwrap();
    let point_executions = Arc::new(AtomicUsize::new(0));
    let zone_executions = Arc::new(AtomicUsize::new(0));
    let observed_point = Arc::clone(&point_executions);
    let observed_zone = Arc::clone(&zone_executions);

    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_present_value(LifeSafetyState::ALARM.to_raw());
    point.set_operation_expected(LifeSafetyOperation::RESET);
    point.set_reset_executor(Arc::new(move |_| {
        observed_point.fetch_add(1, Ordering::AcqRel);
        Ok(LifeSafetyPointResetCommit {
            present_value: Some(LifeSafetyState::QUIET),
            ..Default::default()
        })
    }));
    let mut zone = LifeSafetyZoneObject::new(1, "zone").unwrap();
    zone.set_present_value(LifeSafetyState::FAULT.to_raw());
    zone.set_operation_expected(LifeSafetyOperation::RESET);
    zone.set_reset_executor(Arc::new(move |_| {
        observed_zone.fetch_add(1, Ordering::AcqRel);
        Ok(LifeSafetyZoneResetCommit {
            present_value: Some(LifeSafetyState::ACTIVE),
            ..Default::default()
        })
    }));
    let mut db = clocked_test_database();
    db.add(Box::new(point)).unwrap();
    db.add(Box::new(zone)).unwrap();

    let authorizations = Arc::new(AtomicUsize::new(0));
    let observed_auth = Arc::clone(&authorizations);
    let config = ServerConfig {
        life_safety_operation_authorizer: Some(Arc::new(move |_| {
            observed_auth.fetch_add(1, Ordering::AcqRel);
            true
        })),
        ..ServerConfig::default()
    };
    let db = Arc::new(RwLock::new(db));
    let sent = Arc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(Arc::clone(
        &sent,
    ))));
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
    {
        let mut table = cov_table.write().await;
        for (oid, process_id) in [(point_oid, 1), (zone_oid, 2)] {
            table.subscribe(CovSubscription {
                subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, process_id as u8]),
                subscriber_network: None,
                subscriber_process_identifier: process_id,
                monitored_object_identifier: oid,
                issue_confirmed_notifications: false,
                expires_at: None,
                last_notified_value: None,
                monitored_property: None,
                monitored_property_array_index: None,
                cov_increment: None,
                notification_kind: CovNotificationKind::Single,
                timestamped: false,
            });
        }
    }
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let source = MacAddr::from_slice(&[9, 9, 9]);

    let mut encoded = BytesMut::new();
    request(LifeSafetyOperation::RESET, None)
        .encode(&mut encoded)
        .unwrap();
    let encoded = encoded.freeze();

    let first = dispatch_lso_raw_recording(
        &db,
        &network,
        &cov_table,
        &tracker,
        &config,
        &source,
        &sent,
        0x51,
        encoded.clone(),
    )
    .await;
    assert_eq!(
        first.len(),
        3,
        "targetless reset of two objects sends one ACK plus one COV per object"
    );
    assert_simple_ack(decode_frame(&first[0]));
    let mut notified: Vec<ObjectIdentifier> = first[1..]
        .iter()
        .map(|frame| match decode_frame(frame) {
            Apdu::UnconfirmedRequest(notification) => {
                assert_eq!(
                    notification.service_choice,
                    UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION
                );
                COVNotificationRequest::decode(&notification.service_request)
                    .unwrap()
                    .monitored_object_identifier
            }
            other => panic!("expected unconfirmed COV notification, got {other:?}"),
        })
        .collect();
    notified.sort_by_key(|oid| (oid.object_type().to_raw(), oid.instance_number()));
    assert_eq!(notified, vec![point_oid, zone_oid]);
    assert_eq!(point_executions.load(Ordering::Acquire), 1);
    assert_eq!(zone_executions.load(Ordering::Acquire), 1);
    assert_eq!(authorizations.load(Ordering::Acquire), 1);

    // Identical confirmed bytes within the window: byte-identical SimpleACK,
    // no second executor actuation, no second authorization, no second COV.
    let second = dispatch_lso_raw_recording(
        &db, &network, &cov_table, &tracker, &config, &source, &sent, 0x51, encoded,
    )
    .await;
    assert_eq!(
        second.len(),
        1,
        "executed targetless LSO duplicate replays exactly one ACK with no second COV"
    );
    assert_eq!(
        second[0], first[0],
        "replayed targetless LSO ACK must be byte-identical"
    );
    assert_simple_ack(decode_frame(&second[0]));
    assert_eq!(point_executions.load(Ordering::Acquire), 1);
    assert_eq!(zone_executions.load(Ordering::Acquire), 1);
    assert_eq!(authorizations.load(Ordering::Acquire), 1);
    let guard = db.read().await;
    for (oid, expected) in [
        (point_oid, LifeSafetyState::QUIET),
        (zone_oid, LifeSafetyState::ACTIVE),
    ] {
        assert_eq!(
            guard
                .get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(expected.to_raw())
        );
        assert_eq!(
            guard
                .get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::OPERATION_EXPECTED, None)
                .unwrap(),
            PropertyValue::Enumerated(LifeSafetyOperation::NONE.to_raw())
        );
    }
}
