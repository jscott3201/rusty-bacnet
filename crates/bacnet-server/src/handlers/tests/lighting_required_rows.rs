//! Lighting rows #1092 added, over WriteProperty and ReadProperty:
//! Default_Ramp_Rate and Default_Step_Increment on Lighting Output, and
//! Current_Command_Priority on both lighting objects. Lighting Output's
//! Default_Fade_Time became a writable row with them in #1111.

use super::*;
use bacnet_objects::lighting::{BinaryLightingOutputObject, LightingOutputObject};

pub(super) fn db_with(object: Box<dyn BACnetObject>) -> (ObjectDatabase, ObjectIdentifier) {
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(object).unwrap();
    (db, oid)
}

pub(super) fn write_wire(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
    priority: Option<u8>,
) -> Result<(), Error> {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &value).unwrap();
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: encoded.to_vec(),
        priority,
    }
    .encode(&mut request)
    .unwrap();
    sourced_wp(db, &request).map(|_| ())
}

pub(super) fn read_wire(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    handle_read_property(db, &request, &mut response).unwrap();
    ReadPropertyACK::decode(&response).unwrap().property_value
}

pub(super) fn assert_refused(result: Result<(), Error>, expected: ErrorCode) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(code, expected, "expected {expected:?}");
        }
        other => panic!("expected PROPERTY/{expected:?}, got {other:?}"),
    }
}

#[test]
fn lighting_output_default_ramp_rate_and_step_increment_over_write_property() {
    let (mut db, oid) = db_with(Box::new(LightingOutputObject::new(1, "LO-1").unwrap()));
    for property in [
        PropertyIdentifier::DEFAULT_RAMP_RATE,
        PropertyIdentifier::DEFAULT_STEP_INCREMENT,
    ] {
        // 2.5 is 0x40200000; the range ends 0.1 and 100.0 are accepted.
        write_wire(&mut db, oid, property, PropertyValue::Real(2.5), None).unwrap();
        assert_eq!(
            read_wire(&db, oid, property),
            [0x44, 0x40, 0x20, 0x00, 0x00]
        );
        for edge in [0.1, 100.0] {
            write_wire(&mut db, oid, property, PropertyValue::Real(edge), None).unwrap();
        }
        write_wire(&mut db, oid, property, PropertyValue::Real(2.5), None).unwrap();
        for outside in [0.0, 100.5] {
            assert_refused(
                write_wire(&mut db, oid, property, PropertyValue::Real(outside), None),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
        }
        assert_refused(
            write_wire(&mut db, oid, property, PropertyValue::Unsigned(3), None),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_eq!(
            read_wire(&db, oid, property),
            [0x44, 0x40, 0x20, 0x00, 0x00]
        );
    }
}

#[test]
fn lighting_output_default_fade_time_over_write_property() {
    let fade = PropertyIdentifier::DEFAULT_FADE_TIME;
    let (mut db, oid) = db_with(Box::new(LightingOutputObject::new(1, "LO-1").unwrap()));
    // A new object serves 100 ms, the floor of Clause 12.54.16's range.
    assert_eq!(read_wire(&db, oid, fade), [0x21, 100]);
    for edge in [100, 86_400_000] {
        write_wire(&mut db, oid, fade, PropertyValue::Unsigned(edge), None).unwrap();
    }
    // 86,400,000 is 0x05265C00, a four-octet Unsigned.
    assert_eq!(read_wire(&db, oid, fade), [0x24, 0x05, 0x26, 0x5C, 0x00]);
    write_wire(&mut db, oid, fade, PropertyValue::Unsigned(1_500), None).unwrap();
    for outside in [0, 99, 86_400_001] {
        assert_refused(
            write_wire(&mut db, oid, fade, PropertyValue::Unsigned(outside), None),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_refused(
        write_wire(&mut db, oid, fade, PropertyValue::Real(500.0), None),
        ErrorCode::INVALID_DATA_TYPE,
    );
    // 1,500 is 0x05DC.
    assert_eq!(read_wire(&db, oid, fade), [0x22, 0x05, 0xDC]);
}

#[test]
fn current_command_priority_follows_present_value_commands_over_the_wire() {
    let cases: [(Box<dyn BACnetObject>, PropertyValue); 2] = [
        (
            Box::new(LightingOutputObject::new(1, "LO-1").unwrap()),
            PropertyValue::Real(80.0),
        ),
        (
            Box::new(BinaryLightingOutputObject::new(1, "BLO-1").unwrap()),
            PropertyValue::Enumerated(1),
        ),
    ];
    let ccp = PropertyIdentifier::CURRENT_COMMAND_PRIORITY;
    for (object, on) in cases {
        let (mut db, oid) = db_with(object);
        // Null (0x00) while Relinquish_Default is in effect.
        assert_eq!(read_wire(&db, oid, ccp), [0x00]);
        let pv = PropertyIdentifier::PRESENT_VALUE;
        write_wire(&mut db, oid, pv, on.clone(), Some(10)).unwrap();
        assert_eq!(read_wire(&db, oid, ccp), [0x21, 10]);
        write_wire(&mut db, oid, pv, on, Some(4)).unwrap();
        assert_eq!(read_wire(&db, oid, ccp), [0x21, 4]);
        write_wire(&mut db, oid, pv, PropertyValue::Null, Some(4)).unwrap();
        assert_eq!(read_wire(&db, oid, ccp), [0x21, 10]);
        // No write reaches it.
        assert_refused(
            write_wire(&mut db, oid, ccp, PropertyValue::Unsigned(1), None),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        write_wire(&mut db, oid, pv, PropertyValue::Null, Some(10)).unwrap();
        assert_eq!(read_wire(&db, oid, ccp), [0x00]);
    }
}
