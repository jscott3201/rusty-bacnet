//! Tracking_Value and Reliability writes on Life Safety Point and Zone over
//! WriteProperty and WritePropertyMultiple: taken while Out_Of_Service is
//! TRUE, refused in service (Clauses 12.15.11 and 12.16.11, #1108).

use super::*;
use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::enums::{LifeSafetyState, Reliability};

/// One Point and one Zone, both in service with a Tracking_Value of
/// PRE_ALARM.
fn life_safety_db() -> (ObjectDatabase, [ObjectIdentifier; 2]) {
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point.set_tracking_value(LifeSafetyState::PRE_ALARM);
    let mut zone = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    zone.set_tracking_value(LifeSafetyState::PRE_ALARM);
    let oids = [point.object_identifier(), zone.object_identifier()];
    let mut db = ObjectDatabase::new();
    db.add(Box::new(point)).unwrap();
    db.add(Box::new(zone)).unwrap();
    (db, oids)
}

fn encode(value: &PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut buf, value).unwrap();
    buf.to_vec()
}

fn enumerated(raw: u32) -> PropertyValue {
    PropertyValue::Enumerated(raw)
}

fn write_property(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: encode(&value),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

fn write_property_multiple(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    writes: &[(PropertyIdentifier, PropertyValue)],
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: writes
                .iter()
                .map(|(property, value)| BACnetPropertyValue {
                    property_identifier: *property,
                    property_array_index: None,
                    value: encode(value),
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property_multiple(db, &request).map(|_| ())
}

/// The ReadProperty-ACK value bytes of one property.
fn read_bytes(db: &ObjectDatabase, oid: ObjectIdentifier, property: PropertyIdentifier) -> Vec<u8> {
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

fn assert_property_error(result: Result<(), Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected PROPERTY / {expected:?}, got {result:?}"
    );
}

const PRE_ALARM: [u8; 2] = [0x91, 1];
const ALARM: [u8; 2] = [0x91, 2];
const NO_FAULT_DETECTED: [u8; 2] = [0x91, 0];
const NO_SENSOR: [u8; 2] = [0x91, 1];
/// Status_Flags with only OUT_OF_SERVICE set, then with FAULT as well.
const OUT_OF_SERVICE_FLAGS: [u8; 3] = [0x82, 0x04, 0x10];
const FAULT_FLAGS: [u8; 3] = [0x82, 0x04, 0x50];

#[test]
fn write_property_takes_tracking_value_and_reliability_only_out_of_service() {
    let (mut db, oids) = life_safety_db();
    for oid in oids {
        for (property, value) in [
            (
                PropertyIdentifier::TRACKING_VALUE,
                LifeSafetyState::ALARM.to_raw(),
            ),
            (
                PropertyIdentifier::RELIABILITY,
                Reliability::NO_SENSOR.to_raw(),
            ),
        ] {
            assert_property_error(
                write_property(&mut db, oid, property, enumerated(value)),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::TRACKING_VALUE),
            PRE_ALARM
        );
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::RELIABILITY),
            NO_FAULT_DETECTED
        );

        write_property(
            &mut db,
            oid,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyValue::Boolean(true),
        )
        .unwrap();
        write_property(
            &mut db,
            oid,
            PropertyIdentifier::TRACKING_VALUE,
            enumerated(LifeSafetyState::ALARM.to_raw()),
        )
        .unwrap();
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::TRACKING_VALUE),
            ALARM
        );
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::STATUS_FLAGS),
            OUT_OF_SERVICE_FLAGS
        );
        write_property(
            &mut db,
            oid,
            PropertyIdentifier::RELIABILITY,
            enumerated(Reliability::NO_SENSOR.to_raw()),
        )
        .unwrap();
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::RELIABILITY),
            NO_SENSOR
        );
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::STATUS_FLAGS),
            FAULT_FLAGS
        );
    }
}

#[test]
fn write_property_multiple_takes_tracking_value_and_reliability_only_out_of_service() {
    let (mut db, oids) = life_safety_db();
    let simulation = [
        (
            PropertyIdentifier::TRACKING_VALUE,
            enumerated(LifeSafetyState::ALARM.to_raw()),
        ),
        (
            PropertyIdentifier::RELIABILITY,
            enumerated(Reliability::NO_SENSOR.to_raw()),
        ),
    ];
    for oid in oids {
        // In service the first write is refused and nothing changes.
        assert_property_error(
            write_property_multiple(&mut db, oid, &simulation),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        // Reliability alone is refused the same way.
        assert_property_error(
            write_property_multiple(&mut db, oid, &simulation[1..]),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::TRACKING_VALUE),
            PRE_ALARM
        );
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::RELIABILITY),
            NO_FAULT_DETECTED
        );

        // Out_Of_Service first, then the two simulated values, in one request.
        let mut writes = vec![(
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyValue::Boolean(true),
        )];
        writes.extend(simulation.iter().cloned());
        write_property_multiple(&mut db, oid, &writes).unwrap();
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::TRACKING_VALUE),
            ALARM
        );
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::RELIABILITY),
            NO_SENSOR
        );
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::STATUS_FLAGS),
            FAULT_FLAGS
        );

        // Back in service the device's values return and writes are refused
        // again.
        write_property_multiple(
            &mut db,
            oid,
            &[(
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(false),
            )],
        )
        .unwrap();
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::TRACKING_VALUE),
            PRE_ALARM
        );
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::RELIABILITY),
            NO_FAULT_DETECTED
        );
        assert_property_error(
            write_property_multiple(&mut db, oid, &simulation),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
}

#[test]
fn out_of_service_writes_outside_the_datatypes_are_refused_unchanged() {
    let (mut db, oids) = life_safety_db();
    for oid in oids {
        write_property(
            &mut db,
            oid,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyValue::Boolean(true),
        )
        .unwrap();
        for (property, value, code) in [
            // 100 is reserved for ASHRAE, past 65535 outside the datatype.
            (
                PropertyIdentifier::TRACKING_VALUE,
                enumerated(100),
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            (
                PropertyIdentifier::TRACKING_VALUE,
                enumerated(65_536),
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            (
                PropertyIdentifier::TRACKING_VALUE,
                PropertyValue::Unsigned(2),
                ErrorCode::INVALID_DATA_TYPE,
            ),
            // 11 is reserved inside the named Reliability span.
            (
                PropertyIdentifier::RELIABILITY,
                enumerated(11),
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            (
                PropertyIdentifier::RELIABILITY,
                PropertyValue::Boolean(true),
                ErrorCode::INVALID_DATA_TYPE,
            ),
        ] {
            assert_property_error(write_property(&mut db, oid, property, value.clone()), code);
            // WPM stops at the refused value after committing the valid one
            // before it.
            assert_property_error(
                write_property_multiple(
                    &mut db,
                    oid,
                    &[
                        (
                            PropertyIdentifier::TRACKING_VALUE,
                            enumerated(LifeSafetyState::ALARM.to_raw()),
                        ),
                        (property, value),
                    ],
                ),
                code,
            );
            assert_eq!(
                read_bytes(&db, oid, PropertyIdentifier::TRACKING_VALUE),
                ALARM
            );
            assert_eq!(
                read_bytes(&db, oid, PropertyIdentifier::RELIABILITY),
                NO_FAULT_DETECTED
            );
            write_property(
                &mut db,
                oid,
                PropertyIdentifier::TRACKING_VALUE,
                enumerated(LifeSafetyState::PRE_ALARM.to_raw()),
            )
            .unwrap();
        }
    }
}
