use super::*;

use bacnet_encoding::primitives;
use bacnet_encoding::tags::{encode_tag, TagClass};
use bacnet_objects::analog::{AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::BinaryInputObject;
use bacnet_objects::event::{EventStateChange, EventTransitionCommit};
use bacnet_objects::event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject};
use bacnet_objects::multistate::MultiStateInputObject;
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
use bacnet_types::enums::EventType;

fn add_committed<O: BACnetObject + 'static>(
    db: &mut ObjectDatabase,
    mut object: O,
    state: EventState,
    timestamp: BACnetTimeStamp,
) -> ObjectIdentifier {
    let oid = object.object_identifier();
    object
        .commit_event_transition_internal(EventTransitionCommit {
            change: EventStateChange {
                from: EventState::NORMAL,
                to: state,
            },
            coordinate: bacnet_objects::event::EventTransition::for_target_state(state),
            ack_required: true,
            timestamp,
            message_text: Some("committed".into()),
        })
        .unwrap();
    db.add(Box::new(object)).unwrap();
    oid
}

fn encode_request(
    oid: ObjectIdentifier,
    state: EventState,
    timestamp: BACnetTimeStamp,
) -> BytesMut {
    let request = AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 17,
        event_object_identifier: oid,
        event_state_acknowledged: state.to_raw(),
        timestamp,
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(88),
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded).unwrap();
    encoded
}

fn encode_request_with_raw_source(oid: ObjectIdentifier, source: &[u8]) -> BytesMut {
    let mut encoded = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut encoded, 0, 17);
    primitives::encode_ctx_object_id(&mut encoded, 1, &oid);
    primitives::encode_ctx_enumerated(&mut encoded, 2, EventState::HIGH_LIMIT.to_raw());
    primitives::encode_timestamp(&mut encoded, 3, &BACnetTimeStamp::SequenceNumber(42)).unwrap();
    encode_tag(&mut encoded, 4, TagClass::Context, source.len() as u32);
    encoded.extend_from_slice(source);
    primitives::encode_timestamp(&mut encoded, 5, &BACnetTimeStamp::SequenceNumber(88)).unwrap();
    encoded
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

fn snapshot(db: &ObjectDatabase, oid: ObjectIdentifier) -> (PropertyValue, PropertyValue, u8) {
    let object = db.get(&oid).unwrap();
    (
        object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
            .unwrap(),
        acked(db, oid),
    )
}

fn assert_protocol(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(matches!(
        error,
        Error::Protocol { class: actual_class, code: actual_code }
            if actual_class == class.to_raw() as u32 && actual_code == code.to_raw() as u32
    ));
}

fn configured_event_enrollment() -> EventEnrollmentObject {
    let mut object =
        EventEnrollmentObject::new(1, "EE-ack", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    object.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 77).unwrap(),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    object.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 0.0,
        high_limit: 1.0,
        deadband: 0.1,
    });
    object
}

#[test]
fn supported_families_acknowledge_only_the_correlated_transition() {
    let mut db = ObjectDatabase::new();
    let stamp = BACnetTimeStamp::SequenceNumber(42);
    let oids = [
        add_committed(
            &mut db,
            AnalogInputObject::new(1, "AI-ack", 62).unwrap(),
            EventState::HIGH_LIMIT,
            stamp.clone(),
        ),
        add_committed(
            &mut db,
            AnalogOutputObject::new(1, "AO-ack", 62).unwrap(),
            EventState::HIGH_LIMIT,
            stamp.clone(),
        ),
        add_committed(
            &mut db,
            AnalogValueObject::new(1, "AV-ack", 62).unwrap(),
            EventState::HIGH_LIMIT,
            stamp.clone(),
        ),
        add_committed(
            &mut db,
            configured_event_enrollment(),
            EventState::HIGH_LIMIT,
            stamp.clone(),
        ),
    ];

    for oid in oids {
        assert_eq!(acked(&db, oid), 0b110);
        handle_acknowledge_alarm(
            &mut db,
            &encode_request(oid, EventState::HIGH_LIMIT, stamp.clone()),
        )
        .unwrap();
        assert_eq!(acked(&db, oid), 0b111);
    }
}

#[test]
fn state_and_timestamp_mismatches_return_exact_errors_without_mutation() {
    let mut db = ObjectDatabase::new();
    let oid = add_committed(
        &mut db,
        AnalogInputObject::new(1, "AI-invalid", 62).unwrap(),
        EventState::LOW_LIMIT,
        BACnetTimeStamp::SequenceNumber(42),
    );
    let before = snapshot(&db, oid);

    let state_error = handle_acknowledge_alarm(
        &mut db,
        &encode_request(
            oid,
            EventState::HIGH_LIMIT,
            BACnetTimeStamp::SequenceNumber(99),
        ),
    )
    .unwrap_err();
    assert_protocol(
        state_error,
        ErrorClass::SERVICES,
        ErrorCode::INVALID_EVENT_STATE,
    );
    assert_eq!(snapshot(&db, oid), before);

    let timestamp_error = handle_acknowledge_alarm(
        &mut db,
        &encode_request(
            oid,
            EventState::LOW_LIMIT,
            BACnetTimeStamp::SequenceNumber(41),
        ),
    )
    .unwrap_err();
    assert_protocol(
        timestamp_error,
        ErrorClass::SERVICES,
        ErrorCode::INVALID_TIME_STAMP,
    );
    assert_eq!(snapshot(&db, oid), before);
}

#[test]
fn unknown_uninitialized_and_unsupported_objects_fail_closed() {
    let missing = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();
    let mut empty = ObjectDatabase::new();
    let error = handle_acknowledge_alarm(
        &mut empty,
        &encode_request(
            missing,
            EventState::HIGH_LIMIT,
            BACnetTimeStamp::SequenceNumber(0),
        ),
    )
    .unwrap_err();
    assert_protocol(error, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);

    let mut db = ObjectDatabase::new();
    let objects: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(BinaryInputObject::new(1, "BI-unsupported").unwrap()),
        Box::new(MultiStateInputObject::new(1, "MSI-unsupported", 3).unwrap()),
        Box::new(
            AlertEnrollmentObject::new(
                1,
                "AE-unsupported",
                ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap(),
            )
            .unwrap(),
        ),
    ];
    for object in objects {
        let oid = object.object_identifier();
        db.add(object).unwrap();
        let before = acked(&db, oid);
        let error = handle_acknowledge_alarm(
            &mut db,
            &encode_request(
                oid,
                EventState::HIGH_LIMIT,
                BACnetTimeStamp::SequenceNumber(0),
            ),
        )
        .unwrap_err();
        assert_protocol(error, ErrorClass::OBJECT, ErrorCode::NO_ALARM_CONFIGURED);
        assert_eq!(acked(&db, oid), before);
    }

    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 8).unwrap();
    db.add(Box::new(AnalogInputObject::new(8, "AI-empty", 62).unwrap()))
        .unwrap();
    let before = snapshot(&db, oid);
    let error = handle_acknowledge_alarm(
        &mut db,
        &encode_request(
            oid,
            EventState::HIGH_LIMIT,
            BACnetTimeStamp::SequenceNumber(0),
        ),
    )
    .unwrap_err();
    assert_protocol(error, ErrorClass::SERVICES, ErrorCode::INVALID_TIME_STAMP);
    assert_eq!(snapshot(&db, oid), before);

    let unconfigured = EventEnrollmentObject::new(8, "EE-unconfigured", 0).unwrap();
    let oid = unconfigured.object_identifier();
    db.add(Box::new(unconfigured)).unwrap();
    let before = acked(&db, oid);
    let error = handle_acknowledge_alarm(
        &mut db,
        &encode_request(
            oid,
            EventState::OFFNORMAL,
            BACnetTimeStamp::SequenceNumber(0),
        ),
    )
    .unwrap_err();
    assert_protocol(error, ErrorClass::OBJECT, ErrorCode::NO_ALARM_CONFIGURED);
    assert_eq!(acked(&db, oid), before);
}

#[test]
fn disabled_analog_input_returns_no_alarm_configured_before_correlation() {
    let mut db = ObjectDatabase::new();
    let oid = add_committed(
        &mut db,
        AnalogInputObject::new(1, "AI-disabled", 62).unwrap(),
        EventState::HIGH_LIMIT,
        BACnetTimeStamp::SequenceNumber(42),
    );
    db.get_mut(&oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    let before = snapshot(&db, oid);

    let error = handle_acknowledge_alarm(
        &mut db,
        &encode_request(
            oid,
            EventState::HIGH_LIMIT,
            BACnetTimeStamp::SequenceNumber(42),
        ),
    )
    .unwrap_err();
    assert_protocol(error, ErrorClass::OBJECT, ErrorCode::NO_ALARM_CONFIGURED);
    assert_eq!(snapshot(&db, oid), before);
}

#[test]
fn invalid_source_text_is_sanitized_but_malformed_source_framing_is_rejected() {
    let mut db = ObjectDatabase::new();
    let oid = add_committed(
        &mut db,
        AnalogInputObject::new(1, "AI-source", 62).unwrap(),
        EventState::HIGH_LIMIT,
        BACnetTimeStamp::SequenceNumber(42),
    );

    for source in [
        &[1, 0xff][..],
        &[0xff, 0xff][..],
        &[0, 0xff][..],
        &[4, 0xd8, 0x00][..],
    ] {
        handle_acknowledge_alarm(&mut db, &encode_request_with_raw_source(oid, source)).unwrap();
        assert_eq!(acked(&db, oid), 0b111);
    }

    let before = snapshot(&db, oid);
    let error = handle_acknowledge_alarm(&mut db, &encode_request_with_raw_source(oid, &[]));
    assert!(matches!(error, Err(Error::Decoding { .. })));
    assert_eq!(snapshot(&db, oid), before);
}
