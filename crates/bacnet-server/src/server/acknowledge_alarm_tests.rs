use super::*;
use bacnet_types::bitstring::EventTransitionBits;

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
use bacnet_types::enums::{EngineeringUnits, EventState};
use bacnet_types::primitives::BACnetTimeStamp;

fn request(oid: ObjectIdentifier, state: EventState) -> AcknowledgeAlarmRequest {
    AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 71,
        event_object_identifier: oid,
        event_state_acknowledged: state,
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(77),
    }
}

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
        &RequestServices {
            db: Arc::clone(db),
            notification_transactions: Arc::clone(notification_transactions),
            ..RequestServices::for_test(Arc::clone(&network), ServerConfig::default())
        },
        tracker,
        &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
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

fn acked(db: &ObjectDatabase, oid: ObjectIdentifier) -> EventTransitionBits {
    let PropertyValue::BitString { data, .. } = db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
        .unwrap()
    else {
        panic!("Acked_Transitions must be a bit string");
    };
    EventTransitionBits::from_bacnet(&data)
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
        assert_eq!(acked(&*db.read().await, oid), EventTransitionBits::all());
    }
    assert_eq!(notification_transactions.active_count(), 0);
}

#[tokio::test]
async fn post_issuance_reused_and_new_invoke_ids_are_idempotent_success() {
    let mut ai = AnalogInputObject::new(1, "AI-ack", EngineeringUnits::DEGREES_CELSIUS).unwrap();
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
    assert_eq!(acked(&*db.read().await, oid), EventTransitionBits::all());
    assert_eq!(notification_transactions.active_count(), 0);

    let reused = dispatch(
        &db,
        &tracker,
        &notification_transactions,
        &source,
        0x51,
        &request,
    )
    .await
    .unwrap();
    assert_simple_ack(reused, 0x51);
    assert_eq!(acked(&*db.read().await, oid), EventTransitionBits::all());

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
    assert_eq!(acked(&*db.read().await, oid), EventTransitionBits::all());
    assert_eq!(notification_transactions.active_count(), 0);
}

#[path = "acknowledge_alarm_notification_tests.rs"]
mod notification_tests;
