use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
    ChangeOfValueCriteria,
};

/// Helper: create an EventEnrollment monitoring an AnalogInput with OUT_OF_RANGE.
fn setup_out_of_range(
    present_value: f32,
    high_limit: f32,
    low_limit: f32,
    deadband: f32,
) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();

    // Monitored analog input
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.set_present_value(present_value);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    // Event enrollment
    let mut ee = EventEnrollmentObject::new(1, "EE-OOR", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit,
        high_limit,
        deadband,
    });
    ee.set_event_enable(0x07); // all transitions
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    (db, ee_oid, ai_oid)
}

/// Helper: create an EventEnrollment monitoring an AnalogInput with FLOATING_LIMIT.
///
/// The setpoint is held by a separate AnalogInput so the
/// `setpoint_reference` resolves to `setpoint` rather than the monitored value.
fn setup_floating_limit(
    present_value: f32,
    setpoint: f32,
    high_diff: f32,
    low_diff: f32,
    deadband: f32,
) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(2, "AI-2", 62).unwrap();
    ai.set_present_value(present_value);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    // Separate setpoint object the floating-limit reference resolves to.
    let mut sp = AnalogInputObject::new(3, "AI-SP", 62).unwrap();
    sp.set_present_value(setpoint);
    let sp_oid = sp.object_identifier();
    db.add(Box::new(sp)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(2, "EE-FL", EventType::FLOATING_LIMIT.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::FloatingLimit {
        time_delay: 0,
        setpoint_reference: BACnetDeviceObjectPropertyReference::new_local(
            sp_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
        low_diff_limit: low_diff,
        high_diff_limit: high_diff,
        deadband,
    });
    ee.set_event_enable(0x07);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    (db, ee_oid, ai_oid)
}

/// Helper: create an EventEnrollment monitoring a BinaryInput with CHANGE_OF_STATE.
fn setup_change_of_state(
    present_value: u32,
    alarm_values: &[u32],
) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();

    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    bi.set_present_value(present_value);
    let bi_oid = bi.object_identifier();
    db.add(Box::new(bi)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(3, "EE-COS", EventType::CHANGE_OF_STATE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        bi_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: alarm_values
            .iter()
            .map(|v| BACnetPropertyStates::UnsignedValue(*v))
            .collect(),
    });
    ee.set_event_enable(0x07);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    (db, ee_oid, bi_oid)
}

// ---- OUT_OF_RANGE tests ----

#[test]
fn out_of_range_normal_stays_normal() {
    let (mut db, _ee_oid, _ai_oid) = setup_out_of_range(50.0, 80.0, 20.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());
}

#[test]
fn out_of_range_normal_to_high_limit() {
    let (mut db, ee_oid, ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].enrollment_oid, ee_oid);
    assert_eq!(transitions[0].monitored_oid, ai_oid);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
    assert_eq!(transitions[0].event_type, EventType::OUT_OF_RANGE);

    // Verify event_state was persisted
    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
}

#[test]
fn out_of_range_normal_to_low_limit() {
    let (mut db, ee_oid, _ai_oid) = setup_out_of_range(15.0, 80.0, 20.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::LOW_LIMIT);

    // Verify persisted state
    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::LOW_LIMIT.to_raw())
    );
}

#[test]
fn out_of_range_high_to_normal_with_deadband() {
    let (mut db, ee_oid, ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
    // First: go to HIGH_LIMIT
    evaluate_event_enrollments(&mut db);

    // Update monitored value — still within deadband (80 - 2 = 78)
    let ai = db.get_mut(&ai_oid).unwrap();
    ai.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(79.0),
        None,
    )
    .unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty(), "within deadband — no transition");

    // Drop below deadband
    let ai = db.get_mut(&ai_oid).unwrap();
    ai.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(77.0),
        None,
    )
    .unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::HIGH_LIMIT);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);

    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

#[test]
fn out_of_range_no_change_when_already_faulted() {
    let (mut db, _ee_oid, _ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
    let t1 = evaluate_event_enrollments(&mut db);
    assert_eq!(t1.len(), 1);

    // Second evaluation: same state, no new transition
    let t2 = evaluate_event_enrollments(&mut db);
    assert!(t2.is_empty());
}

#[test]
fn out_of_range_event_enable_suppresses_notification() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(10, "AI-10", 62).unwrap();
    ai.set_present_value(85.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(10, "EE-sup", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(0x04); // only TO_NORMAL enabled
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    // TO_OFFNORMAL not enabled — should not appear in transitions
    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());

    // But event_state should NOT have been updated (notification suppressed)
    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

#[test]
fn out_of_range_skips_out_of_service() {
    let (mut db, ee_oid, _ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);

    // Set enrollment to out-of-service
    let obj = db.get_mut(&ee_oid).unwrap();
    obj.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());
}

// ---- FLOATING_LIMIT tests ----

#[test]
fn floating_limit_normal_stays_normal() {
    // setpoint=50, high_diff=10, low_diff=10 → limits at 60/40
    let (mut db, _ee_oid, _ai_oid) = setup_floating_limit(50.0, 50.0, 10.0, 10.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());
}

#[test]
fn floating_limit_to_high() {
    // setpoint=50, high_diff=10 → high_limit=60; value=65 exceeds
    let (mut db, ee_oid, ai_oid) = setup_floating_limit(65.0, 50.0, 10.0, 10.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].enrollment_oid, ee_oid);
    assert_eq!(transitions[0].monitored_oid, ai_oid);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
    assert_eq!(transitions[0].event_type, EventType::FLOATING_LIMIT);
}

#[test]
fn floating_limit_to_low() {
    // setpoint=50, low_diff=10 → low_limit=40; value=35 below
    let (mut db, _ee_oid, _ai_oid) = setup_floating_limit(35.0, 50.0, 10.0, 10.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::LOW_LIMIT);
}

#[test]
fn floating_limit_deadband_hysteresis() {
    // setpoint=50, high_diff=10, deadband=2 → high_limit=60, return threshold=58
    let (mut db, _ee_oid, ai_oid) = setup_floating_limit(65.0, 50.0, 10.0, 10.0, 2.0);
    evaluate_event_enrollments(&mut db);

    // Still above return threshold (58)
    let ai = db.get_mut(&ai_oid).unwrap();
    ai.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(59.0),
        None,
    )
    .unwrap();
    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());

    // Below return threshold
    let ai = db.get_mut(&ai_oid).unwrap();
    ai.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(57.0),
        None,
    )
    .unwrap();
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);
}

// ---- CHANGE_OF_STATE tests ----

#[test]
fn change_of_state_normal_when_not_in_alarm_set() {
    // Binary INACTIVE (0), alarm on ACTIVE (1)
    let (mut db, _ee_oid, _bi_oid) = setup_change_of_state(0, &[1]);
    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());
}

#[test]
fn change_of_state_to_offnormal() {
    // Binary ACTIVE (1), alarm on ACTIVE (1)
    let (mut db, ee_oid, bi_oid) = setup_change_of_state(1, &[1]);
    let transitions = evaluate_event_enrollments(&mut db);
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
    evaluate_event_enrollments(&mut db);

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

    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::OFFNORMAL);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);
}

#[test]
fn change_of_state_multiple_alarm_values() {
    // Alarm on values 1, 3, 5
    let (mut db, _ee_oid, _bi_oid) = setup_change_of_state(3, &[1, 3, 5]);
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
}

// ---- CHANGE_OF_BITSTRING tests ----

#[test]
fn change_of_bitstring_normal() {
    let mut db = ObjectDatabase::new();

    // Create an object with a bitstring property (using a multistate or similar)
    // For testing, we'll use an EventEnrollment monitoring another enrollment's EVENT_ENABLE
    let mut target = EventEnrollmentObject::new(50, "Target", EventType::NONE.to_raw()).unwrap();
    // EVENT_ENABLE is a 3-bit bitstring
    target.set_event_enable(0x05); // bits: TO_OFFNORMAL | TO_NORMAL
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(51, "EE-COBS", EventType::CHANGE_OF_BITSTRING.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        target_oid,
        PropertyIdentifier::EVENT_ENABLE.to_raw(),
    )));
    // mask=0xFF, alarm_pattern=0xE0 (all 3 high bits set)
    ee.set_event_parameters(BACnetEventParameter::ChangeOfBitstring {
        time_delay: 0,
        bitmask: (0, vec![0xFF]),
        list_of_values: vec![(0, vec![0xE0])],
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    // 0x05 << 5 = 0xA0, mask 0xFF → 0xA0, alarm 0xE0 → no match → NORMAL
    assert!(transitions.is_empty());
}

#[test]
fn change_of_bitstring_offnormal() {
    let mut db = ObjectDatabase::new();

    let mut target = EventEnrollmentObject::new(60, "Target2", EventType::NONE.to_raw()).unwrap();
    target.set_event_enable(0x07); // all 3 bits set
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(61, "EE-COBS2", EventType::CHANGE_OF_BITSTRING.to_raw())
            .unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        target_oid,
        PropertyIdentifier::EVENT_ENABLE.to_raw(),
    )));
    // mask=0xE0, alarm_pattern=0xE0 (all 3 high bits)
    ee.set_event_parameters(BACnetEventParameter::ChangeOfBitstring {
        time_delay: 0,
        bitmask: (0, vec![0xE0]),
        list_of_values: vec![(0, vec![0xE0])],
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    // 0x07 << 5 = 0xE0, mask 0xE0 → 0xE0, alarm 0xE0 → match → OFFNORMAL
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
}

// ---- CHANGE_OF_VALUE tests ----

#[test]
fn change_of_value_within_increment() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(70, "AI-COV", 62).unwrap();
    ai.set_present_value(3.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(70, "EE-COV", EventType::CHANGE_OF_VALUE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    // |3.0| < 5.0 → NORMAL
    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());
}

#[test]
fn change_of_value_exceeds_increment() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(71, "AI-COV2", 62).unwrap();
    ai.set_present_value(10.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(71, "EE-COV2", EventType::CHANGE_OF_VALUE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    // |10.0| >= 5.0 → OFFNORMAL
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
}

// ---- Integration: multiple enrollments ----

#[test]
fn evaluates_multiple_enrollments() {
    let mut db = ObjectDatabase::new();

    // Two analog inputs
    let mut ai1 = AnalogInputObject::new(80, "AI-80", 62).unwrap();
    ai1.set_present_value(90.0); // will trigger HIGH_LIMIT
    let ai1_oid = ai1.object_identifier();
    db.add(Box::new(ai1)).unwrap();

    let mut ai2 = AnalogInputObject::new(81, "AI-81", 62).unwrap();
    ai2.set_present_value(50.0); // normal
    let ai2_oid = ai2.object_identifier();
    db.add(Box::new(ai2)).unwrap();

    // Two enrollments
    let mut ee1 =
        EventEnrollmentObject::new(80, "EE-80", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee1.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai1_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee1.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee1.set_event_enable(0x07);
    db.add(Box::new(ee1)).unwrap();

    let mut ee2 =
        EventEnrollmentObject::new(81, "EE-81", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee2.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai2_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee2.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee2.set_event_enable(0x07);
    db.add(Box::new(ee2)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    // Only AI-80 triggers (90 > 80)
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].monitored_oid, ai1_oid);
}

#[test]
fn missing_monitored_object_is_skipped() {
    let mut db = ObjectDatabase::new();

    let fake_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 999).unwrap();
    let mut ee =
        EventEnrollmentObject::new(90, "EE-miss", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        fake_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    // Should not panic or return transitions
    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());
}

// ---- Extended / Opaque (compat) tests ----

#[test]
fn extended_algorithm_produces_no_transition() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(93, "AI-93", 62).unwrap();
    ai.set_present_value(85.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(93, "EE-ext", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    // Extended [9] is preserved but not evaluated — no transition.
    ee.set_event_parameters(BACnetEventParameter::Extended {
        vendor_id: 42,
        extended_event_type: 99,
        parameters: vec![0xDE, 0xAD],
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    assert!(
        transitions.is_empty(),
        "Extended algorithm is not evaluated"
    );
}

/// Legacy raw-octet OUT_OF_RANGE values written by an older client must still
/// evaluate correctly. The octets are wrapped as `Opaque` and the evaluator
/// falls back to the little-endian byte layout keyed on `Event_Type`.
#[test]
fn legacy_le_out_of_range_fallback_round_trip() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(94, "AI-94", 62).unwrap();
    ai.set_present_value(85.0); // above high_limit 80 → HIGH_LIMIT
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(94, "EE-leg", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    // Simulate an old client writing raw little-endian octets.
    ee.write_property(
        PropertyIdentifier::EVENT_PARAMETERS,
        None,
        PropertyValue::OctetString(encode_out_of_range_params(80.0, 20.0, 2.0)),
        None,
    )
    .unwrap();
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
}

/// Legacy raw-octet CHANGE_OF_STATE values must still evaluate correctly via
/// the `Event_Type`-keyed fallback.
#[test]
fn legacy_le_change_of_state_fallback() {
    let mut db = ObjectDatabase::new();

    let mut bi = BinaryInputObject::new(95, "BI-95").unwrap();
    bi.set_present_value(1); // matches alarm value 1 → OFFNORMAL
    let bi_oid = bi.object_identifier();
    db.add(Box::new(bi)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(95, "EE-legcos", EventType::CHANGE_OF_STATE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        bi_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::Opaque {
        tag: 0xFF,
        data: encode_change_of_state_params(&[1]),
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
}

/// CHANGE_OF_VALUE with a `bitmask` criteria reports OFFNORMAL when any masked
/// bit is set on the monitored bitstring value.
#[test]
fn change_of_value_bitmask_criteria() {
    let mut db = ObjectDatabase::new();

    // Target object exposing a bitstring property (EVENT_ENABLE, 3 bits).
    let mut target = EventEnrollmentObject::new(96, "Tgt", EventType::NONE.to_raw()).unwrap();
    target.set_event_enable(0x07); // 0x07 << 5 = 0xE0
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(97, "EE-covbm", EventType::CHANGE_OF_VALUE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        target_oid,
        PropertyIdentifier::EVENT_ENABLE.to_raw(),
    )));
    // Bitmask criterion: any bit in 0x80 set → OFFNORMAL.
    ee.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: ChangeOfValueCriteria::Bitmask {
            unused_bits: 5,
            data: vec![0x80],
        },
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    // 0xE0 & 0x80 = 0x80 (bit set) → OFFNORMAL
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
}

/// A CHANGE_OF_VALUE enrollment whose monitored value is the wrong type for
/// its criterion must skip evaluation rather than spuriously transitioning
/// to NORMAL (which could emit a false TO_NORMAL notification).
#[test]
fn change_of_value_wrong_type_monitored_value_skips() {
    let mut db = ObjectDatabase::new();

    // Target object whose EVENT_ENABLE is a BitString (not a Real) — the
    // ReferencedPropertyIncrement criterion needs a Real.
    let mut target = EventEnrollmentObject::new(98, "Tgt2", EventType::NONE.to_raw()).unwrap();
    target.set_event_enable(0x07);
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(99, "EE-covwt", EventType::CHANGE_OF_VALUE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        target_oid,
        PropertyIdentifier::EVENT_ENABLE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
    });
    ee.set_event_enable(0x07);
    // Force the enrollment into OFFNORMAL so a spurious NORMAL transition
    // would otherwise be emitted.
    ee.write_property(
        PropertyIdentifier::EVENT_STATE,
        None,
        PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw()),
        None,
    )
    .unwrap();
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    // Wrong-type monitored value → skip, no transition to NORMAL.
    assert!(
        transitions.is_empty(),
        "wrong-type monitored value must skip"
    );
}

#[test]
fn no_reference_is_skipped() {
    let mut db = ObjectDatabase::new();

    let ee = EventEnrollmentObject::new(91, "EE-noref", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());
}

#[test]
fn empty_parameters_is_skipped() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(92, "AI-92", 62).unwrap();
    ai.set_present_value(100.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(92, "EE-noparam", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    // No parameters set — should remain at current state
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());
}
