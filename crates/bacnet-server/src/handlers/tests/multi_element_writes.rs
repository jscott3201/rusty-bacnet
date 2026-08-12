//! Wire-level integration for #182: WriteProperty and WritePropertyMultiple
//! consume the WHOLE `propertyValue` payload. Multi-element application-tagged
//! values land at the object arm as `PropertyValue::List`; a partial element
//! at the tail is PROPERTY / INVALID_DATA_ENCODING with the stored value
//! untouched, and a well-formed trailing element is refused by a scalar arm as
//! the wrong shape — never silently dropped between decoder and arm.

use super::*;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_objects::traits::BACnetObject;

fn encode_value(value: PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &value).unwrap();
    buf.to_vec()
}

fn write_raw(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    property_value: Vec<u8>,
) -> Result<(), Error> {
    let request = WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: array_index,
        property_value,
        priority: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    handle_write_property(db, &buf).map(|_| ())
}

/// Read a property over the wire and loop-decode the flattened result the
/// same way the write path decodes (single element → scalar, else `List`).
fn read_prop(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_read_property(db, &buf, &mut ack_buf).unwrap();
    let raw = ReadPropertyACK::decode(&ack_buf).unwrap().property_value;
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < raw.len() {
        let (value, new_offset) =
            bacnet_encoding::primitives::decode_application_value(&raw, offset).unwrap();
        values.push(value);
        offset = new_offset;
    }
    match values.len() {
        1 => values.pop().unwrap(),
        _ => PropertyValue::List(values),
    }
}

/// A refused write carries exactly PROPERTY / `expected_code` and leaves
/// `property` reading back as `expected`.
fn assert_refused(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    raw_value: Vec<u8>,
    expected_code: ErrorCode,
    expected: PropertyValue,
    context: &str,
) {
    match write_raw(db, oid, property, None, raw_value).expect_err(context) {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                ErrorClass::PROPERTY.to_raw() as u32,
                "{context}: wrong error class"
            );
            assert_eq!(
                code,
                expected_code.to_raw() as u32,
                "{context}: wrong error code"
            );
        }
        other => panic!("{context}: expected PROPERTY/{expected_code:?}, got {other:?}"),
    }
    assert_eq!(
        read_prop(db, oid, property),
        expected,
        "{context}: refused write must leave the property unchanged"
    );
}

// ---------------------------------------------------------------------------
// Full-consumption matrix: no trailing byte may be silently dropped (#182
// guardrail). A complete-TLV trailing element that does not decode (here an
// unassigned application tag 13) fails the decode loop as PROPERTY /
// INVALID_DATA_ENCODING; a whole extra decodable element reaches the arm and
// fails its shape check as INVALID_DATA_TYPE; a TLV-truncated tail never
// survives the service request's own framing walk. An empty payload is
// refused outright.
// ---------------------------------------------------------------------------

#[test]
fn scalar_write_with_undecodable_trailing_element_is_refused_and_preserves() {
    let mut db = ObjectDatabase::new();
    let lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
    let oid = lo.object_identifier();
    db.add(Box::new(lo)).unwrap();

    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::SETPOINT,
        None,
        encode_value(PropertyValue::Real(72.0)),
    )
    .unwrap();

    // A valid Real followed by a well-formed TLV using unassigned application
    // tag 13: the request framing walks it, but there is no application-13
    // decoder, so the write-value decode refuses it.
    let mut bytes = encode_value(PropertyValue::Real(50.0));
    bytes.extend_from_slice(&[0xD1, 0x00]);
    assert_refused(
        &mut db,
        oid,
        PropertyIdentifier::SETPOINT,
        bytes,
        ErrorCode::INVALID_DATA_ENCODING,
        PropertyValue::Real(72.0),
        "scalar + undecodable trailing element",
    );
}

#[test]
fn list_write_with_undecodable_trailing_element_is_refused_and_preserves() {
    let mut db = make_db_with_msi();
    let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();

    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::ALARM_VALUES,
        None,
        vec![0x21, 2, 0x21, 3],
    )
    .unwrap();
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::ALARM_VALUES),
        PropertyValue::List(vec![PropertyValue::Unsigned(2), PropertyValue::Unsigned(3)])
    );

    // Whole-list write with an application-13 tail: refused by the decode
    // loop, stored list untouched.
    assert_refused(
        &mut db,
        oid,
        PropertyIdentifier::ALARM_VALUES,
        vec![0x21, 4, 0x21, 5, 0xD1, 0x00],
        ErrorCode::INVALID_DATA_ENCODING,
        PropertyValue::List(vec![PropertyValue::Unsigned(2), PropertyValue::Unsigned(3)]),
        "list + undecodable trailing element",
    );
}

#[test]
fn list_write_with_tlv_truncated_tail_is_refused_before_the_value_decode() {
    let mut db = make_db_with_msi();
    let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();

    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::ALARM_VALUES,
        None,
        vec![0x21, 2, 0x21, 3],
    )
    .unwrap();

    // The second element's tag promises a content octet that never arrives.
    // The WriteProperty-Request [3] framing walk itself cannot skip such a
    // value (it would have to eat the closing tag), so the request decode
    // refuses before `decode_write_property_value` runs — an error either
    // way, and nothing is stored.
    let err = write_raw(
        &mut db,
        oid,
        PropertyIdentifier::ALARM_VALUES,
        None,
        vec![0x21, 4, 0x21, 5, 0x21],
    )
    .unwrap_err();
    assert!(
        !matches!(err, Error::Protocol { .. }),
        "a framing-level truncation is not an object-level protocol error: {err:?}"
    );
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::ALARM_VALUES),
        PropertyValue::List(vec![PropertyValue::Unsigned(2), PropertyValue::Unsigned(3)]),
        "truncated write must leave the list unchanged"
    );
}

#[test]
fn well_formed_trailing_element_reaches_scalar_arm_as_list_and_is_refused() {
    let mut db = ObjectDatabase::new();
    let lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
    let oid = lo.object_identifier();
    db.add(Box::new(lo)).unwrap();

    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::SETPOINT,
        None,
        encode_value(PropertyValue::Real(72.0)),
    )
    .unwrap();

    // Two complete Reals: the decode succeeds (a two-element List), and the
    // scalar SETPOINT arm refuses the wrong shape — the previous decoder
    // silently dropped the second element and stored the first.
    let mut bytes = encode_value(PropertyValue::Real(50.0));
    bytes.extend_from_slice(&encode_value(PropertyValue::Real(99.0)));
    assert_refused(
        &mut db,
        oid,
        PropertyIdentifier::SETPOINT,
        bytes,
        ErrorCode::INVALID_DATA_TYPE,
        PropertyValue::Real(72.0),
        "scalar + well-formed second element",
    );
}

#[test]
fn empty_property_value_is_refused() {
    let mut db = make_db_with_msi();
    let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();

    assert_refused(
        &mut db,
        oid,
        PropertyIdentifier::ALARM_VALUES,
        Vec::new(),
        ErrorCode::INVALID_DATA_ENCODING,
        PropertyValue::List(vec![]),
        "empty propertyValue",
    );
}

// ---------------------------------------------------------------------------
// DateTime-paired value properties (#182): a BACnetDateTime writes as
// application-tagged Date + Time, which the decode loop delivers to the
// value-object arms as `List([Date, Time])` (Clause 12.x value types,
// tranche-L1 exclusion lifted).
// ---------------------------------------------------------------------------

use bacnet_objects::value_types::{DateTimePatternValueObject, DateTimeValueObject};
use bacnet_types::primitives::{Date, Time};

const TEST_DATE: Date = Date {
    year: 0x7E, // 2026
    month: 8,
    day: 12,
    day_of_week: 3,
};
const TEST_TIME: Time = Time {
    hour: 9,
    minute: 41,
    second: 30,
    hundredths: 0,
};

fn datetime_pv() -> PropertyValue {
    PropertyValue::List(vec![
        PropertyValue::Date(TEST_DATE),
        PropertyValue::Time(TEST_TIME),
    ])
}

#[test]
fn datetime_value_present_value_and_priority_array_over_the_wire() {
    let mut db = ObjectDatabase::new();
    let obj = DateTimeValueObject::new(1, "DTV-1").unwrap();
    let oid = obj.object_identifier();
    db.add(Box::new(obj)).unwrap();

    // Present_Value write: the two application-tagged members.
    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        encode_value(datetime_pv()),
    )
    .unwrap();
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::PRESENT_VALUE),
        datetime_pv()
    );

    // A priority-array ENTRY write of a different datetime: indexed write to
    // slot 2 wins over the earlier priority-16 command.
    let later = PropertyValue::List(vec![
        PropertyValue::Date(TEST_DATE),
        PropertyValue::Time(Time {
            hour: 12,
            ..TEST_TIME
        }),
    ]);
    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(2),
        encode_value(later.clone()),
    )
    .unwrap();
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::PRESENT_VALUE),
        later
    );
}

#[test]
fn datetime_relinquish_default_over_the_wire_recaptures_present_value() {
    let mut db = ObjectDatabase::new();
    let obj = DateTimeValueObject::new(1, "DTV-1").unwrap();
    let oid = obj.object_identifier();
    db.add(Box::new(obj)).unwrap();

    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::RELINQUISH_DEFAULT,
        None,
        encode_value(datetime_pv()),
    )
    .unwrap();
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::RELINQUISH_DEFAULT),
        datetime_pv()
    );
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::PRESENT_VALUE),
        datetime_pv(),
        "with an empty priority array, PV resolves to the written default"
    );

    // The pattern-typed sibling (DATETIMEPATTERN_VALUE) carries the same arm.
    let pat = DateTimePatternValueObject::new(2, "DTPV-2").unwrap();
    let pat_oid = pat.object_identifier();
    db.add(Box::new(pat)).unwrap();
    write_raw(
        &mut db,
        pat_oid,
        PropertyIdentifier::RELINQUISH_DEFAULT,
        None,
        encode_value(datetime_pv()),
    )
    .unwrap();
    assert_eq!(
        read_prop(&db, pat_oid, PropertyIdentifier::RELINQUISH_DEFAULT),
        datetime_pv()
    );
}

#[test]
fn datetime_write_with_mispaired_members_is_refused_and_preserves() {
    let mut db = ObjectDatabase::new();
    let obj = DateTimeValueObject::new(1, "DTV-1").unwrap();
    let oid = obj.object_identifier();
    db.add(Box::new(obj)).unwrap();
    let baseline = read_prop(&db, oid, PropertyIdentifier::PRESENT_VALUE);

    // Two Dates, no Time: decodes fine but fails the arm's pair shape.
    let bytes = encode_value(PropertyValue::List(vec![
        PropertyValue::Date(TEST_DATE),
        PropertyValue::Date(TEST_DATE),
    ]));
    assert_refused(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        bytes,
        ErrorCode::INVALID_DATA_TYPE,
        baseline,
        "date+date is not a BACnetDateTime",
    );
}
