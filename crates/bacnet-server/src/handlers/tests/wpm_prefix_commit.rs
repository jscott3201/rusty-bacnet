use super::*;

use bacnet_objects::analog::{AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::event::{EventStateChange, EventTransition, EventTransitionCommit};
use bacnet_objects::event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject};
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{
    WriteAccessSpecification, WritePropertyAttempt, WritePropertyMultipleRequest,
};
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::primitives::BACnetTimeStamp;

fn encode_value(value: &PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut bytes, value).unwrap();
    bytes.to_vec()
}

fn encode_request(oid: ObjectIdentifier, properties: Vec<BACnetPropertyValue>) -> BytesMut {
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: properties,
        }],
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes);
    bytes
}

fn detailed(db: &mut ObjectDatabase, request: &[u8]) -> WritePropertyMultipleOutcome {
    let mut snapshots = crate::life_safety_cov::LifeSafetyCovSnapshots::default();
    handle_write_property_multiple_detailed(db, request, &mut snapshots)
}

fn assert_protocol(error: Error, class: ErrorClass, code: ErrorCode) {
    let Error::Protocol {
        class: actual_class,
        code: actual_code,
    } = error
    else {
        panic!("expected protocol error, got {error:?}");
    };
    assert_eq!(actual_class, class.to_raw() as u32);
    assert_eq!(actual_code, code.to_raw() as u32);
}

fn assert_reference(
    actual: &BACnetObjectPropertyReference,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) {
    assert_eq!(actual.object_identifier, oid);
    assert_eq!(actual.property_identifier, property.to_raw());
    assert_eq!(actual.property_array_index, index);
}

#[test]
fn event_enrollment_prefix_commits_before_read_only_first_failure() {
    let mut db = ObjectDatabase::new();
    let object = EventEnrollmentObject::new(1, "EE-1", 5).unwrap();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    let request = encode_request(
        oid,
        vec![
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::DESCRIPTION,
                property_array_index: None,
                value: encode_value(&PropertyValue::CharacterString("committed".into())),
                priority: None,
            },
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::ACKED_TRANSITIONS,
                property_array_index: None,
                value: encode_value(&PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![0xe0],
                }),
                priority: None,
            },
        ],
    );

    let WritePropertyMultipleOutcome::Error {
        error,
        first_failed_write_attempt,
        committed_oids,
    } = detailed(&mut db, &request)
    else {
        panic!("expected formal WPM failure");
    };
    assert_protocol(error, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    assert_reference(
        &first_failed_write_attempt,
        oid,
        PropertyIdentifier::ACKED_TRANSITIONS,
        None,
    );
    assert_eq!(committed_oids, vec![oid]);
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("committed".into())
    );
}

fn acked_transitions_objects() -> Vec<Box<dyn BACnetObject>> {
    let source = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    vec![
        Box::new(AnalogInputObject::new(1, "AI", 62).unwrap()),
        Box::new(AnalogOutputObject::new(1, "AO", 62).unwrap()),
        Box::new(AnalogValueObject::new(1, "AV", 62).unwrap()),
        Box::new(BinaryInputObject::new(1, "BI").unwrap()),
        Box::new(BinaryOutputObject::new(1, "BO").unwrap()),
        Box::new(BinaryValueObject::new(1, "BV").unwrap()),
        Box::new(MultiStateInputObject::new(1, "MSI", 3).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "MSO", 3).unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV", 3).unwrap()),
        Box::new(EventEnrollmentObject::new(1, "EE", 5).unwrap()),
        Box::new(AlertEnrollmentObject::new(1, "Alert", source).unwrap()),
    ]
}

fn event_snapshot(object: &dyn BACnetObject) -> [PropertyValue; 4] {
    [
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
    ]
    .map(|property| object.read_property(property, None).unwrap())
}

fn seed_unacknowledged(object: &mut dyn BACnetObject) {
    object
        .write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    if object.object_identifier().object_type() == ObjectType::ALERT_ENROLLMENT {
        object
            .set_event_state_internal(EventState::OFFNORMAL)
            .unwrap();
        object.set_acked_transitions_internal(1, false).unwrap();
    } else {
        object
            .commit_event_transition_internal(EventTransitionCommit {
                change: EventStateChange {
                    from: EventState::NORMAL,
                    to: EventState::OFFNORMAL,
                },
                coordinate: EventTransition::ToOffnormal,
                ack_required: true,
                timestamp: BACnetTimeStamp::SequenceNumber(42),
                message_text: None,
            })
            .unwrap();
    }
    assert_eq!(
        event_snapshot(object)[1],
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x60]
        }
    );
}

#[test]
fn acked_transitions_write_property_denies_every_family_without_mutation() {
    for mut object in acked_transitions_objects() {
        seed_unacknowledged(&mut *object);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        let before = event_snapshot(db.get(&oid).unwrap());
        for octet in [0xe0, 0x00, 0xa0] {
            let request = WritePropertyRequest {
                object_identifier: oid,
                property_identifier: PropertyIdentifier::ACKED_TRANSITIONS,
                property_array_index: None,
                property_value: encode_value(&PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![octet],
                }),
                priority: Some(8),
            };
            let mut bytes = BytesMut::new();
            request.encode(&mut bytes);
            assert_protocol(
                handle_write_property(&mut db, &bytes).unwrap_err(),
                ErrorClass::PROPERTY,
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert_eq!(event_snapshot(db.get(&oid).unwrap()), before, "{oid:?}");
        }
    }
}

#[test]
fn acked_transitions_wpm_authorizer_allow_retains_prefix_and_stops_at_denial() {
    // No prefix, an ordinary prefix, and a destructive detection-reset prefix.
    // None may apply the denied write or suffix; successful prefixes must NOT
    // be rolled back, even though objects retain local snapshot/restore hooks.
    for prefix_len in 0..=2 {
        for mut object in acked_transitions_objects() {
            let mut reset = event_snapshot(&*object);
            reset[3] = PropertyValue::Boolean(false);
            seed_unacknowledged(&mut *object);
            let before = event_snapshot(&*object);
            let oid = object.object_identifier();
            let mut db = ObjectDatabase::new();
            db.add(object).unwrap();
            let property = |property_identifier, value| BACnetPropertyValue {
                property_identifier,
                property_array_index: None,
                value: encode_value(&value),
                priority: None,
            };
            let mut properties = vec![
                property(
                    PropertyIdentifier::DESCRIPTION,
                    PropertyValue::CharacterString("committed".into()),
                ),
                property(
                    PropertyIdentifier::EVENT_DETECTION_ENABLE,
                    PropertyValue::Boolean(false),
                ),
            ];
            properties.truncate(prefix_len);
            let denied = BACnetPropertyValue {
                property_identifier: PropertyIdentifier::ACKED_TRANSITIONS,
                property_array_index: None,
                value: encode_value(&PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![0x00],
                }),
                priority: Some(8),
            };
            properties.push(denied.clone());
            properties.push(property(
                PropertyIdentifier::DESCRIPTION,
                PropertyValue::CharacterString("suffix".into()),
            ));
            properties.push(property(
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                PropertyValue::Boolean(true),
            ));
            let request = encode_request(oid, properties);
            let allowed = std::cell::RefCell::new(Vec::new());
            let authorize = |attempt: &WritePropertyAttempt| {
                allowed.borrow_mut().push(attempt.clone());
                Ok(())
            };
            let mut snapshots = crate::life_safety_cov::LifeSafetyCovSnapshots::default();
            let WritePropertyMultipleOutcome::Error {
                error,
                first_failed_write_attempt,
                committed_oids,
            } = handle_write_property_multiple_authorized(
                &mut db,
                &request,
                &mut snapshots,
                Some(&authorize),
            )
            else {
                panic!("expected Acked_Transitions WPM denial for {oid:?}");
            };
            assert_protocol(error, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
            assert_reference(
                &first_failed_write_attempt,
                oid,
                PropertyIdentifier::ACKED_TRANSITIONS,
                None,
            );
            assert_eq!(
                committed_oids,
                if prefix_len == 0 { vec![] } else { vec![oid] }
            );
            let allowed = allowed.into_inner();
            assert_eq!(
                allowed.len(),
                prefix_len + 1,
                "suffix reached authorizer for {oid:?}"
            );
            let attempt = allowed.last().unwrap();
            assert_eq!(attempt.reference, first_failed_write_attempt);
            assert_eq!(attempt.value, denied.value);
            assert_eq!(attempt.priority, denied.priority);
            let object = db.get(&oid).unwrap();
            assert_eq!(
                event_snapshot(object),
                if prefix_len == 2 { reset } else { before },
                "{oid:?}"
            );
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::DESCRIPTION, None)
                    .unwrap(),
                PropertyValue::CharacterString(
                    if prefix_len == 0 { "" } else { "committed" }.into()
                ),
                "{oid:?}"
            );
        }
    }
}

#[test]
fn unknown_object_is_semantic_result_with_actual_reference_before_any_commit() {
    let mut db = ObjectDatabase::new();
    let oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 999).unwrap();
    let request = encode_request(
        oid,
        vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: encode_value(&PropertyValue::Enumerated(1)),
            priority: None,
        }],
    );

    let WritePropertyMultipleOutcome::Error {
        error,
        first_failed_write_attempt,
        committed_oids,
    } = detailed(&mut db, &request)
    else {
        panic!("unknown object is a WPM Result(-), not a Reject");
    };
    assert_protocol(error, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);
    assert_reference(
        &first_failed_write_attempt,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    assert!(committed_oids.is_empty());
}

#[test]
fn semantic_index_value_name_and_write_arm_failures_keep_exact_reference() {
    let mut db = ObjectDatabase::new();
    let a = BinaryValueObject::new(1, "A").unwrap();
    let a_oid = a.object_identifier();
    db.add(Box::new(a)).unwrap();
    db.add(Box::new(BinaryValueObject::new(2, "taken").unwrap()))
        .unwrap();

    let cases = [
        (
            PropertyIdentifier::from_raw(9_999),
            None,
            encode_value(&PropertyValue::Null),
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
        (
            PropertyIdentifier::DESCRIPTION,
            Some(1),
            encode_value(&PropertyValue::CharacterString("x".into())),
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            PropertyIdentifier::DESCRIPTION,
            None,
            vec![0x09, 0x00],
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            PropertyIdentifier::OBJECT_NAME,
            None,
            encode_value(&PropertyValue::CharacterString("taken".into())),
            ErrorClass::OBJECT,
            ErrorCode::DUPLICATE_NAME,
        ),
        (
            PropertyIdentifier::OBJECT_TYPE,
            None,
            encode_value(&PropertyValue::Enumerated(5)),
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
    ];

    for (property, index, value, class, code) in cases {
        let request = encode_request(
            a_oid,
            vec![BACnetPropertyValue {
                property_identifier: property,
                property_array_index: index,
                value,
                priority: None,
            }],
        );
        let WritePropertyMultipleOutcome::Error {
            error,
            first_failed_write_attempt,
            committed_oids,
        } = detailed(&mut db, &request)
        else {
            panic!("expected semantic failure for {property:?}");
        };
        assert_protocol(error, class, code);
        assert_reference(&first_failed_write_attempt, a_oid, property, index);
        assert!(committed_oids.is_empty());
    }
}

#[test]
fn malformed_before_first_write_rejects_without_mutation() {
    let mut db = ObjectDatabase::new();
    let object = BinaryValueObject::new(1, "BV-1").unwrap();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    let mut request = encode_request(
        oid,
        vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::DESCRIPTION,
            property_array_index: None,
            value: encode_value(&PropertyValue::CharacterString("not-written".into())),
            priority: None,
        }],
    );
    request[0] = 0x1c;

    assert!(matches!(
        detailed(&mut db, &request),
        WritePropertyMultipleOutcome::Reject { reason } if reason == RejectReason::INVALID_TAG
    ));
    assert_ne!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("not-written".into())
    );
}

#[test]
fn malformed_after_prefix_uses_exact_or_sentinel_reference_and_keeps_prefix() {
    let make_db = || {
        let mut db = ObjectDatabase::new();
        db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
            .unwrap();
        db
    };
    let oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();
    let first = BACnetPropertyValue {
        property_identifier: PropertyIdentifier::DESCRIPTION,
        property_array_index: None,
        value: encode_value(&PropertyValue::CharacterString("prefix".into())),
        priority: None,
    };
    let second = BACnetPropertyValue {
        property_identifier: PropertyIdentifier::OBJECT_TYPE,
        property_array_index: Some(4),
        value: encode_value(&PropertyValue::Enumerated(5)),
        priority: None,
    };

    let mut exact_wire = encode_request(oid, vec![first.clone(), second.clone()]);
    let mut second_wire = BytesMut::new();
    second.encode(&mut second_wire);
    let second_start = exact_wire
        .windows(second_wire.len())
        .position(|window| window == second_wire.as_ref())
        .unwrap();
    let value_close = second_start + second_wire.iter().rposition(|byte| *byte == 0x2f).unwrap();
    exact_wire[value_close] = 0x3f;
    let mut db = make_db();
    let WritePropertyMultipleOutcome::Error {
        error,
        first_failed_write_attempt,
        committed_oids,
    } = detailed(&mut db, &exact_wire)
    else {
        panic!("expected post-prefix syntax Result(-)");
    };
    assert_protocol(error, ErrorClass::SERVICES, ErrorCode::INVALID_TAG);
    assert_reference(
        &first_failed_write_attempt,
        oid,
        PropertyIdentifier::OBJECT_TYPE,
        Some(4),
    );
    assert_eq!(committed_oids, vec![oid]);

    let mut sentinel_wire = encode_request(oid, vec![first]);
    sentinel_wire.extend_from_slice(&[0xff]);
    let mut db = make_db();
    let WritePropertyMultipleOutcome::Error {
        first_failed_write_attempt,
        committed_oids,
        ..
    } = detailed(&mut db, &sentinel_wire)
    else {
        panic!("expected post-prefix sentinel Result(-)");
    };
    assert_eq!(
        first_failed_write_attempt
            .object_identifier
            .instance_number(),
        ObjectIdentifier::MAX_INSTANCE
    );
    assert_eq!(
        first_failed_write_attempt.object_identifier.object_type(),
        ObjectType::DEVICE
    );
    assert_eq!(
        first_failed_write_attempt.property_identifier,
        PropertyIdentifier::ALL.to_raw()
    );
    assert_eq!(first_failed_write_attempt.property_array_index, None);
    assert_eq!(committed_oids, vec![oid]);
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("prefix".into())
    );
}
