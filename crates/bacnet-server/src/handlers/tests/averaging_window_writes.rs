//! Averaging window rows over ReadProperty, WriteProperty and
//! WritePropertyMultiple (Clause 12.5, #1092). The window rows read and take
//! writes on the wire, each accepted write empties the sample window, and the
//! statistics of an empty window go out as positive infinity, NaN and
//! negative infinity.
use super::*;
use PropertyIdentifier as P;
use bacnet_objects::averaging::AveragingObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::error::RemoteError;

fn averaging_db(samples: &[f32]) -> (ObjectDatabase, ObjectIdentifier) {
    averaging_db_with(None, samples)
}

/// A database holding AVG-1, its Window_Samples written first when given,
/// then fed `samples`.
fn averaging_db_with(
    window_samples: Option<u64>,
    samples: &[f32],
) -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let mut object = AveragingObject::new(1, "AVG-1").unwrap();
    if let Some(window_samples) = window_samples {
        object
            .write_property(
                P::WINDOW_SAMPLES,
                None,
                PropertyValue::Unsigned(window_samples),
                None,
            )
            .unwrap();
    }
    for &sample in samples {
        object.add_sample(sample).unwrap();
    }
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    (db, oid)
}

fn encode_value(value: &PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, value).unwrap();
    bytes.to_vec()
}

fn write_wire(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: P,
    value: PropertyValue,
) -> Result<(), Error> {
    let request = WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: encode_value(&value),
        priority: None,
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes).unwrap();
    handle_write_property(db, &bytes).map(|_| ())
}

/// The application-tagged bytes a ReadProperty of `property` returns.
fn read_wire(db: &ObjectDatabase, oid: ObjectIdentifier, property: P) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut request);
    let mut ack = BytesMut::new();
    handle_read_property(db, &request, &mut ack).unwrap();
    ReadPropertyACK::decode(&ack).unwrap().property_value
}

/// `(minimum, maximum, average, attempted, valid)` as wire bytes.
fn statistics(db: &ObjectDatabase, oid: ObjectIdentifier) -> [Vec<u8>; 5] {
    [
        P::MINIMUM_VALUE,
        P::MAXIMUM_VALUE,
        P::AVERAGE_VALUE,
        P::ATTEMPTED_SAMPLES,
        P::VALID_SAMPLES,
    ]
    .map(|property| read_wire(db, oid, property))
}

fn real(value: f32) -> Vec<u8> {
    encode_value(&PropertyValue::Real(value))
}

fn unsigned(value: u64) -> Vec<u8> {
    encode_value(&PropertyValue::Unsigned(value))
}

/// An empty window on the wire: REAL tag 0x44 with the IEEE-754 single
/// patterns for +INF, -INF and the quiet NaN, then two zero counts.
fn empty() -> [Vec<u8>; 5] {
    [
        vec![0x44, 0x7F, 0x80, 0x00, 0x00],
        vec![0x44, 0xFF, 0x80, 0x00, 0x00],
        vec![0x44, 0x7F, 0xC0, 0x00, 0x00],
        vec![0x21, 0x00],
        vec![0x21, 0x00],
    ]
}

fn assert_property_error(result: Result<(), Error>, expected: ErrorCode, context: &str) {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => {
            assert_eq!(class, ErrorClass::PROPERTY, "{context}");
            assert_eq!(code, expected, "{context}");
        }
        other => panic!("{context}: expected PROPERTY/{expected:?}, got {other:?}"),
    }
}

#[test]
fn read_property_serves_the_window_rows_and_empty_window_statistics() {
    let (db, oid) = averaging_db(&[]);
    assert_eq!(statistics(&db, oid), empty());
    assert_eq!(read_wire(&db, oid, P::WINDOW_SAMPLES), unsigned(15));
    assert_eq!(read_wire(&db, oid, P::WINDOW_INTERVAL), unsigned(900));
}

#[test]
fn write_property_window_rows_reset_the_statistics() {
    let (mut db, oid) = averaging_db(&[4.0, 8.0]);
    let filled = [real(4.0), real(8.0), real(6.0), unsigned(2), unsigned(2)];
    assert_eq!(statistics(&db, oid), filled);

    write_wire(&mut db, oid, P::WINDOW_SAMPLES, PropertyValue::Unsigned(2)).unwrap();
    assert_eq!(statistics(&db, oid), empty());
    assert_eq!(read_wire(&db, oid, P::WINDOW_SAMPLES), unsigned(2));

    // With room for two samples, a third pushes the first out.
    let (mut db, oid) = averaging_db_with(Some(2), &[1.0, 5.0, 9.0]);
    assert_eq!(
        statistics(&db, oid),
        [real(5.0), real(9.0), real(7.0), unsigned(2), unsigned(2)]
    );
    write_wire(
        &mut db,
        oid,
        P::WINDOW_INTERVAL,
        PropertyValue::Unsigned(120),
    )
    .unwrap();
    assert_eq!(statistics(&db, oid), empty());
    assert_eq!(read_wire(&db, oid, P::WINDOW_INTERVAL), unsigned(120));
    assert_eq!(read_wire(&db, oid, P::WINDOW_SAMPLES), unsigned(2));

    let (mut db, oid) = averaging_db(&[3.0]);
    write_wire(
        &mut db,
        oid,
        P::ATTEMPTED_SAMPLES,
        PropertyValue::Unsigned(0),
    )
    .unwrap();
    assert_eq!(statistics(&db, oid), empty());
    // The window settings survive an Attempted_Samples reset.
    assert_eq!(read_wire(&db, oid, P::WINDOW_SAMPLES), unsigned(15));
    assert_eq!(read_wire(&db, oid, P::WINDOW_INTERVAL), unsigned(900));
}

#[test]
fn write_property_window_rows_refuse_bad_values_unchanged() {
    let (mut db, oid) = averaging_db(&[4.0, 8.0]);
    let before = statistics(&db, oid);
    for (property, value, code) in [
        (
            P::WINDOW_SAMPLES,
            PropertyValue::Unsigned(0),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::WINDOW_SAMPLES,
            PropertyValue::Unsigned(1_441),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::WINDOW_SAMPLES,
            PropertyValue::Real(4.0),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            P::WINDOW_INTERVAL,
            PropertyValue::Unsigned(0),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::WINDOW_INTERVAL,
            PropertyValue::Unsigned(1 << 32),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::ATTEMPTED_SAMPLES,
            PropertyValue::Unsigned(1),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::ATTEMPTED_SAMPLES,
            PropertyValue::Signed(0),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        // The statistics stay read-only.
        (
            P::AVERAGE_VALUE,
            PropertyValue::Real(1.0),
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
        (
            P::VALID_SAMPLES,
            PropertyValue::Unsigned(0),
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
    ] {
        let context = format!("{property:?} = {value:?}");
        assert_property_error(write_wire(&mut db, oid, property, value), code, &context);
        assert_eq!(statistics(&db, oid), before, "{context}");
        assert_eq!(read_wire(&db, oid, P::WINDOW_SAMPLES), unsigned(15));
        assert_eq!(read_wire(&db, oid, P::WINDOW_INTERVAL), unsigned(900));
    }
}

#[test]
fn write_property_multiple_resets_and_keeps_the_prefix_on_a_refusal() {
    let request = |oid: ObjectIdentifier, writes: Vec<(P, PropertyValue)>| {
        let mut bytes = BytesMut::new();
        WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: oid,
                list_of_properties: writes
                    .into_iter()
                    .map(|(property_identifier, value)| BACnetPropertyValue {
                        property_identifier,
                        property_array_index: None,
                        value: encode_value(&value),
                        priority: None,
                    })
                    .collect(),
            }],
        }
        .encode(&mut bytes)
        .unwrap();
        bytes
    };

    let (mut db, oid) = averaging_db(&[4.0, 8.0]);
    let accepted = request(
        oid,
        vec![
            (P::WINDOW_INTERVAL, PropertyValue::Unsigned(300)),
            (P::WINDOW_SAMPLES, PropertyValue::Unsigned(60)),
        ],
    );
    handle_write_property_multiple(&mut db, &accepted).unwrap();
    assert_eq!(statistics(&db, oid), empty());
    assert_eq!(read_wire(&db, oid, P::WINDOW_INTERVAL), unsigned(300));
    assert_eq!(read_wire(&db, oid, P::WINDOW_SAMPLES), unsigned(60));

    // The first write commits and resets; the second is refused.
    let (mut db, oid) = averaging_db(&[2.0]);
    let refused = request(
        oid,
        vec![
            (P::WINDOW_SAMPLES, PropertyValue::Unsigned(30)),
            (P::ATTEMPTED_SAMPLES, PropertyValue::Unsigned(7)),
        ],
    );
    assert!(handle_write_property_multiple(&mut db, &refused).is_err());
    assert_eq!(statistics(&db, oid), empty());
    assert_eq!(read_wire(&db, oid, P::WINDOW_SAMPLES), unsigned(30));
}
