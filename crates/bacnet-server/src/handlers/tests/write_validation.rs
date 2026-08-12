//! Wire-level integration for the write-path validation tranche
//! (#252, #240, #255, #270): a conformant peer's WriteProperty must be gated
//! or validated exactly as the object layer is, with the property left
//! untouched when the write is refused.
//!
//! Error pairings follow Clause 15.9.1.3: an unwritable-now property is
//! PROPERTY / WRITE_ACCESS_DENIED, a value outside the property's range is
//! PROPERTY / VALUE_OUT_OF_RANGE, and an encoding that does not match the
//! property's datatype is PROPERTY / INVALID_DATA_ENCODING.

use super::*;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_objects::trend::TrendLogObject;

fn encode_value(value: PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &value).unwrap();
    buf.to_vec()
}

fn write_wire(
    db: &mut ObjectDatabase,
    target_oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    let request = WritePropertyRequest {
        object_identifier: target_oid,
        property_identifier: property,
        property_array_index: None,
        property_value: encode_value(value),
        priority: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    handle_write_property(db, &buf).map(|_| ())
}

fn read_wire(
    db: &ObjectDatabase,
    target_oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    let request = ReadPropertyRequest {
        object_identifier: target_oid,
        property_identifier: property,
        property_array_index: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_read_property(db, &buf, &mut ack_buf).unwrap();
    let raw = ReadPropertyACK::decode(&ack_buf.to_vec())
        .unwrap()
        .property_value;
    bacnet_encoding::primitives::decode_application_value(&raw, 0)
        .unwrap()
        .0
}

/// Assert a refused write carries exactly the expected PROPERTY-class code and
/// that `property` still reads back as `expected` (state unchanged).
fn assert_refused(
    db: &mut ObjectDatabase,
    target_oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
    expected_code: ErrorCode,
    expected: PropertyValue,
    context: &str,
) {
    match write_wire(db, target_oid, property, value)
        .expect_err(&format!("{context}: write must be refused"))
    {
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
        read_wire(db, target_oid, property),
        expected,
        "{context}: refused write must leave the property unchanged"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// #240 — in-service Reliability writes are gated (Loop, Schedule); Trend Log
// Reliability is not network-writable at all.
// ──────────────────────────────────────────────────────────────────────────

/// Drive an OOS+Reliability carrier through the Clause 12.17 footnote-7 /
/// Clause 12.24 evaluation-inhibit contract over confirmed WriteProperty:
/// in-service write denied, out-of-service write validated and stored,
/// simulated value restored on the return to service.
fn assert_wire_reliability_gate(
    db: &mut ObjectDatabase,
    target_oid: ObjectIdentifier,
    context: &str,
) {
    assert_refused(
        db,
        target_oid,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(1),
        ErrorCode::WRITE_ACCESS_DENIED,
        PropertyValue::Enumerated(0),
        &format!("{context} in-service Reliability"),
    );

    write_wire(
        db,
        target_oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .expect("Out_Of_Service must be writable");

    assert_refused(
        db,
        target_oid,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(11), // reserved for a future addendum
        ErrorCode::VALUE_OUT_OF_RANGE,
        PropertyValue::Enumerated(0),
        &format!("{context} reserved Reliability"),
    );

    write_wire(
        db,
        target_oid,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(1), // NO_SENSOR
    )
    .expect("out-of-service Reliability write must succeed");
    assert_eq!(
        read_wire(db, target_oid, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(1),
        "{context}: simulated Reliability must read back"
    );

    write_wire(
        db,
        target_oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(false),
    )
    .expect("returning to service must succeed");
    assert_eq!(
        read_wire(db, target_oid, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(0),
        "{context}: returning to service must discard the simulation"
    );
}

#[test]
fn loop_reliability_gate_holds_over_write_property() {
    let mut db = ObjectDatabase::new();
    let lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
    let lo_oid = lo.object_identifier();
    db.add(Box::new(lo)).unwrap();

    assert_wire_reliability_gate(&mut db, lo_oid, "LOOP");
}

#[test]
fn schedule_reliability_gate_holds_over_write_property() {
    let mut db = ObjectDatabase::new();
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(0.0)).unwrap();
    let sched_oid = sched.object_identifier();
    db.add(Box::new(sched)).unwrap();

    assert_wire_reliability_gate(&mut db, sched_oid, "SCHEDULE");
}

#[test]
fn trend_log_reliability_write_is_denied_over_write_property() {
    let mut db = ObjectDatabase::new();
    let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    let tl_oid = tl.object_identifier();
    db.add(Box::new(tl)).unwrap();

    // Clause 12.25 Table 12-29: Reliability O, no writability footnote; the
    // object-family gate does not apply, so even out-of-service writes refuse.
    write_wire(
        &mut db,
        tl_oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    assert_refused(
        &mut db,
        tl_oid,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(1),
        ErrorCode::WRITE_ACCESS_DENIED,
        PropertyValue::Enumerated(0),
        "TL out-of-service Reliability",
    );
}
