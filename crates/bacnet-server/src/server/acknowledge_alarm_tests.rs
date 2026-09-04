use super::*;

use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::event::{EventStateChange, EventTransition, EventTransitionCommit};
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_types::enums::EventState;
use bacnet_types::primitives::BACnetTimeStamp;

fn request(oid: ObjectIdentifier, state: EventState) -> AcknowledgeAlarmRequest {
    AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 71,
        event_object_identifier: oid,
        event_state_acknowledged: state.to_raw(),
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(77),
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch(
    db: &Arc<RwLock<ObjectDatabase>>,
    tracker: &Arc<ConfirmedRequestTracker>,
    notification_transactions: &Arc<NotificationTransactions>,
    source_mac: &MacAddr,
    invoke_id: u8,
    request: &AcknowledgeAlarmRequest,
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
        service_choice: ConfirmedServiceChoice::ACKNOWLEDGE_ALARM,
        service_request: service_request.freeze(),
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
        tracker,
        &device_bindings,
        &comm_state,
        &dcc_timer,
        &ServerConfig::default(),
        source_mac,
        None,
        confirmed,
        Some(tx),
    )
    .await;

    rx.await.map(|bytes| {
        let npdu = decode_npdu(bytes).unwrap();
        decode_apdu(npdu.payload).unwrap()
    })
}

fn acked(db: &ObjectDatabase, oid: ObjectIdentifier) -> u8 {
    let PropertyValue::BitString { data, .. } = db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
        .unwrap()
    else {
        panic!("Acked_Transitions must be a bit string");
    };
    bacnet_types::bitstring::unpack_octet(&data, 3)
}

fn assert_simple_ack(apdu: Apdu, invoke_id: u8) {
    let Apdu::SimpleAck(ack) = apdu else {
        panic!("expected SimpleACK");
    };
    assert_eq!(ack.invoke_id, invoke_id);
    assert_eq!(
        ack.service_choice,
        ConfirmedServiceChoice::ACKNOWLEDGE_ALARM
    );
}

fn target_objects() -> Vec<Box<dyn BACnetObject>> {
    vec![
        Box::new(BinaryInputObject::new(1, "BI-ack").unwrap()),
        Box::new(BinaryOutputObject::new(1, "BO-ack").unwrap()),
        Box::new(BinaryValueObject::new(1, "BV-ack").unwrap()),
        Box::new(MultiStateInputObject::new(1, "MSI-ack", 3).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "MSO-ack", 3).unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV-ack", 3).unwrap()),
    ]
}

#[tokio::test]
async fn binary_and_multistate_families_return_simple_ack() {
    let mut objects = ObjectDatabase::new();
    let mut oids = Vec::new();
    for mut object in target_objects() {
        object
            .write_property(
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                None,
                PropertyValue::Boolean(true),
                None,
            )
            .unwrap();
        let oid = object.object_identifier();
        object
            .commit_event_transition_internal(EventTransitionCommit {
                change: EventStateChange {
                    from: EventState::NORMAL,
                    to: EventState::OFFNORMAL,
                },
                coordinate: EventTransition::ToOffnormal,
                ack_required: true,
                timestamp: BACnetTimeStamp::SequenceNumber(42),
                message_text: Some("offnormal".into()),
            })
            .unwrap();
        objects.add(object).unwrap();
        oids.push(oid);
    }

    let db = Arc::new(RwLock::new(objects));
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let notification_transactions = NotificationTransactions::new();
    let source = MacAddr::from_slice(&[1, 2, 3]);

    for (index, oid) in oids.into_iter().enumerate() {
        let invoke_id = 0x60 + index as u8;
        let response = dispatch(
            &db,
            &tracker,
            &notification_transactions,
            &source,
            invoke_id,
            &request(oid, EventState::OFFNORMAL),
        )
        .await
        .unwrap();
        assert_simple_ack(response, invoke_id);
        assert_eq!(acked(&*db.read().await, oid), 0b111);
    }
    assert_eq!(notification_transactions.active_count(), 0);
}

#[tokio::test]
async fn exact_duplicate_is_silent_and_new_invoke_id_is_idempotent_success() {
    let mut ai = AnalogInputObject::new(1, "AI-ack", 62).unwrap();
    let oid = ai.object_identifier();
    ai.commit_event_transition_internal(EventTransitionCommit {
        change: EventStateChange {
            from: EventState::NORMAL,
            to: EventState::HIGH_LIMIT,
        },
        coordinate: EventTransition::ToOffnormal,
        ack_required: true,
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        message_text: Some("high".into()),
    })
    .unwrap();
    let mut objects = ObjectDatabase::new();
    objects.add(Box::new(ai)).unwrap();
    let db = Arc::new(RwLock::new(objects));
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let notification_transactions = NotificationTransactions::new();
    let source = MacAddr::from_slice(&[1, 2, 3]);
    let request = request(oid, EventState::HIGH_LIMIT);

    let first = dispatch(
        &db,
        &tracker,
        &notification_transactions,
        &source,
        0x51,
        &request,
    )
    .await
    .unwrap();
    assert_simple_ack(first, 0x51);
    assert_eq!(acked(&*db.read().await, oid), 0b111);
    assert_eq!(notification_transactions.active_count(), 0);

    assert!(dispatch(
        &db,
        &tracker,
        &notification_transactions,
        &source,
        0x51,
        &request,
    )
    .await
    .is_err());
    assert_eq!(acked(&*db.read().await, oid), 0b111);

    let new_transaction = dispatch(
        &db,
        &tracker,
        &notification_transactions,
        &source,
        0x52,
        &request,
    )
    .await
    .unwrap();
    assert_simple_ack(new_transaction, 0x52);
    assert_eq!(acked(&*db.read().await, oid), 0b111);
    assert_eq!(notification_transactions.active_count(), 0);
}

#[path = "acknowledge_alarm_notification_tests.rs"]
mod notification_tests;
