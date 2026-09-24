use super::*;

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
            table
                .subscribe(CovSubscription {
                    subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, process_id as u8]),
                    subscriber_network: None,
                    subscriber_process_identifier: process_id,
                    monitored_object_identifier: oid,
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
