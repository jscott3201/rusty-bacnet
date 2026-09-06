//! CHANGE_OF_STATE algorithm tests.
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use super::*;
use bacnet_objects::accumulator::AccumulatorObject;
use bacnet_objects::value_types::IntegerValueObject;
use bacnet_types::constructed::{BACnetExtendedPropertyState, BACnetProprietaryPropertyState};

fn setup_integer_change_of_state(
    present_value: i32,
    alarm_value: i32,
) -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let mut value = IntegerValueObject::new(1, "IV-1").unwrap();
    value
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Signed(present_value),
            Some(8),
        )
        .unwrap();
    let value_oid = value.object_identifier();
    db.add(Box::new(value)).unwrap();

    let mut enrollment =
        EventEnrollmentObject::new(3, "EE-COS-IV", EventType::CHANGE_OF_STATE.to_raw()).unwrap();
    enrollment.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        value_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    enrollment.set_event_parameters(BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![BACnetPropertyStates::IntegerValue(alarm_value)],
    });
    enrollment.set_event_enable(0x07);
    db.add(Box::new(enrollment)).unwrap();
    (db, value_oid)
}

// ---- CHANGE_OF_STATE tests ----

#[test]
fn change_of_state_normal_when_not_in_alarm_set() {
    // Binary INACTIVE (0), alarm on ACTIVE (1)
    let (mut db, _ee_oid, _bi_oid) = setup_change_of_state(0, &[1]);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(transitions.is_empty());
}

#[test]
fn change_of_state_to_offnormal() {
    // Binary ACTIVE (1), alarm on ACTIVE (1)
    let (mut db, ee_oid, bi_oid) = setup_change_of_state(1, &[1]);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].enrollment_oid, ee_oid);
    assert_eq!(transitions[0].monitored_oid, bi_oid);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
    assert_eq!(transitions[0].event_type, EventType::CHANGE_OF_STATE);
}

#[test]
fn change_of_state_back_to_normal() {
    let (mut db, _ee_oid, bi_oid) = setup_change_of_state(1, &[1]);
    evaluate_event_enrollments(&mut db, 1);

    // Set monitored value to non-alarm
    let bi = db.get_mut(&bi_oid).unwrap();
    bi.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    bi.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(0),
        None,
    )
    .unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::OFFNORMAL);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);
}

#[test]
fn change_of_state_large_unsigned_is_a_delayed_nonmatch() {
    let mut db = ObjectDatabase::new();
    let mut accumulator = AccumulatorObject::new(1, "ACC-1", 0).unwrap();
    accumulator
        .write_property(
            PropertyIdentifier::MAX_PRES_VALUE,
            None,
            PropertyValue::Unsigned(1),
            None,
        )
        .unwrap();
    let accumulator_oid = accumulator.object_identifier();
    db.add(Box::new(accumulator)).unwrap();

    let mut enrollment =
        EventEnrollmentObject::new(3, "EE-COS-U", EventType::CHANGE_OF_STATE.to_raw()).unwrap();
    enrollment.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        accumulator_oid,
        PropertyIdentifier::MAX_PRES_VALUE.to_raw(),
    )));
    enrollment.set_event_parameters(BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![BACnetPropertyStates::UnsignedValue(1)],
    });
    enrollment.set_time_delay_normal(Some(2));
    enrollment.set_event_enable(0x07);
    db.add(Box::new(enrollment)).unwrap();

    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::OFFNORMAL
    );
    db.get_mut(&accumulator_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::MAX_PRES_VALUE,
            None,
            PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
            None,
        )
        .unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::NORMAL
    );
}

#[test]
fn change_of_state_multiple_alarm_values() {
    // Alarm on values 1, 3, 5
    let (mut db, _ee_oid, _bi_oid) = setup_change_of_state(3, &[1, 3, 5]);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
}

#[test]
fn change_of_state_matches_signed_integer_values_end_to_end() {
    for value in [i32::MIN, -1, 7, i32::MAX] {
        let (mut db, monitored_oid) = setup_integer_change_of_state(value, value);
        let transitions = evaluate_event_enrollments(&mut db, 1);
        assert_eq!(transitions.len(), 1, "signed value {value}");
        assert_eq!(transitions[0].monitored_oid, monitored_oid);
        assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
    }
}

#[test]
fn change_of_state_matches_boolean_extended_and_proprietary_values() {
    let extended = BACnetExtendedPropertyState::new(256, 7).unwrap();
    let cases = [
        (
            BACnetPropertyStates::BooleanValue(true),
            algorithms::PropertyStateValue::Boolean(true),
        ),
        (
            BACnetPropertyStates::ExtendedValue(extended),
            algorithms::PropertyStateValue::Enumerated(7),
        ),
        (
            BACnetPropertyStates::Other(
                BACnetProprietaryPropertyState::primitive(64, vec![7]).unwrap(),
            ),
            algorithms::PropertyStateValue::Enumerated(7),
        ),
    ];

    for (state, value) in cases {
        let evaluation =
            algorithms::eval_change_of_state_struct(&[state], value, EventState::NORMAL, None);
        assert_eq!(evaluation.indication.unwrap().target, EventState::OFFNORMAL);
    }

    // Context [63] packs the choice discriminator for transport; the
    // referenced property's semantic value remains Enumerated(7).
    assert!(algorithms::eval_change_of_state_struct(
        &[BACnetPropertyStates::ExtendedValue(extended)],
        algorithms::PropertyStateValue::Unsigned(u64::from(extended.encoded())),
        EventState::NORMAL,
        None,
    )
    .indication
    .is_none());
}

#[test]
fn change_of_state_keeps_discrete_datatypes_distinct() {
    let cases = [
        (
            BACnetPropertyStates::BooleanValue(true),
            algorithms::PropertyStateValue::Boolean(true),
        ),
        (
            BACnetPropertyStates::IntegerValue(1),
            algorithms::PropertyStateValue::Signed(1),
        ),
        (
            BACnetPropertyStates::UnsignedValue(1),
            algorithms::PropertyStateValue::Unsigned(1),
        ),
        (
            BACnetPropertyStates::BinaryValue(1),
            algorithms::PropertyStateValue::Enumerated(1),
        ),
    ];

    for (state, value) in cases {
        let evaluation =
            algorithms::eval_change_of_state_struct(&[state], value, EventState::NORMAL, None);
        assert_eq!(evaluation.indication.unwrap().condition, 0);
    }

    assert!(algorithms::eval_change_of_state_struct(
        &[BACnetPropertyStates::BinaryValue(1)],
        algorithms::PropertyStateValue::Unsigned(1),
        EventState::NORMAL,
        None,
    )
    .indication
    .is_none());
    assert!(algorithms::eval_change_of_state_struct(
        &[BACnetPropertyStates::UnsignedValue(1)],
        algorithms::PropertyStateValue::Enumerated(1),
        EventState::NORMAL,
        None,
    )
    .indication
    .is_none());

    let alarm_values = [
        BACnetPropertyStates::BooleanValue(true),
        BACnetPropertyStates::UnsignedValue(1),
    ];
    let first = algorithms::eval_change_of_state_struct(
        &alarm_values,
        algorithms::PropertyStateValue::Boolean(true),
        EventState::NORMAL,
        None,
    )
    .indication
    .unwrap();
    let second = algorithms::eval_change_of_state_struct(
        &alarm_values,
        algorithms::PropertyStateValue::Unsigned(1),
        EventState::OFFNORMAL,
        first.offnormal_value,
    )
    .indication
    .unwrap();
    assert_ne!(first.condition, second.condition);
}
