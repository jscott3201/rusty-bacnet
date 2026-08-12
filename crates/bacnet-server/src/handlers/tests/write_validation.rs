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
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_objects::multistate::MultiStateOutputObject;
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
    let raw = ReadPropertyACK::decode(&ack_buf).unwrap().property_value;
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

// ──────────────────────────────────────────────────────────────────────────
// #255 — Notify_Type rejects values outside the three-value production;
// Event_Enable / Limit_Enable reject a BitString whose declared shape does
// not match its fixed-width production.
// ──────────────────────────────────────────────────────────────────────────

/// BACnetNotifyType is a closed production {alarm(0), event(1),
/// ack-notification(2)} (Clause 21): an out-of-production write is
/// PROPERTY / VALUE_OUT_OF_RANGE per Clause 15.9.1.3.
fn assert_notify_type_validated(
    db: &mut ObjectDatabase,
    target_oid: ObjectIdentifier,
    baseline: PropertyValue,
    context: &str,
) {
    for out_of_production in [3u32, 99, u32::MAX] {
        assert_refused(
            db,
            target_oid,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyValue::Enumerated(out_of_production),
            ErrorCode::VALUE_OUT_OF_RANGE,
            baseline.clone(),
            &format!("{context} Notify_Type={out_of_production}"),
        );
    }
    for in_production in [0u32, 1, 2] {
        write_wire(
            db,
            target_oid,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyValue::Enumerated(in_production),
        )
        .expect("named Notify_Type values must be accepted");
    }
}

/// A fixed N-bit production has exactly one canonical encoding: one content
/// octet whose high N bits are content (8-N unused). Any other shape is
/// PROPERTY / INVALID_DATA_ENCODING per Clause 15.9.1.3.
fn assert_event_enable_validated(
    db: &mut ObjectDatabase,
    target_oid: ObjectIdentifier,
    context: &str,
) {
    let baseline = read_wire(db, target_oid, PropertyIdentifier::EVENT_ENABLE);
    for (unused_bits, data, label) in [
        (0u8, vec![0xFFu8], "8-bit string where 3 are defined"),
        (5u8, vec![0xFFu8, 0xFF], "two content octets"),
        (4u8, vec![0xF0u8], "half-octet string"),
        (5u8, vec![], "no content octet"),
    ] {
        assert_refused(
            db,
            target_oid,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::BitString { unused_bits, data },
            ErrorCode::INVALID_DATA_ENCODING,
            baseline.clone(),
            &format!("{context} Event_Enable {label}"),
        );
    }

    // The canonical full-width encoding stays accepted.
    write_wire(
        db,
        target_oid,
        PropertyIdentifier::EVENT_ENABLE,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xA0], // to-offnormal + to-normal, MSB-first
        },
    )
    .expect("canonical 3-bit encoding must be accepted");
    assert_eq!(
        read_wire(db, target_oid, PropertyIdentifier::EVENT_ENABLE),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xA0],
        },
        "{context}: accepted Event_Enable must read back"
    );
}

#[test]
fn analog_event_property_writes_are_validated_over_write_property() {
    let mut db = ObjectDatabase::new();
    let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    assert_notify_type_validated(&mut db, ai_oid, PropertyValue::Enumerated(0), "AI");
    assert_event_enable_validated(&mut db, ai_oid, "AI");

    // Limit_Enable is a 2-bit production (BACnetLimitEnable): unused_bits must
    // be 6; the 3-bit Event_Enable shape must not be accepted here.
    let baseline = read_wire(&db, ai_oid, PropertyIdentifier::LIMIT_ENABLE);
    for (unused_bits, data, label) in [
        (0u8, vec![0xFFu8], "8-bit string where 2 are defined"),
        (5u8, vec![0xE0u8], "Event_Enable's 3-bit shape"),
        (6u8, vec![0xC0u8, 0x00], "extra content octet"),
    ] {
        assert_refused(
            &mut db,
            ai_oid,
            PropertyIdentifier::LIMIT_ENABLE,
            PropertyValue::BitString { unused_bits, data },
            ErrorCode::INVALID_DATA_ENCODING,
            baseline.clone(),
            &format!("AI Limit_Enable {label}"),
        );
    }
    write_wire(
        &mut db,
        ai_oid,
        PropertyIdentifier::LIMIT_ENABLE,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![0xC0], // both limits enabled, MSB-first
        },
    )
    .expect("canonical 2-bit encoding must be accepted");
}

#[test]
fn binary_event_property_writes_are_validated_over_write_property() {
    let mut db = ObjectDatabase::new();
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let bv_oid = bv.object_identifier();
    db.add(Box::new(bv)).unwrap();

    assert_notify_type_validated(&mut db, bv_oid, PropertyValue::Enumerated(0), "BV");
    assert_event_enable_validated(&mut db, bv_oid, "BV");
}

#[test]
fn multistate_event_property_writes_are_validated_over_write_property() {
    let mut db = ObjectDatabase::new();
    let mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    let mso_oid = mso.object_identifier();
    db.add(Box::new(mso)).unwrap();

    assert_notify_type_validated(&mut db, mso_oid, PropertyValue::Enumerated(0), "MSO");
    assert_event_enable_validated(&mut db, mso_oid, "MSO");
}

#[test]
fn event_enrollment_writes_are_validated_over_write_property() {
    let mut db = ObjectDatabase::new();
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    assert_notify_type_validated(&mut db, ee_oid, PropertyValue::Enumerated(0), "EE");
    assert_event_enable_validated(&mut db, ee_oid, "EE");
}

// ──────────────────────────────────────────────────────────────────────────
// #270 — Relinquish_Default is writable on commandable objects (permitted,
// not required, writability), validated like a Present_Value write, and the
// Present_Value resolves to it once the priority array is empty.
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn relinquish_default_write_recaptures_present_value_over_write_property() {
    let mut db = ObjectDatabase::new();
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    let ao_oid = ao.object_identifier();
    db.add(Box::new(ao)).unwrap();

    // Occupy priority 8 so PV tracks the command, not the default.
    let slot = WritePropertyRequest {
        object_identifier: ao_oid,
        property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
        property_array_index: Some(8),
        property_value: encode_value(PropertyValue::Real(55.0)),
        priority: None,
    };
    let mut buf = BytesMut::new();
    slot.encode(&mut buf);
    handle_write_property(&mut db, &buf).unwrap();
    assert_eq!(
        read_wire(&db, ao_oid, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(55.0)
    );

    write_wire(
        &mut db,
        ao_oid,
        PropertyIdentifier::RELINQUISH_DEFAULT,
        PropertyValue::Real(12.5),
    )
    .expect("Relinquish_Default must be writable");
    assert_eq!(
        read_wire(&db, ao_oid, PropertyIdentifier::RELINQUISH_DEFAULT),
        PropertyValue::Real(12.5)
    );
    assert_eq!(
        read_wire(&db, ao_oid, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(55.0),
        "a live command must still outrank the new default"
    );

    // Non-finite is rejected with no state change.
    assert_refused(
        &mut db,
        ao_oid,
        PropertyIdentifier::RELINQUISH_DEFAULT,
        PropertyValue::Real(f32::NAN),
        ErrorCode::VALUE_OUT_OF_RANGE,
        PropertyValue::Real(12.5),
        "AO NaN Relinquish_Default",
    );

    // Relinquish priority 8: PV falls back to the new default.
    let slot = WritePropertyRequest {
        object_identifier: ao_oid,
        property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
        property_array_index: Some(8),
        property_value: encode_value(PropertyValue::Null),
        priority: None,
    };
    let mut buf = BytesMut::new();
    slot.encode(&mut buf);
    handle_write_property(&mut db, &buf).unwrap();
    assert_eq!(
        read_wire(&db, ao_oid, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(12.5),
        "with an empty priority array, PV must resolve to the written default"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// #225 — Time_Delay_Normal (property 356): the Clause 13.3 pTimeDelayNormal
// backing store on a representative intrinsic-reporting type (Analog Input,
// Table 12-2 row O5). RP/WP over the wire, fallback readback when unwritten,
// and the shared write-path validation pairings on refusal.
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn time_delay_normal_round_trips_over_write_property_and_read_property() {
    let mut db = ObjectDatabase::new();
    let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    // Never written, the read-back is Time_Delay's value: Clause 13.3 — "If
    // no value is available for this parameter, then it takes on the value of
    // the pTimeDelay parameter."
    write_wire(
        &mut db,
        ai_oid,
        PropertyIdentifier::TIME_DELAY,
        PropertyValue::Unsigned(11),
    )
    .expect("Time_Delay must be writable");
    assert_eq!(
        read_wire(&db, ai_oid, PropertyIdentifier::TIME_DELAY_NORMAL),
        PropertyValue::Unsigned(11)
    );

    // The property itself is settlable and reads back independently.
    write_wire(
        &mut db,
        ai_oid,
        PropertyIdentifier::TIME_DELAY_NORMAL,
        PropertyValue::Unsigned(9),
    )
    .expect("Time_Delay_Normal must be writable");
    assert_eq!(
        read_wire(&db, ai_oid, PropertyIdentifier::TIME_DELAY_NORMAL),
        PropertyValue::Unsigned(9)
    );
    assert_eq!(
        read_wire(&db, ai_oid, PropertyIdentifier::TIME_DELAY),
        PropertyValue::Unsigned(11),
        "Time_Delay is untouched by a Time_Delay_Normal write"
    );

    // Wrong BACnet datatype: PROPERTY / INVALID_DATA_TYPE, value preserved.
    assert_refused(
        &mut db,
        ai_oid,
        PropertyIdentifier::TIME_DELAY_NORMAL,
        PropertyValue::Enumerated(9),
        ErrorCode::INVALID_DATA_TYPE,
        PropertyValue::Unsigned(9),
        "AI Enumerated Time_Delay_Normal",
    );

    // Unrepresentable in the u32 backing store: PROPERTY / VALUE_OUT_OF_RANGE.
    assert_refused(
        &mut db,
        ai_oid,
        PropertyIdentifier::TIME_DELAY_NORMAL,
        PropertyValue::Unsigned(u32::MAX as u64 + 1),
        ErrorCode::VALUE_OUT_OF_RANGE,
        PropertyValue::Unsigned(9),
        "AI oversized Time_Delay_Normal",
    );
}
