//! Mode writes against Accepted_Modes on Life Safety Point and Zone over
//! WriteProperty and WritePropertyMultiple (Clauses 12.15.13 and 12.16.13,
//! #1092).

use super::*;
use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::enums::{LifeSafetyMode, LifeSafetyState};
use bacnet_types::error::RemoteError;

/// One Point and one Zone, both accepting only OFF, ON and TEST.
fn life_safety_db() -> (ObjectDatabase, [ObjectIdentifier; 2]) {
    let accepted = [
        LifeSafetyMode::OFF,
        LifeSafetyMode::ON,
        LifeSafetyMode::TEST,
    ];
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point.set_accepted_modes(accepted);
    let mut zone = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    zone.set_accepted_modes(accepted);
    zone.set_tracking_value(LifeSafetyState::ALARM);
    let oids = [point.object_identifier(), zone.object_identifier()];
    let mut db = ObjectDatabase::new();
    db.add(Box::new(point)).unwrap();
    db.add(Box::new(zone)).unwrap();
    (db, oids)
}

fn enumerated(raw: u32) -> Vec<u8> {
    let mut buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut buf, raw);
    buf.to_vec()
}

fn write_property(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: Vec<u8>,
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: value,
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
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
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected PROPERTY / {expected:?}, got {result:?}"
    );
}

#[test]
fn write_property_takes_a_listed_mode_and_refuses_any_other() {
    let (mut db, oids) = life_safety_db();
    for oid in oids {
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::ACCEPTED_MODES),
            [0x91, 0, 0x91, 1, 0x91, 2]
        );
        write_property(&mut db, oid, PropertyIdentifier::MODE, enumerated(2)).unwrap();
        assert_eq!(read_bytes(&db, oid, PropertyIdentifier::MODE), [0x91, 2]);
        // ARMED is a standard mode this object does not list; 300 is
        // proprietary and unlisted.
        for refused in [LifeSafetyMode::ARMED.to_raw(), 300] {
            assert_property_error(
                write_property(&mut db, oid, PropertyIdentifier::MODE, enumerated(refused)),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            assert_eq!(read_bytes(&db, oid, PropertyIdentifier::MODE), [0x91, 2]);
        }
    }
}

#[test]
fn write_property_multiple_stops_at_the_unlisted_mode_after_committing_the_prefix() {
    let (mut db, oids) = life_safety_db();
    for oid in oids {
        let mode = |raw| BACnetPropertyValue {
            property_identifier: PropertyIdentifier::MODE,
            property_array_index: None,
            value: enumerated(raw),
            priority: None,
        };
        let mut request = BytesMut::new();
        WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: oid,
                list_of_properties: vec![
                    mode(LifeSafetyMode::ON.to_raw()),
                    mode(LifeSafetyMode::DISARMED.to_raw()),
                ],
            }],
        }
        .encode(&mut request)
        .unwrap();
        assert_property_error(
            handle_write_property_multiple(&mut db, &request).map(|_| ()),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::MODE),
            [0x91, 1],
            "the listed ON before the refused DISARMED stays committed"
        );
    }
}

/// The Zone is in service here; out of service its Tracking_Value takes
/// writes (`life_safety_oos_writes.rs`, #1108).
#[test]
fn accepted_modes_and_the_in_service_zone_tracking_value_refuse_writes() {
    let (mut db, [point, zone]) = life_safety_db();
    for oid in [point, zone] {
        let mut list = BytesMut::new();
        bacnet_encoding::primitives::encode_app_enumerated(&mut list, 5);
        assert_property_error(
            write_property(
                &mut db,
                oid,
                PropertyIdentifier::ACCEPTED_MODES,
                list.to_vec(),
            ),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::ACCEPTED_MODES),
            [0x91, 0, 0x91, 1, 0x91, 2]
        );
    }
    assert_property_error(
        write_property(
            &mut db,
            zone,
            PropertyIdentifier::TRACKING_VALUE,
            enumerated(0),
        ),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(
        read_bytes(&db, zone, PropertyIdentifier::TRACKING_VALUE),
        [0x91, 2]
    );
}
