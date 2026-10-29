//! Present_Value and Reliability writes on a Credential Data Input over
//! WriteProperty and WritePropertyMultiple: taken while Out_Of_Service is
//! TRUE, refused in service (Clauses 12.36.4, 12.36.7 and 12.36.8, Table 12-43
//! footnote 1, #1168).

use bacnet_types::error::RemoteError;
use std::sync::Arc;

use super::*;
use bacnet_objects::access_control::CredentialDataInputObject;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::constructed::{BACnetAuthenticationFactor, BACnetAuthenticationFactorFormat};
use bacnet_types::enums::{AuthenticationFactorType, Reliability};
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};

/// 2026-10-02 (a Friday).
const DATE: Date = Date {
    year: 126,
    month: 10,
    day: 2,
    day_of_week: 5,
};

/// A Device clock that always reads 11:30 on `DATE`.
struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: DATE,
            local_time: Time {
                hour: 11,
                minute: 30,
                second: 0,
                hundredths: 0,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

/// An in-service reader of Wiegand 26 cards (class 0) and of vendor 260's
/// format 7 (class 3) whose last read was a Wiegand 26 card at 09:30.
fn reader_db() -> (ObjectDatabase, ObjectIdentifier) {
    let mut reader = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    reader
        .set_supported_formats([
            (
                BACnetAuthenticationFactorFormat::standard(AuthenticationFactorType::WIEGAND26),
                0,
            ),
            (BACnetAuthenticationFactorFormat::custom(260, 7), 3),
        ])
        .unwrap();
    reader
        .set_present_value(
            BACnetAuthenticationFactor {
                format_type: AuthenticationFactorType::WIEGAND26,
                format_class: 0,
                value: vec![0x12, 0x34, 0x56],
            },
            BACnetTimeStamp::DateTime {
                date: DATE,
                time: Time {
                    hour: 9,
                    minute: 30,
                    second: 0,
                    hundredths: 0,
                },
            },
        )
        .unwrap();
    let oid = reader.object_identifier();
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(Arc::new(FixedClock)));
    db.add(Box::new(reader)).unwrap();
    (db, oid)
}

fn encode(value: &PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut buf, value).unwrap();
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

fn write_property_multiple(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    writes: &[(PropertyIdentifier, Vec<u8>)],
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
                    value: value.clone(),
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

const ROWS: [PropertyIdentifier; 4] = [
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::UPDATE_TIME,
    PropertyIdentifier::RELIABILITY,
    PropertyIdentifier::STATUS_FLAGS,
];

/// Present_Value, Update_Time, Reliability and Status_Flags as served.
fn served(db: &ObjectDatabase, oid: ObjectIdentifier) -> [Vec<u8>; 4] {
    ROWS.map(|property| read_bytes(db, oid, property))
}

/// format type [0] WIEGAND26, format class [1] 0, value [2] 12 34 56.
const CARD: [u8; 8] = [0x09, 0x08, 0x19, 0x00, 0x2B, 0x12, 0x34, 0x56];
/// format type [0] CUSTOM, format class [1] 3, value [2] AB.
const CUSTOM: [u8; 6] = [0x09, 0x02, 0x19, 0x03, 0x29, 0xAB];

/// The datetime choice at `hour`:30 on `DATE`.
fn stamp(hour: u8) -> Vec<u8> {
    vec![0x2E, 0xA4, 126, 10, 2, 5, 0xB4, hour, 30, 0, 0, 0x2F]
}

/// The reader's own values; `out_of_service` sets that flag.
fn device(out_of_service: bool) -> [Vec<u8>; 4] {
    [
        CARD.to_vec(),
        stamp(9),
        vec![0x91, 0],
        vec![0x82, 0x04, if out_of_service { 0x10 } else { 0x00 }],
    ]
}

/// A simulated vendor 260 read stamped at 11:30 and a simulated
/// UNRELIABLE_OTHER, which sets FAULT beside OUT_OF_SERVICE.
fn simulated() -> [Vec<u8>; 4] {
    [
        CUSTOM.to_vec(),
        stamp(11),
        vec![0x91, 7],
        vec![0x82, 0x04, 0x50],
    ]
}

fn simulation() -> [(PropertyIdentifier, Vec<u8>); 2] {
    [
        (PropertyIdentifier::PRESENT_VALUE, CUSTOM.to_vec()),
        (
            PropertyIdentifier::RELIABILITY,
            encode(&PropertyValue::Enumerated(
                Reliability::UNRELIABLE_OTHER.to_raw(),
            )),
        ),
    ]
}

fn out_of_service(value: bool) -> (PropertyIdentifier, Vec<u8>) {
    (
        PropertyIdentifier::OUT_OF_SERVICE,
        encode(&PropertyValue::Boolean(value)),
    )
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
fn write_property_takes_reader_rows_only_out_of_service() {
    let (mut db, oid) = reader_db();
    for (property, value) in simulation() {
        assert_property_error(
            write_property(&mut db, oid, property, value),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_eq!(served(&db, oid), device(false));

    let (property, value) = out_of_service(true);
    write_property(&mut db, oid, property, value).unwrap();
    assert_eq!(served(&db, oid), device(true));
    for (property, value) in simulation() {
        write_property(&mut db, oid, property, value).unwrap();
    }
    assert_eq!(served(&db, oid), simulated());

    // The return to service serves the reader's values again.
    let (property, value) = out_of_service(false);
    write_property(&mut db, oid, property, value).unwrap();
    assert_eq!(served(&db, oid), device(false));
    for (property, value) in simulation() {
        assert_property_error(
            write_property(&mut db, oid, property, value),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
}

#[test]
fn write_property_multiple_takes_reader_rows_only_out_of_service() {
    let (mut db, oid) = reader_db();
    assert_property_error(
        write_property_multiple(&mut db, oid, &simulation()),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(served(&db, oid), device(false));

    // Out_Of_Service first, then both simulated values, in one request.
    let mut writes = vec![out_of_service(true)];
    writes.extend(simulation());
    write_property_multiple(&mut db, oid, &writes).unwrap();
    assert_eq!(served(&db, oid), simulated());

    // A simulated value and the return to service in one request: the
    // reader's values come back and the simulation is dropped.
    write_property_multiple(
        &mut db,
        oid,
        &[
            (PropertyIdentifier::PRESENT_VALUE, CARD.to_vec()),
            out_of_service(false),
        ],
    )
    .unwrap();
    assert_eq!(served(&db, oid), device(false));
}

#[test]
fn reader_row_writes_outside_their_datatypes_are_refused_unchanged() {
    const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
    const RELIABILITY: PropertyIdentifier = PropertyIdentifier::RELIABILITY;
    let (mut db, oid) = reader_db();
    let (property, value) = out_of_service(true);
    write_property(&mut db, oid, property, value).unwrap();
    for (property, value, code) in [
        // Application-tagged values, and a factor missing its value.
        (
            PV,
            encode(&PropertyValue::Enumerated(8)),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            PV,
            encode(&PropertyValue::OctetString(vec![0x12])),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (PV, CARD[..4].to_vec(), ErrorCode::INVALID_DATA_TYPE),
        // A format type past the closed production.
        (
            PV,
            vec![0x09, 0x19, 0x19, 0x00, 0x28],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        // Wiegand 37, which the reader doesn't declare, and Wiegand 26 with
        // another format class.
        (
            PV,
            vec![0x09, 0x09, 0x19, 0x00, 0x28],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            PV,
            vec![0x09, 0x08, 0x19, 0x03, 0x28],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        // 11 is reserved for ASHRAE, 65536 past the datatype.
        (
            RELIABILITY,
            encode(&PropertyValue::Enumerated(11)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            RELIABILITY,
            encode(&PropertyValue::Enumerated(65_536)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            RELIABILITY,
            encode(&PropertyValue::Unsigned(7)),
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        write_property(&mut db, oid, PV, CARD.to_vec()).unwrap();
        let before = served(&db, oid);
        assert_property_error(write_property(&mut db, oid, property, value.clone()), code);
        assert_eq!(served(&db, oid), before, "{property:?} {value:02X?}");
        // WPM stops at the refused value after committing the valid one
        // before it.
        assert_property_error(
            write_property_multiple(
                &mut db,
                oid,
                &[(PV, CUSTOM.to_vec()), (property, value.clone())],
            ),
            code,
        );
        let [present_value, _, reliability, _] = served(&db, oid);
        assert_eq!(present_value, CUSTOM, "{property:?} {value:02X?}");
        assert_eq!(reliability, before[2], "{property:?} {value:02X?}");
    }
    // A proprietary Reliability goes through and reads back.
    write_property(
        &mut db,
        oid,
        RELIABILITY,
        encode(&PropertyValue::Enumerated(64)),
    )
    .unwrap();
    assert_eq!(read_bytes(&db, oid, RELIABILITY), [0x91, 64]);
}
