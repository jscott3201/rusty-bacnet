//! Elevator Group Landing_Call_Control and Landing_Calls over the services
//! (#980): BACnetLandingCallStatus values (Clause 12.58, Table 12-76), not an
//! Enumerated and an Unsigned count.

use super::*;
use bacnet_objects::elevator::ElevatorGroupObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::constructed::{BACnetLandingCallStatus, LandingCallCommand};
use bacnet_types::enums::LiftCarDirection;
use bacnet_types::error::RemoteError;

const LCC: PropertyIdentifier = PropertyIdentifier::LANDING_CALL_CONTROL;
const LANDING_CALLS: PropertyIdentifier = PropertyIdentifier::LANDING_CALLS;

/// floor `[0]` 5, direction `[1]` UP.
const UP_FROM_5: &[u8] = &[0x09, 0x05, 0x19, 0x03];

fn group_db() -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    let oid = group.object_identifier();
    db.add(Box::new(group)).unwrap();
    (db, oid)
}

fn encode_value(value: PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &value).unwrap();
    buf.to_vec()
}

fn write(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    array_index: Option<u32>,
    property_value: &[u8],
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: LCC,
        property_array_index: array_index,
        property_value: property_value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

/// The raw propertyValue a ReadProperty ACK carries.
fn read_wire(db: &ObjectDatabase, oid: ObjectIdentifier, property: PropertyIdentifier) -> Vec<u8> {
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

/// Landing_Call_Control as read over the wire, decoded independently.
fn control(db: &ObjectDatabase, oid: ObjectIdentifier) -> BACnetLandingCallStatus {
    let bytes = read_wire(db, oid, LCC);
    let (status, end) =
        bacnet_encoding::constructed::decode_landing_call_status(&bytes, 0).unwrap();
    assert_eq!(end, bytes.len());
    status
}

fn assert_property_error(result: Result<(), Error>, expected: ErrorCode, context: &str) {
    match result.expect_err(context) {
        Error::Remote(RemoteError::Error { class, code }) => {
            assert_eq!(class, ErrorClass::PROPERTY, "{context}");
            assert_eq!(code, expected, "{context}: {expected:?}");
        }
        other => panic!("{context}: expected PROPERTY/{expected:?}, got {other:?}"),
    }
}

#[test]
fn landing_call_control_writes_over_wp_and_reads_back_over_rp() {
    let (mut db, oid) = group_db();
    assert_eq!(read_wire(&db, oid, LCC), [0x09, 0x00, 0x19, 0x00]);
    let cases: &[(&[u8], BACnetLandingCallStatus)] = &[
        (
            UP_FROM_5,
            BACnetLandingCallStatus {
                floor_number: 5,
                command: LandingCallCommand::Direction(LiftCarDirection::UP),
                floor_text: None,
            },
        ),
        (
            &[
                0x09, 0x0C, 0x29, 0x14, 0x3D, 0x06, 0x00, b'L', b'o', b'b', b'b', b'y',
            ],
            BACnetLandingCallStatus {
                floor_number: 12,
                command: LandingCallCommand::Destination(20),
                floor_text: Some("Lobby".into()),
            },
        ),
        (
            &[0x09, 0x00, 0x1A, 0x04, 0x00],
            BACnetLandingCallStatus {
                floor_number: 0,
                command: LandingCallCommand::Direction(LiftCarDirection::from_raw(1024)),
                floor_text: None,
            },
        ),
    ];
    for (bytes, expected) in cases {
        write(&mut db, oid, None, bytes).unwrap();
        assert_eq!(read_wire(&db, oid, LCC), *bytes);
        assert_eq!(control(&db, oid), *expected);
    }
}

#[test]
fn landing_call_control_wp_refusals_preserve_the_last_call() {
    let (mut db, oid) = group_db();
    write(&mut db, oid, None, UP_FROM_5).unwrap();
    let cases: Vec<(&str, Option<u32>, Vec<u8>, ErrorCode)> = vec![
        (
            "application Enumerated",
            None,
            encode_value(PropertyValue::Enumerated(1)),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            "application Unsigned",
            None,
            encode_value(PropertyValue::Unsigned(1)),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            "context floor then application direction",
            None,
            vec![0x09, 0x05, 0x91, 0x03],
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            "reserved direction 6",
            None,
            vec![0x09, 0x05, 0x19, 0x06],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            "direction 65536",
            None,
            vec![0x09, 0x05, 0x1B, 0x01, 0x00, 0x00],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        // Well-formed calls whose Unsigned members don't fit their types are
        // out of range, not malformed (Clause 15.9.1.3).
        (
            "floor 300, direction UP",
            None,
            vec![0x0A, 0x01, 0x2C, 0x19, 0x03],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            "destination 256",
            None,
            vec![0x09, 0x05, 0x2A, 0x01, 0x00],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            "direction 2^32",
            None,
            vec![0x09, 0x05, 0x1D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            "floor-number only",
            None,
            vec![0x09, 0x05],
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        (
            "oversized floor without a command",
            None,
            vec![0x0A, 0x01, 0x2C],
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        (
            "both command alternatives",
            None,
            vec![0x09, 0x05, 0x19, 0x03, 0x29, 0x14],
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        (
            "array index",
            Some(1),
            UP_FROM_5.to_vec(),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
    ];
    let before = read_wire(&db, oid, LCC);
    for (what, index, bytes, expected) in cases {
        assert_property_error(write(&mut db, oid, index, &bytes), expected, what);
        assert_eq!(read_wire(&db, oid, LCC), before, "{what}");
    }
    // The property isn't commandable, so a NULL succeeds and leaves the
    // call as it was (#1396).
    write(&mut db, oid, None, &encode_value(PropertyValue::Null)).unwrap();
    assert_eq!(read_wire(&db, oid, LCC), before);
}

#[test]
fn landing_call_control_wpm_keeps_the_committed_prefix() {
    let (mut db, oid) = group_db();
    let property = |value: &[u8]| BACnetPropertyValue {
        property_identifier: LCC,
        property_array_index: None,
        value: value.to_vec(),
        priority: None,
    };
    let mut request = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![property(UP_FROM_5), property(&[0x09, 0x05, 0x19, 0x06])],
        }],
    }
    .encode(&mut request)
    .unwrap();
    assert_property_error(
        handle_write_property_multiple(&mut db, &request).map(|_| ()),
        ErrorCode::VALUE_OUT_OF_RANGE,
        "second call has a reserved direction",
    );
    assert_eq!(read_wire(&db, oid, LCC), UP_FROM_5);
}

#[test]
fn landing_calls_read_as_a_list_and_refuse_writes() {
    let (mut db, oid) = group_db();
    assert!(read_wire(&db, oid, LANDING_CALLS).is_empty());

    let mut db_with_calls = ObjectDatabase::new();
    let mut group = ElevatorGroupObject::new(2, "EG-2").unwrap();
    group
        .set_landing_calls(vec![
            BACnetLandingCallStatus {
                floor_number: 2,
                command: LandingCallCommand::Direction(LiftCarDirection::DOWN),
                floor_text: None,
            },
            BACnetLandingCallStatus {
                floor_number: 9,
                command: LandingCallCommand::Destination(1),
                floor_text: Some("L".into()),
            },
        ])
        .unwrap();
    let with_calls = group.object_identifier();
    db_with_calls.add(Box::new(group)).unwrap();
    let bytes = read_wire(&db_with_calls, with_calls, LANDING_CALLS);
    assert_eq!(
        bytes,
        [
            0x09, 0x02, 0x19, 0x04, 0x09, 0x09, 0x29, 0x01, 0x3A, 0x00, 0x4C
        ]
    );
    assert_eq!(
        bacnet_encoding::constructed::decode_landing_call_status_list(&bytes)
            .unwrap()
            .len(),
        2
    );

    // Landing_Calls is status the application owns (Table 12-76 gives it no
    // write requirement), so a client write is refused.
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: LANDING_CALLS,
        property_array_index: None,
        property_value: UP_FROM_5.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    assert_property_error(
        handle_write_property(&mut db, &request).map(|_| ()),
        ErrorCode::WRITE_ACCESS_DENIED,
        "Landing_Calls write",
    );
}
