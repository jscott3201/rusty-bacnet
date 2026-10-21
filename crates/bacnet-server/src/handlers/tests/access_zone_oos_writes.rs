//! Occupancy_Count and Reliability writes on an Access Zone over
//! WriteProperty and WritePropertyMultiple: taken while Out_Of_Service is
//! TRUE, refused in service (Clauses 12.32.9, 12.32.10 and 12.32.11, Table
//! 12-37 footnote 1, #1247). Adjust_Value writes, taken either way but moving
//! the count only in service (Clause 12.32.13, footnote 5, #1284).

use super::*;
use bacnet_objects::access_control::AccessZoneObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::enums::Reliability;

const COUNT: PropertyIdentifier = PropertyIdentifier::OCCUPANCY_COUNT;
const RELIABILITY: PropertyIdentifier = PropertyIdentifier::RELIABILITY;

/// An in-service zone whose own count is 12.
fn zone_db() -> (ObjectDatabase, ObjectIdentifier) {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_occupancy_count(12);
    let oid = zone.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(zone)).unwrap();
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

/// Occupancy_Count, Reliability and Status_Flags as served.
fn served(db: &ObjectDatabase, oid: ObjectIdentifier) -> [Vec<u8>; 3] {
    [COUNT, RELIABILITY, PropertyIdentifier::STATUS_FLAGS].map(|p| read_bytes(db, oid, p))
}

/// The zone's own values; `out_of_service` sets that flag.
fn own(out_of_service: bool) -> [Vec<u8>; 3] {
    [
        vec![0x21, 12],
        vec![0x91, 0],
        vec![0x82, 0x04, if out_of_service { 0x10 } else { 0x00 }],
    ]
}

/// A simulated count of 40 and a simulated UNRELIABLE_OTHER, which sets
/// FAULT beside OUT_OF_SERVICE.
fn simulated() -> [Vec<u8>; 3] {
    [vec![0x21, 40], vec![0x91, 7], vec![0x82, 0x04, 0x50]]
}

fn simulation() -> [(PropertyIdentifier, Vec<u8>); 2] {
    [
        (COUNT, encode(&PropertyValue::Unsigned(40))),
        (
            RELIABILITY,
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
        matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected PROPERTY / {expected:?}, got {result:?}"
    );
}

#[test]
fn write_property_takes_zone_rows_only_out_of_service() {
    let (mut db, oid) = zone_db();
    for (property, value) in simulation() {
        assert_property_error(
            write_property(&mut db, oid, property, value),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_eq!(served(&db, oid), own(false));

    let (property, value) = out_of_service(true);
    write_property(&mut db, oid, property, value).unwrap();
    assert_eq!(served(&db, oid), own(true));
    for (property, value) in simulation() {
        write_property(&mut db, oid, property, value).unwrap();
    }
    assert_eq!(served(&db, oid), simulated());

    // The return to service serves the zone's values again.
    let (property, value) = out_of_service(false);
    write_property(&mut db, oid, property, value).unwrap();
    assert_eq!(served(&db, oid), own(false));
    for (property, value) in simulation() {
        assert_property_error(
            write_property(&mut db, oid, property, value),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
}

#[test]
fn write_property_multiple_takes_zone_rows_only_out_of_service() {
    let (mut db, oid) = zone_db();
    assert_property_error(
        write_property_multiple(&mut db, oid, &simulation()),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(served(&db, oid), own(false));

    // Out_Of_Service first, then both simulated values, in one request.
    let mut writes = vec![out_of_service(true)];
    writes.extend(simulation());
    write_property_multiple(&mut db, oid, &writes).unwrap();
    assert_eq!(served(&db, oid), simulated());

    // A simulated count and the return to service in one request: the zone's
    // values come back and the simulation is dropped.
    write_property_multiple(
        &mut db,
        oid,
        &[
            (COUNT, encode(&PropertyValue::Unsigned(3))),
            out_of_service(false),
        ],
    )
    .unwrap();
    assert_eq!(served(&db, oid), own(false));
}

#[test]
fn zone_row_writes_outside_their_datatypes_are_refused_unchanged() {
    let (mut db, oid) = zone_db();
    let (property, value) = out_of_service(true);
    write_property(&mut db, oid, property, value).unwrap();
    for (property, value, code) in [
        (
            COUNT,
            encode(&PropertyValue::Signed(5)),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            COUNT,
            encode(&PropertyValue::Real(5.0)),
            ErrorCode::INVALID_DATA_TYPE,
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
        let before = served(&db, oid);
        assert_property_error(write_property(&mut db, oid, property, value.clone()), code);
        assert_eq!(served(&db, oid), before, "{property:?} {value:02X?}");
        // WPM stops at the refused value after committing the valid one
        // before it.
        assert_property_error(
            write_property_multiple(
                &mut db,
                oid,
                &[
                    (COUNT, encode(&PropertyValue::Unsigned(99))),
                    (property, value.clone()),
                ],
            ),
            code,
        );
        let [count, reliability, _] = served(&db, oid);
        assert_eq!(count, [0x21, 99], "{property:?} {value:02X?}");
        assert_eq!(reliability, before[1], "{property:?} {value:02X?}");
        let (property, value) = (COUNT, encode(&PropertyValue::Unsigned(12)));
        write_property(&mut db, oid, property, value).unwrap();
    }
    // A proprietary Reliability and a count past one octet go through.
    write_property(
        &mut db,
        oid,
        RELIABILITY,
        encode(&PropertyValue::Enumerated(64)),
    )
    .unwrap();
    write_property(&mut db, oid, COUNT, encode(&PropertyValue::Unsigned(1_000))).unwrap();
    assert_eq!(read_bytes(&db, oid, RELIABILITY), [0x91, 64]);
    assert_eq!(read_bytes(&db, oid, COUNT), [0x22, 0x03, 0xE8]);
}

#[test]
fn adjust_value_writes_move_the_count_in_service_only() {
    const ADJUST: PropertyIdentifier = PropertyIdentifier::ADJUST_VALUE;
    const STATE: PropertyIdentifier = PropertyIdentifier::OCCUPANCY_STATE;
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_occupancy_count(12);
    zone.set_occupancy_limits(0, 10).unwrap();
    let oid = zone.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(zone)).unwrap();
    let counting = |db: &ObjectDatabase| [COUNT, STATE, ADJUST].map(|p| read_bytes(db, oid, p));
    // Twelve is above the upper limit of ten (#1284).
    assert_eq!(
        counting(&db),
        [vec![0x21, 12], vec![0x91, 4], vec![0x31, 0]]
    );

    // In service a write moves the count: WriteProperty by -3, then
    // WritePropertyMultiple by -1 to sit at the limit.
    write_property(&mut db, oid, ADJUST, encode(&PropertyValue::Signed(-3))).unwrap();
    assert_eq!(
        counting(&db),
        [vec![0x21, 9], vec![0x91, 0], vec![0x31, 0xFD]]
    );
    write_property_multiple(&mut db, oid, &[(ADJUST, encode(&PropertyValue::Signed(1)))]).unwrap();
    assert_eq!(
        counting(&db),
        [vec![0x21, 10], vec![0x91, 3], vec![0x31, 1]]
    );

    // Out of service the value is kept, but the count stays as simulated.
    let (property, value) = out_of_service(true);
    write_property(&mut db, oid, property, value).unwrap();
    write_property(&mut db, oid, ADJUST, encode(&PropertyValue::Signed(5))).unwrap();
    assert_eq!(
        counting(&db),
        [vec![0x21, 10], vec![0x91, 3], vec![0x31, 5]]
    );
    assert_property_error(
        write_property(&mut db, oid, ADJUST, encode(&PropertyValue::Unsigned(5))),
        ErrorCode::INVALID_DATA_TYPE,
    );
    // Occupancy_State and the other counting rows stay read-only.
    for property in [
        STATE,
        PropertyIdentifier::OCCUPANCY_COUNT_ENABLE,
        PropertyIdentifier::OCCUPANCY_UPPER_LIMIT,
    ] {
        let value = read_bytes(&db, oid, property);
        assert_property_error(
            write_property(&mut db, oid, property, value),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
}

/// An AddListElement or RemoveListElement request on Alarm_Values.
fn list_request(oid: ObjectIdentifier, elements: &[u8]) -> Vec<u8> {
    let mut request = BytesMut::new();
    bacnet_services::list_manipulation::ListElementRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::ALARM_VALUES,
        property_array_index: None,
        list_of_elements: elements.to_vec(),
    }
    .encode(&mut request)
    .unwrap();
    request.to_vec()
}

#[test]
fn alarm_values_take_occupancy_states_over_the_wire() {
    const ALARM: PropertyIdentifier = PropertyIdentifier::ALARM_VALUES;
    let (mut db, oid) = zone_db();
    // ABOVE_UPPER_LIMIT alone, then with BELOW_LOWER_LIMIT (#1305).
    write_property(&mut db, oid, ALARM, vec![0x91, 4]).unwrap();
    assert_eq!(read_bytes(&db, oid, ALARM), [0x91, 4]);
    write_property_multiple(&mut db, oid, &[(ALARM, vec![0x91, 4, 0x91, 1])]).unwrap();
    assert_eq!(read_bytes(&db, oid, ALARM), [0x91, 4, 0x91, 1]);
    // 7 is reserved for ASHRAE, and an Unsigned is the wrong datatype; the
    // refusal names the element and changes nothing.
    for (value, code, element) in [
        (vec![0x91, 4, 0x91, 7], ErrorCode::VALUE_OUT_OF_RANGE, 2),
        (vec![0x21, 4], ErrorCode::INVALID_DATA_TYPE, 1),
    ] {
        assert_eq!(
            list_refusal(write_property(&mut db, oid, ALARM, value)),
            (ErrorClass::PROPERTY, code, element)
        );
        assert_eq!(read_bytes(&db, oid, ALARM), [0x91, 4, 0x91, 1]);
    }

    // The list services edit it too.
    handle_add_list_element(&mut db, &list_request(oid, &[0x91, 2])).unwrap();
    assert_eq!(read_bytes(&db, oid, ALARM), [0x91, 4, 0x91, 1, 0x91, 2]);
    assert_eq!(
        list_refusal(handle_add_list_element(
            &mut db,
            &list_request(oid, &[0x91, 5, 0x91, 9])
        )),
        (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 2)
    );
    handle_remove_list_element(&mut db, &list_request(oid, &[0x91, 4, 0x91, 1])).unwrap();
    assert_eq!(read_bytes(&db, oid, ALARM), [0x91, 2]);
}

#[test]
fn alarm_values_refuse_normal_over_the_wire() {
    const ALARM: PropertyIdentifier = PropertyIdentifier::ALARM_VALUES;
    const HELD: [u8; 6] = [0x91, 4, 0x91, 5, 0x91, 6];
    let (mut db, oid) = zone_db();
    // ABOVE_UPPER_LIMIT, DISABLED and NOT_SUPPORTED may all alarm (#1401).
    write_property(&mut db, oid, ALARM, HELD.to_vec()).unwrap();
    assert_eq!(read_bytes(&db, oid, ALARM), HELD);
    // NORMAL (0) may not, over any service: the refusal names the element
    // and leaves the list alone.
    let refused = [
        (
            "WriteProperty",
            write_property(&mut db, oid, ALARM, vec![0x91, 3, 0x91, 0]),
            2,
        ),
        (
            "WritePropertyMultiple",
            write_property_multiple(&mut db, oid, &[(ALARM, vec![0x91, 0])]),
            1,
        ),
        (
            "AddListElement",
            handle_add_list_element(&mut db, &list_request(oid, &[0x91, 2, 0x91, 0])),
            2,
        ),
    ];
    for (service, result, element) in refused {
        assert_eq!(
            list_refusal(result),
            (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, element),
            "{service}"
        );
    }
    assert_eq!(read_bytes(&db, oid, ALARM), HELD);
}
