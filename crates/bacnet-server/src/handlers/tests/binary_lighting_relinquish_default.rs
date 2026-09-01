//! Focused WP/WPM coverage for Binary Lighting Output Relinquish_Default.

use super::*;
use bacnet_objects::lighting::BinaryLightingOutputObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};

fn encode_value(value: PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, &value).unwrap();
    bytes.to_vec()
}

fn binary_lighting_output_db() -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let object = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    (db, oid)
}

fn write_wire(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
    priority: Option<u8>,
) -> Result<(), Error> {
    let request = WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: encode_value(value),
        priority,
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes);
    handle_write_property(db, &bytes).map(|_| ())
}

fn write_multiple_relinquish_defaults(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    values: &[u32],
) -> Result<Vec<ObjectIdentifier>, Error> {
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: values
                .iter()
                .map(|value| BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::RELINQUISH_DEFAULT,
                    property_array_index: None,
                    value: encode_value(PropertyValue::Enumerated(*value)),
                    priority: None,
                })
                .collect(),
        }],
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes);
    handle_write_property_multiple(db, &bytes)
}

fn read_property(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
) -> PropertyValue {
    db.get(&oid)
        .unwrap()
        .read_property(property, array_index)
        .unwrap()
}

fn assert_value_out_of_range<T>(result: Result<T, Error>) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32);
        }
        Err(other) => panic!("expected PROPERTY/VALUE_OUT_OF_RANGE, got {other:?}"),
        Ok(_) => panic!("expected PROPERTY/VALUE_OUT_OF_RANGE, got success"),
    }
}

fn assert_command_state(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    relinquish_default: u32,
    present_value: u32,
    priority_8: PropertyValue,
) {
    assert_eq!(
        read_property(db, oid, PropertyIdentifier::RELINQUISH_DEFAULT, None),
        PropertyValue::Enumerated(relinquish_default)
    );
    assert_eq!(
        read_property(db, oid, PropertyIdentifier::PRESENT_VALUE, None),
        PropertyValue::Enumerated(present_value)
    );
    assert_eq!(
        read_property(db, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(8)),
        priority_8
    );
}

#[test]
fn binary_lighting_output_relinquish_default_accepts_off_on_over_write_property() {
    let (mut db, oid) = binary_lighting_output_db();

    for value in [1, 0] {
        write_wire(
            &mut db,
            oid,
            PropertyIdentifier::RELINQUISH_DEFAULT,
            PropertyValue::Enumerated(value),
            None,
        )
        .expect("OFF/ON Relinquish_Default must be accepted");
        assert_command_state(&db, oid, value, value, PropertyValue::Null);
    }
}

#[test]
fn binary_lighting_output_relinquish_default_rejects_non_binary_values_over_write_property() {
    let (mut db, oid) = binary_lighting_output_db();
    write_wire(
        &mut db,
        oid,
        PropertyIdentifier::RELINQUISH_DEFAULT,
        PropertyValue::Enumerated(1),
        None,
    )
    .unwrap();

    for value in 2..=5 {
        assert_value_out_of_range(write_wire(
            &mut db,
            oid,
            PropertyIdentifier::RELINQUISH_DEFAULT,
            PropertyValue::Enumerated(value),
            None,
        ));
        assert_command_state(&db, oid, 1, 1, PropertyValue::Null);
    }
}

#[test]
fn binary_lighting_output_failed_wpm_restores_default_present_value_and_priority() {
    let (mut db, oid) = binary_lighting_output_db();
    write_wire(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Enumerated(1),
        Some(8),
    )
    .unwrap();
    let priority_before = read_property(&db, oid, PropertyIdentifier::PRIORITY_ARRAY, None);
    assert_command_state(&db, oid, 0, 1, PropertyValue::Enumerated(1));

    assert_value_out_of_range(write_multiple_relinquish_defaults(&mut db, oid, &[1, 2]));
    assert_command_state(&db, oid, 0, 1, PropertyValue::Enumerated(1));
    assert_eq!(
        read_property(&db, oid, PropertyIdentifier::PRIORITY_ARRAY, None),
        priority_before,
        "failed WPM must restore the complete priority array"
    );

    write_wire(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Null,
        Some(8),
    )
    .unwrap();
    assert_command_state(&db, oid, 0, 0, PropertyValue::Null);
}
