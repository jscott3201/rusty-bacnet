use super::*;

use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::constructed::BACnetObjectPropertyReference;

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
