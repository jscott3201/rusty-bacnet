//! The Loop rows #1062 added, over ReadProperty and WriteProperty:
//! Controlled_Variable_Units, Action, Priority_For_Writing and the gain units
//! rows read back with their datatypes, Action takes DIRECT or REVERSE only,
//! the application-set rows refuse writes, and Action on a Loop takes no array
//! index although Command's ACTION array still does.

use super::*;
use bacnet_objects::command::CommandObject;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_types::enums::{Action, EngineeringUnits};
use bacnet_types::error::RemoteError;

fn loop_db() -> (ObjectDatabase, ObjectIdentifier) {
    let mut lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    lo.set_controlled_variable_units(EngineeringUnits::DEGREES_CELSIUS)
        .unwrap();
    lo.set_proportional_constant_units(EngineeringUnits::PERCENT)
        .unwrap();
    lo.set_priority_for_writing(8).unwrap();
    let oid = lo.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(lo)).unwrap();
    db.add(Box::new(CommandObject::new(1, "CMD-1").unwrap()))
        .unwrap();
    (db, oid)
}

fn write(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    value: PropertyValue,
) -> Result<(), Error> {
    let mut property_value = BytesMut::new();
    encode_property_value(&mut property_value, &value).unwrap();
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: array_index,
        property_value: property_value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

/// The raw propertyValue a ReadProperty ACK carries.
fn read_wire(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
) -> Result<Vec<u8>, Error> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: array_index,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    handle_read_property(db, &request, &mut response)?;
    Ok(ReadPropertyACK::decode(&response).unwrap().property_value)
}

fn assert_property_error<T: std::fmt::Debug>(result: Result<T, Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected PROPERTY/{expected:?}, got {result:?}"
    );
}

#[test]
fn rp_loop_serves_units_action_and_priority_for_writing() {
    let (db, oid) = loop_db();
    for (property, bytes) in [
        // Application-tagged Enumerated (tag 9) and Unsigned (tag 2).
        (
            PropertyIdentifier::CONTROLLED_VARIABLE_UNITS,
            vec![0x91, 62],
        ),
        (
            PropertyIdentifier::PROPORTIONAL_CONSTANT_UNITS,
            vec![0x91, 98],
        ),
        (PropertyIdentifier::INTEGRAL_CONSTANT_UNITS, vec![0x91, 95]),
        (
            PropertyIdentifier::DERIVATIVE_CONSTANT_UNITS,
            vec![0x91, 95],
        ),
        (PropertyIdentifier::ACTION, vec![0x91, 0]),
        (PropertyIdentifier::PRIORITY_FOR_WRITING, vec![0x21, 8]),
    ] {
        assert_eq!(
            read_wire(&db, oid, property, None).unwrap(),
            bytes,
            "{property:?}"
        );
    }
}

#[test]
fn wp_loop_action_holds_to_bacnet_action() {
    let (mut db, oid) = loop_db();
    let action = PropertyIdentifier::ACTION;
    write(
        &mut db,
        oid,
        action,
        None,
        PropertyValue::Enumerated(Action::REVERSE.to_raw()),
    )
    .unwrap();
    assert_eq!(read_wire(&db, oid, action, None).unwrap(), [0x91, 1]);
    for (value, code) in [
        (PropertyValue::Enumerated(2), ErrorCode::VALUE_OUT_OF_RANGE),
        (PropertyValue::Unsigned(0), ErrorCode::INVALID_DATA_TYPE),
    ] {
        assert_property_error(write(&mut db, oid, action, None, value), code);
        assert_eq!(read_wire(&db, oid, action, None).unwrap(), [0x91, 1]);
    }
}

#[test]
fn wp_loop_application_set_rows_are_read_only() {
    let (mut db, oid) = loop_db();
    for (property, value) in [
        (
            PropertyIdentifier::CONTROLLED_VARIABLE_UNITS,
            PropertyValue::Enumerated(95),
        ),
        (
            PropertyIdentifier::PROPORTIONAL_CONSTANT_UNITS,
            PropertyValue::Enumerated(95),
        ),
        (
            PropertyIdentifier::INTEGRAL_CONSTANT_UNITS,
            PropertyValue::Enumerated(62),
        ),
        (
            PropertyIdentifier::DERIVATIVE_CONSTANT_UNITS,
            PropertyValue::Enumerated(62),
        ),
        (
            PropertyIdentifier::PRIORITY_FOR_WRITING,
            PropertyValue::Unsigned(1),
        ),
    ] {
        let before = read_wire(&db, oid, property, None).unwrap();
        assert_property_error(
            write(&mut db, oid, property, None, value),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(read_wire(&db, oid, property, None).unwrap(), before);
    }
}

#[test]
fn loop_action_rejects_an_array_index_but_command_action_admits_one() {
    let (mut db, oid) = loop_db();
    let action = PropertyIdentifier::ACTION;
    assert_property_error(
        read_wire(&db, oid, action, Some(1)),
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
    assert_property_error(
        write(
            &mut db,
            oid,
            action,
            Some(1),
            PropertyValue::Enumerated(Action::REVERSE.to_raw()),
        ),
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
    assert_eq!(read_wire(&db, oid, action, None).unwrap(), [0x91, 0]);
    let command = ObjectIdentifier::new(ObjectType::COMMAND, 1).unwrap();
    read_wire(&db, command, action, Some(0)).unwrap();
}
