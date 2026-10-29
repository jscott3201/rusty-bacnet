use super::*;
use bacnet_objects::accumulator::PulseConverterObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::RemoteError;

fn pulse_converter_db() -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let object = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    (db, oid)
}

fn encode_value(value: PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, &value).unwrap();
    bytes.to_vec()
}

fn write_wire(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<ObjectIdentifier, Error> {
    let request = WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: encode_value(value),
        priority: None,
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes).unwrap();
    handle_write_property(db, &bytes)
}

fn write_multiple_wire(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    writes: Vec<(PropertyIdentifier, PropertyValue)>,
) -> Result<Vec<ObjectIdentifier>, Error> {
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: writes
                .into_iter()
                .map(|(property_identifier, value)| BACnetPropertyValue {
                    property_identifier,
                    property_array_index: None,
                    value: encode_value(value),
                    priority: None,
                })
                .collect(),
        }],
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes).unwrap();
    handle_write_property_multiple(db, &bytes)
}

fn assert_write_access_denied<T>(result: Result<T, Error>) {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(code, ErrorCode::WRITE_ACCESS_DENIED);
        }
        Err(other) => panic!("expected PROPERTY/WRITE_ACCESS_DENIED, got {other:?}"),
        Ok(_) => panic!("expected PROPERTY/WRITE_ACCESS_DENIED, got success"),
    }
}

fn assert_state(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    present_value: f32,
    out_of_service: bool,
) {
    let object = db.get(&oid).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(present_value)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap(),
        PropertyValue::Boolean(out_of_service)
    );
}

#[test]
fn write_property_present_value_requires_out_of_service() {
    let (mut db, oid) = pulse_converter_db();

    assert_write_access_denied(write_wire(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(12.5),
    ));
    assert_state(&db, oid, 0.0, false);

    write_wire(
        &mut db,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    write_wire(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(12.5),
    )
    .unwrap();
    assert_state(&db, oid, 12.5, true);
}

#[test]
fn write_property_multiple_present_value_before_oos_fails_without_mutation() {
    let (mut db, oid) = pulse_converter_db();

    assert_write_access_denied(write_multiple_wire(
        &mut db,
        oid,
        vec![
            (PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(12.5)),
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            ),
        ],
    ));
    assert_state(&db, oid, 0.0, false);
}

#[test]
fn write_property_multiple_oos_before_present_value_succeeds() {
    let (mut db, oid) = pulse_converter_db();

    write_multiple_wire(
        &mut db,
        oid,
        vec![
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            ),
            (PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(12.5)),
        ],
    )
    .unwrap();
    assert_state(&db, oid, 12.5, true);
}

#[test]
fn write_property_multiple_later_failure_keeps_present_value_and_oos_prefix() {
    let (mut db, oid) = pulse_converter_db();

    assert_write_access_denied(write_multiple_wire(
        &mut db,
        oid,
        vec![
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            ),
            (PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(12.5)),
            (
                PropertyIdentifier::OBJECT_TYPE,
                PropertyValue::Enumerated(ObjectType::PULSE_CONVERTER.to_raw()),
            ),
        ],
    ));
    assert_state(&db, oid, 12.5, true);
}

// --- #1092: Count and the rows an Adjust_Value write updates ---

/// A Pulse Converter holding `pulses`, in a database with a Device clock.
fn clocked_pulse_converter_db(pulses: u64) -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = crate::server::clocked_test_database();
    let mut object =
        PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    object.add_pulses(pulses).unwrap();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    (db, oid)
}

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

/// Count, Count_Before_Change, Adjust_Value and Count_Change_Time as read
/// over the wire.
fn count_rows(db: &ObjectDatabase, oid: ObjectIdentifier) -> [Vec<u8>; 4] {
    [
        read_wire(db, oid, PropertyIdentifier::COUNT),
        read_wire(db, oid, PropertyIdentifier::COUNT_BEFORE_CHANGE),
        read_wire(db, oid, PropertyIdentifier::ADJUST_VALUE),
        read_wire(db, oid, PropertyIdentifier::COUNT_CHANGE_TIME),
    ]
}

/// An all-unspecified BACnetDateTime: Date 0xA4 then Time 0xB4, every field X'FF'.
const UNSPECIFIED_DATETIME: [u8; 10] = [0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xB4, 0xFF, 0xFF, 0xFF, 0xFF];

#[test]
fn write_property_adjust_value_adjusts_count_and_stamps_the_change() {
    let (mut db, oid) = clocked_pulse_converter_db(55);
    write_wire(
        &mut db,
        oid,
        PropertyIdentifier::SCALE_FACTOR,
        PropertyValue::Real(12.5),
    )
    .unwrap();
    // 55 pulses at 12.5 each: 687.5 is 0x442BE000.
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::PRESENT_VALUE),
        [0x44, 0x44, 0x2B, 0xE0, 0x00]
    );
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::COUNT_CHANGE_TIME),
        UNSPECIFIED_DATETIME
    );

    // 30 / 12.5 = 2.4, so Count drops by 2.
    write_wire(
        &mut db,
        oid,
        PropertyIdentifier::ADJUST_VALUE,
        PropertyValue::Real(30.0),
    )
    .unwrap();
    let [count, before, adjust, changed] = count_rows(&db, oid);
    assert_eq!(count, [0x21, 53]);
    assert_eq!(before, [0x21, 55]);
    assert_eq!(adjust, [0x44, 0x41, 0xF0, 0x00, 0x00]);
    // The Device clock stamped a specified date and time.
    assert_eq!(changed.len(), 10);
    assert_eq!((changed[0], changed[5]), (0xA4, 0xB4));
    assert_ne!(changed[1], 0xFF, "year is specified");
    assert_ne!(changed[6], 0xFF, "hour is specified");
    // 53 x 12.5 = 662.5, 0x4425A000.
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::PRESENT_VALUE),
        [0x44, 0x44, 0x25, 0xA0, 0x00]
    );
}

#[test]
fn write_property_adjust_value_past_zero_is_value_out_of_range_and_atomic() {
    let (mut db, oid) = clocked_pulse_converter_db(2);
    let before = count_rows(&db, oid);
    assert_eq!(before[3], UNSPECIFIED_DATETIME);
    for adjust in [3.0, f32::MAX] {
        match write_wire(
            &mut db,
            oid,
            PropertyIdentifier::ADJUST_VALUE,
            PropertyValue::Real(adjust),
        ) {
            Err(Error::Remote(RemoteError::Error { class, code })) => {
                assert_eq!(class, ErrorClass::PROPERTY);
                assert_eq!(code, ErrorCode::VALUE_OUT_OF_RANGE);
            }
            other => panic!("{adjust}: expected VALUE_OUT_OF_RANGE, got {other:?}"),
        }
        assert_eq!(count_rows(&db, oid), before, "{adjust}");
    }
}

#[test]
fn write_property_count_rows_are_write_access_denied() {
    let (mut db, oid) = clocked_pulse_converter_db(4);
    for (property, value) in [
        (PropertyIdentifier::COUNT, PropertyValue::Unsigned(0)),
        (
            PropertyIdentifier::COUNT_BEFORE_CHANGE,
            PropertyValue::Unsigned(0),
        ),
        (PropertyIdentifier::COV_PERIOD, PropertyValue::Unsigned(60)),
    ] {
        assert_write_access_denied(write_wire(&mut db, oid, property, value));
    }
    assert_eq!(read_wire(&db, oid, PropertyIdentifier::COUNT), [0x21, 4]);
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::COV_PERIOD),
        [0x21, 0]
    );
}
