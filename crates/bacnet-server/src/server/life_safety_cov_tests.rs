use super::cov_notifications_tests::recording_transport;
use super::*;
use crate::server::test_transport::{SendLog, TestTransport};

use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyPointResetCommit};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bytes::Bytes;

fn point_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap()
}

fn subscription(
    property: Option<PropertyIdentifier>,
    kind: CovNotificationKind,
    process_id: u32,
) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, process_id as u8]),
        subscriber_network: None,
        subscriber_process_identifier: process_id,
        monitored_object_identifier: point_oid(),
        issue_confirmed_notifications: false,
        // A Multiple context always has a finite lifetime.
        expires_at: (kind == CovNotificationKind::Multiple)
            .then(|| Instant::now() + Duration::from_secs(3600)),
        last_notified_observation: None,
        monitored_property: property,
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: kind,
        timestamped: false,
    }
}

fn life_safety_db() -> ObjectDatabase {
    let mut db = clocked_test_database();
    db.add(Box::new(LifeSafetyPointObject::new(1, "point").unwrap()))
        .unwrap();
    db
}

fn decode_sent(sent: &SendLog) -> Vec<Apdu> {
    sent.lock()
        .iter()
        .map(|frame| decode_apdu(decode_npdu(frame.npdu.clone()).unwrap().payload).unwrap())
        .collect()
}

fn single_properties(apdu: &Apdu) -> Vec<PropertyIdentifier> {
    let Apdu::UnconfirmedRequest(request) = apdu else {
        panic!("expected unconfirmed COV notification, got {apdu:?}");
    };
    assert_eq!(
        request.service_choice,
        UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION
    );
    COVNotificationRequest::decode(&request.service_request)
        .unwrap()
        .list_of_values
        .into_iter()
        .map(|value| value.property_identifier)
        .collect()
}

struct ExactFixture {
    sent: SendLog,
    db: Arc<RwLock<ObjectDatabase>>,
    network: Arc<NetworkLayer<TestTransport>>,
    cov_table: Arc<RwLock<CovSubscriptionTable>>,
    cov_in_flight: Arc<Semaphore>,
    transactions: Arc<NotificationTransactions>,
    comm_state: Arc<CommState>,
}

impl ExactFixture {
    async fn new(subscriptions: impl IntoIterator<Item = CovSubscription>) -> Self {
        let (transport, sent) = recording_transport();
        let network = Arc::new(NetworkLayer::new(transport));
        let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
        {
            let mut table = cov_table.write().await;
            for subscription in subscriptions {
                table.admit_for_test(subscription, 0).unwrap();
            }
        }
        Self {
            sent,
            db: Arc::new(RwLock::new(life_safety_db())),
            network,
            cov_table,
            cov_in_flight: Arc::new(Semaphore::new(255)),
            transactions: NotificationTransactions::new(),
            comm_state: Arc::new(CommState::default()),
        }
    }

    async fn fire(&self, changes: &[PropertyIdentifier]) {
        BACnetServer::<TestTransport>::fire_life_safety_cov_notifications(
            &crate::server::cov_notify_context::CovNotifyContext {
                db: &self.db,
                network: &self.network,
                cov_table: &self.cov_table,
                cov_in_flight: &self.cov_in_flight,
                notification_transactions: &self.transactions,
                comm_state: &self.comm_state,
                config: &Arc::default(),
            },
            &point_oid(),
            changes,
        )
        .await;
    }

    fn take_apdus(&self) -> Vec<Apdu> {
        let decoded = decode_sent(&self.sent);
        self.sent.clear();
        decoded
    }
}

#[tokio::test]
async fn exact_single_cov_filters_whole_and_property_payloads() {
    let fixture = ExactFixture::new([
        subscription(None, CovNotificationKind::Single, 1),
        subscription(
            Some(PropertyIdentifier::SILENCED),
            CovNotificationKind::Single,
            2,
        ),
        subscription(
            Some(PropertyIdentifier::OPERATION_EXPECTED),
            CovNotificationKind::Single,
            3,
        ),
        subscription(
            Some(PropertyIdentifier::STATUS_FLAGS),
            CovNotificationKind::Single,
            4,
        ),
    ])
    .await;

    fixture.fire(&[PropertyIdentifier::SILENCED]).await;
    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 1);
    assert_eq!(
        single_properties(&apdus[0]),
        vec![
            PropertyIdentifier::SILENCED,
            PropertyIdentifier::STATUS_FLAGS,
        ]
    );

    fixture.fire(&[PropertyIdentifier::PRESENT_VALUE]).await;
    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 1);
    assert_eq!(
        single_properties(&apdus[0]),
        vec![
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
        ]
    );

    fixture.fire(&[PropertyIdentifier::STATUS_FLAGS]).await;
    let mut payloads: Vec<_> = fixture.take_apdus().iter().map(single_properties).collect();
    payloads.sort_by_key(|properties| properties[0].to_raw());
    assert_eq!(payloads.len(), 4);
    assert!(payloads.contains(&vec![PropertyIdentifier::STATUS_FLAGS]));
    assert!(payloads.contains(&vec![
        PropertyIdentifier::OPERATION_EXPECTED,
        PropertyIdentifier::STATUS_FLAGS,
    ]));
    assert!(payloads.contains(&vec![
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::STATUS_FLAGS,
    ]));
    assert!(payloads.contains(&vec![
        PropertyIdentifier::SILENCED,
        PropertyIdentifier::STATUS_FLAGS,
    ]));

    // A selected-value repeat is now suppressed; change the expected-operation state.
    fixture
        .db
        .write()
        .await
        .get_mut(&point_oid())
        .unwrap()
        .set_life_safety_operation_expected_internal(
            bacnet_types::enums::LifeSafetyOperation::SILENCE,
        )
        .unwrap();
    fixture
        .fire(&[PropertyIdentifier::OPERATION_EXPECTED])
        .await;
    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 1);
    assert_eq!(
        single_properties(&apdus[0]),
        vec![
            PropertyIdentifier::OPERATION_EXPECTED,
            PropertyIdentifier::STATUS_FLAGS,
        ]
    );
}

#[tokio::test]
async fn exact_point_tracking_cov_does_not_add_unrelated_subscriptions() {
    let fixture = ExactFixture::new([
        subscription(
            Some(PropertyIdentifier::TRACKING_VALUE),
            CovNotificationKind::Single,
            1,
        ),
        subscription(
            Some(PropertyIdentifier::SILENCED),
            CovNotificationKind::Single,
            2,
        ),
    ])
    .await;

    fixture.fire(&[PropertyIdentifier::TRACKING_VALUE]).await;

    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 1);
    assert_eq!(
        single_properties(&apdus[0]),
        vec![
            PropertyIdentifier::TRACKING_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
        ]
    );
}

#[tokio::test]
async fn exact_multiple_cov_groups_matching_properties_and_one_status_flags() {
    let mut silenced = subscription(
        Some(PropertyIdentifier::SILENCED),
        CovNotificationKind::Multiple,
        7,
    );
    silenced.timestamped = true;
    let mut operation_expected = subscription(
        Some(PropertyIdentifier::OPERATION_EXPECTED),
        CovNotificationKind::Multiple,
        7,
    );
    operation_expected.subscriber_mac = silenced.subscriber_mac.clone();
    let mut present_value = subscription(
        Some(PropertyIdentifier::PRESENT_VALUE),
        CovNotificationKind::Multiple,
        7,
    );
    present_value.subscriber_mac = silenced.subscriber_mac.clone();
    let fixture = ExactFixture::new([silenced, operation_expected, present_value]).await;

    fixture.fire(&[PropertyIdentifier::SILENCED]).await;

    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 1);
    let Apdu::UnconfirmedRequest(request) = &apdus[0] else {
        panic!("expected unconfirmed multiple notification");
    };
    assert_eq!(
        request.service_choice,
        UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
    );
    let notification = COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
    assert!(notification.timestamp.is_some());
    let values = &notification.list_of_cov_notifications[0].list_of_values;
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].property_identifier, PropertyIdentifier::SILENCED);
    assert_eq!(
        values[1].property_identifier,
        PropertyIdentifier::STATUS_FLAGS
    );
    assert!(values.iter().all(|value| value.time_of_change.is_some()));
}

#[tokio::test]
async fn trusted_rearm_and_local_oos_write_notify_only_actual_deltas() {
    let (transport, sent) = recording_transport();
    let mut server = BACnetServer::<TestTransport>::generic_builder()
        .transport(transport)
        .database(life_safety_db())
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();
    server
        .cov_table
        .write()
        .await
        .subscribe(subscription(
            Some(PropertyIdentifier::OPERATION_EXPECTED),
            CovNotificationKind::Single,
            1,
        ))
        .unwrap();

    server
        .set_life_safety_operation_expected_local(&point_oid(), LifeSafetyOperation::SILENCE)
        .await
        .unwrap();
    assert_eq!(decode_sent(&sent).len(), 1);
    server
        .set_life_safety_operation_expected_local(&point_oid(), LifeSafetyOperation::SILENCE)
        .await
        .unwrap();
    assert_eq!(decode_sent(&sent).len(), 1, "same-value rearm is silent");

    server
        .write_local(
            &point_oid(),
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_eq!(decode_sent(&sent).len(), 2, "Status_Flags fans out");
    server
        .write_local(
            &point_oid(),
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_eq!(decode_sent(&sent).len(), 2, "same-value write is silent");

    server.stop().await.unwrap();
}

struct DispatchFixture {
    sent: SendLog,
    db: Arc<RwLock<ObjectDatabase>>,
    network: Arc<NetworkLayer<TestTransport>>,
    cov_table: Arc<RwLock<CovSubscriptionTable>>,
    seg_ack_senders: Arc<segmented_send::SegmentedSendRegistry>,
    seg_send_permits: Arc<Semaphore>,
    cov_in_flight: Arc<Semaphore>,
    learned_routers: Arc<Mutex<LearnedRouterCache>>,
    transactions: Arc<NotificationTransactions>,
    tracker: Arc<ConfirmedRequestTracker>,
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
    comm_state: Arc<CommState>,
    dcc_timer: Arc<Mutex<crate::server::dcc_timer::TimerSlot>>,
    config: Arc<ServerConfig>,
    source_mac: MacAddr,
}

impl DispatchFixture {
    async fn new(
        db: ObjectDatabase,
        subscriptions: impl IntoIterator<Item = CovSubscription>,
    ) -> Self {
        let (transport, sent) = recording_transport();
        let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
        {
            let mut table = cov_table.write().await;
            for subscription in subscriptions {
                table.admit_for_test(subscription, 0).unwrap();
            }
        }
        Self {
            network: Arc::new(NetworkLayer::new(transport)),
            sent,
            db: Arc::new(RwLock::new(db)),
            cov_table,
            seg_ack_senders: Arc::new(segmented_send::SegmentedSendRegistry::default()),
            seg_send_permits: Arc::new(Semaphore::new(MAX_SEG_SENDERS)),
            cov_in_flight: Arc::new(Semaphore::new(255)),
            learned_routers: Arc::new(Mutex::new(LearnedRouterCache::new())),
            transactions: NotificationTransactions::new(),
            tracker: Arc::new(ConfirmedRequestTracker::default()),
            device_bindings: Arc::new(RwLock::new(DeviceBindingTable::new())),
            comm_state: Arc::new(CommState::default()),
            dcc_timer: Arc::new(Mutex::new(crate::server::dcc_timer::TimerSlot::default())),
            config: Arc::new(ServerConfig {
                life_safety_operation_authorizer: Some(Arc::new(|_| true)),
                ..ServerConfig::default()
            }),
            source_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xD0]),
        }
    }

    async fn dispatch(
        &self,
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        service_request: Bytes,
    ) {
        self.dispatch_at(
            &self.source_mac,
            None,
            invoke_id,
            service_choice,
            service_request,
        )
        .await;
    }

    async fn dispatch_at(
        &self,
        source: &MacAddr,
        routed: Option<&NpduAddress>,
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        service_request: Bytes,
    ) {
        BACnetServer::<TestTransport>::handle_confirmed_request(
            &RequestServices {
                db: Arc::clone(&self.db),
                cov_table: Arc::clone(&self.cov_table),
                seg_ack_senders: Arc::clone(&self.seg_ack_senders),
                seg_send_permits: Arc::clone(&self.seg_send_permits),
                cov_in_flight: Arc::clone(&self.cov_in_flight),
                learned_routers: Arc::clone(&self.learned_routers),
                notification_transactions: Arc::clone(&self.transactions),
                device_bindings: Arc::clone(&self.device_bindings),
                comm_state: Arc::clone(&self.comm_state),
                dcc_timer: Arc::clone(&self.dcc_timer),
                ..RequestServices::for_test(Arc::clone(&self.network), self.config.clone())
            },
            &self.tracker,
            &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
            source,
            routed.cloned(),
            ConfirmedRequestPdu {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: 480,
                invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice,
                service_request,
            },
            None,
        )
        .await;
    }

    fn take_apdus(&self) -> Vec<Apdu> {
        let apdus = decode_sent(&self.sent);
        self.sent.clear();
        apdus
    }
}

fn encode_write_property(property: PropertyIdentifier, value: PropertyValue) -> Bytes {
    let mut value_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut value_buf, &value).unwrap();
    let request = WritePropertyRequest {
        object_identifier: point_oid(),
        property_identifier: property,
        property_array_index: None,
        property_value: value_buf.to_vec(),
        priority: None,
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded).unwrap();
    encoded.freeze()
}

#[tokio::test]
async fn network_write_property_and_multiple_use_exact_status_deltas() {
    let fixture = DispatchFixture::new(
        life_safety_db(),
        [
            subscription(
                Some(PropertyIdentifier::OPERATION_EXPECTED),
                CovNotificationKind::Single,
                1,
            ),
            subscription(None, CovNotificationKind::Single, 2),
        ],
    )
    .await;

    fixture
        .dispatch(
            1,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            encode_write_property(
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            ),
        )
        .await;
    let apdus = fixture.take_apdus();
    assert!(matches!(apdus[0], Apdu::SimpleAck(_)));
    let payloads: Vec<_> = apdus[1..].iter().map(single_properties).collect();
    assert_eq!(payloads.len(), 2);
    assert!(payloads.contains(&vec![
        PropertyIdentifier::OPERATION_EXPECTED,
        PropertyIdentifier::STATUS_FLAGS,
    ]));
    assert!(payloads.contains(&vec![
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::STATUS_FLAGS,
    ]));

    fixture
        .dispatch(
            2,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            encode_write_property(
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            ),
        )
        .await;
    assert_eq!(
        fixture.take_apdus().len(),
        1,
        "same-value WP has only its ACK"
    );

    let mut value_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(
        &mut value_buf,
        &PropertyValue::Boolean(false),
    )
    .unwrap();
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: point_oid(),
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::OUT_OF_SERVICE,
                property_array_index: None,
                value: value_buf.to_vec(),
                priority: None,
            }],
        }],
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded).unwrap();
    fixture
        .dispatch(
            3,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            encoded.freeze(),
        )
        .await;
    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 3);
    assert!(matches!(apdus[0], Apdu::SimpleAck(_)));
    let payloads: Vec<_> = apdus[1..].iter().map(single_properties).collect();
    assert!(payloads.contains(&vec![
        PropertyIdentifier::OPERATION_EXPECTED,
        PropertyIdentifier::STATUS_FLAGS,
    ]));
    assert!(payloads.contains(&vec![
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::STATUS_FLAGS,
    ]));
}

#[tokio::test]
async fn operation_ack_precedes_exact_cov_and_duplicate_replays_ack_without_second_cov() {
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_present_value(bacnet_types::enums::LifeSafetyState::ALARM);
    point.set_operation_expected(LifeSafetyOperation::RESET);
    point.set_reset_executor(Arc::new(|_| {
        Ok(LifeSafetyPointResetCommit {
            present_value: Some(bacnet_types::enums::LifeSafetyState::QUIET),
            ..Default::default()
        })
    }));
    let mut db = clocked_test_database();
    db.add(Box::new(point)).unwrap();
    let fixture =
        DispatchFixture::new(db, [subscription(None, CovNotificationKind::Single, 1)]).await;
    let request = LifeSafetyOperationRequest {
        requesting_process_identifier: 9,
        requesting_source: "operator".into(),
        request: LifeSafetyOperation::RESET,
        object_identifier: Some(point_oid()),
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded).unwrap();
    let encoded = encoded.freeze();

    fixture
        .dispatch(
            0x51,
            ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
            encoded.clone(),
        )
        .await;
    let first_raw: Vec<Bytes> = fixture
        .sent
        .lock()
        .iter()
        .map(|frame| frame.npdu.clone())
        .collect();
    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 2);
    assert!(matches!(apdus[0], Apdu::SimpleAck(_)));
    assert_eq!(
        single_properties(&apdus[1]),
        vec![
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
        ]
    );

    // PR-0801: retransmitted already-executed LSO replays the byte-identical
    // SimpleACK with zero side effects — no second COV. This is a local
    // service-specific extension, not a Standard mandate.
    fixture
        .dispatch(0x51, ConfirmedServiceChoice::LIFE_SAFETY_OPERATION, encoded)
        .await;
    let second_raw: Vec<Bytes> = fixture
        .sent
        .lock()
        .iter()
        .map(|frame| frame.npdu.clone())
        .collect();
    assert_eq!(
        second_raw.len(),
        1,
        "executed LSO duplicate replays exactly one ACK with no second COV"
    );
    assert_eq!(
        second_raw[0], first_raw[0],
        "replayed LSO ACK must be byte-identical"
    );
    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 1);
    assert!(matches!(apdus[0], Apdu::SimpleAck(_)));

    {
        let mut db = fixture.db.write().await;
        db.get_mut(&point_oid())
            .unwrap()
            .set_life_safety_operation_expected_internal(LifeSafetyOperation::SILENCE)
            .unwrap();
    }
    let silence = LifeSafetyOperationRequest {
        requesting_process_identifier: 9,
        requesting_source: "operator".into(),
        request: LifeSafetyOperation::SILENCE,
        object_identifier: Some(point_oid()),
    };
    let mut encoded = BytesMut::new();
    silence.encode(&mut encoded).unwrap();
    fixture
        .dispatch(
            0x52,
            ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
            encoded.freeze(),
        )
        .await;
    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 1, "Silenced/OE-only operation has only ACK");
    assert!(matches!(apdus[0], Apdu::SimpleAck(_)));
}

mod custom_operation;
mod event_state_pins;
mod failures;
mod initial;
mod routed;
mod schedule;
mod simulation;
mod wpm_prefix;

#[path = "life_safety_cov_tests/identity.rs"]
mod identity;

#[path = "life_safety_cov_tests/identity_arrays.rs"]
mod identity_arrays;

#[cfg(test)]
mod lifetime;

#[cfg(test)]
mod lifetime_mixed;
