//! Access Door's BACnetDoorValue properties and Access Credential's
//! BACnetBinaryPV Credential_Status over WriteProperty and ReadProperty
//! (#979, #1073): door writes outside the closed set are refused with
//! VALUE_OUT_OF_RANGE and change nothing, Credential_Status is derived and
//! refuses every write, and the credential's Present_Value, which Table
//! 12-40 doesn't define, is an unknown property.

use super::*;
use bacnet_objects::access_control::{AccessCredentialObject, AccessDoorObject};
use bacnet_types::enums::{BinaryPV, DoorValue};

fn db_with(object: Box<dyn BACnetObject>) -> (ObjectDatabase, ObjectIdentifier) {
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(object).unwrap();
    (db, oid)
}

fn write(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
    priority: Option<u8>,
) -> Result<(), Error> {
    let mut property_value = BytesMut::new();
    encode_property_value(&mut property_value, &value).unwrap();
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: property_value.to_vec(),
        priority,
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
) -> Result<Vec<u8>, Error> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    handle_read_property(db, &request, &mut response)?;
    Ok(ReadPropertyACK::decode(&response).unwrap().property_value)
}

fn assert_property_error<T: std::fmt::Debug>(result: Result<T, Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected PROPERTY/{expected:?}, got {result:?}"
    );
}

#[test]
fn wp_access_door_present_value_holds_to_door_value() {
    let (mut db, oid) = db_with(Box::new(AccessDoorObject::new(1, "DOOR-1").unwrap()));
    let pv = PropertyIdentifier::PRESENT_VALUE;
    for &(name, value) in DoorValue::ALL_NAMED {
        write(
            &mut db,
            oid,
            pv,
            PropertyValue::Enumerated(value.to_raw()),
            Some(8),
        )
        .unwrap_or_else(|e| panic!("{name} must be accepted: {e:?}"));
        assert_eq!(
            read_wire(&db, oid, pv).unwrap(),
            [0x91, value.to_raw() as u8],
            "{name}"
        );
    }
    write(&mut db, oid, pv, PropertyValue::Enumerated(1), Some(8)).unwrap();
    let before = [
        read_wire(&db, oid, pv).unwrap(),
        read_wire(&db, oid, PropertyIdentifier::PRIORITY_ARRAY).unwrap(),
    ];
    for raw in [4, 1024, u32::MAX] {
        for priority in [Some(8), Some(1)] {
            assert_property_error(
                write(&mut db, oid, pv, PropertyValue::Enumerated(raw), priority),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
        }
    }
    assert_eq!(
        [
            read_wire(&db, oid, pv).unwrap(),
            read_wire(&db, oid, PropertyIdentifier::PRIORITY_ARRAY).unwrap(),
        ],
        before
    );
}

#[test]
fn wp_access_door_relinquish_default_holds_to_lock_or_unlock() {
    let (mut db, oid) = db_with(Box::new(AccessDoorObject::new(1, "DOOR-1").unwrap()));
    let rd = PropertyIdentifier::RELINQUISH_DEFAULT;
    write(&mut db, oid, rd, PropertyValue::Enumerated(1), None).unwrap();
    assert_eq!(read_wire(&db, oid, rd).unwrap(), [0x91, 1]);
    // Clause 12.26.11 keeps the two pulses out of Relinquish_Default
    // (#1073).
    for raw in [2, 3, 4] {
        assert_property_error(
            write(&mut db, oid, rd, PropertyValue::Enumerated(raw), None),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(read_wire(&db, oid, rd).unwrap(), [0x91, 1]);
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::PRESENT_VALUE).unwrap(),
        [0x91, 1]
    );
}

#[test]
fn wp_access_credential_status_is_read_only() {
    let (mut db, oid) = db_with(Box::new(AccessCredentialObject::new(1, "CRED-1").unwrap()));
    let cs = PropertyIdentifier::CREDENTIAL_STATUS;
    // The status follows Reason_For_Disable (#1073): a new credential has
    // no reason, so it reads ACTIVE, and no write reaches it.
    for value in [
        PropertyValue::Enumerated(BinaryPV::INACTIVE.to_raw()),
        PropertyValue::Enumerated(BinaryPV::ACTIVE.to_raw()),
        PropertyValue::Enumerated(2),
        PropertyValue::Unsigned(0),
    ] {
        assert_property_error(
            write(&mut db, oid, cs, value, None),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(read_wire(&db, oid, cs).unwrap(), [0x91, 1]);
    }
}

#[test]
fn rp_wp_access_credential_present_value_is_unknown() {
    let (mut db, oid) = db_with(Box::new(AccessCredentialObject::new(1, "CRED-1").unwrap()));
    let pv = PropertyIdentifier::PRESENT_VALUE;
    assert_property_error(read_wire(&db, oid, pv), ErrorCode::UNKNOWN_PROPERTY);
    for value in [PropertyValue::Enumerated(1), PropertyValue::Null] {
        assert_property_error(
            write(&mut db, oid, pv, value, None),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::CREDENTIAL_STATUS).unwrap(),
        [0x91, 1]
    );
}
