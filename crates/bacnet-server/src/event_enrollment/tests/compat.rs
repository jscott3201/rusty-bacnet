//! Extended and Opaque (legacy raw-octet) compatibility tests.
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, ChangeOfValueCriteria,
};

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
    target.set_event_enable(0x07); // internal 0x07 -> wire 0xE0 (MSB-first)
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
    // would otherwise be emitted. Seeded via the internal builder, not a
    // network write — `Event_State` is read-only over the network (issue #130).
    ee.set_event_state(EventState::OFFNORMAL.to_raw());
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

/// Periodic evaluation persists a detected transition through the internal
/// lifecycle path (`set_event_state_internal`), NOT the network
/// `write_property(EVENT_STATE, …)` route. The network route rejects
/// `EVENT_STATE` writes entirely (issue #130): driving a transition still
/// updates `Event_State`, and a direct network write of `EVENT_STATE` on the
/// same object is refused.
#[test]
fn evaluation_does_not_use_network_write_route() {
    use bacnet_objects::event_enrollment::EventEnrollmentObject;

    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(77, "AI-77", 62).unwrap();
    ai.set_present_value(85.0); // above high_limit 80 → HIGH_LIMIT
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee = EventEnrollmentObject::new(77, "EE-77", EventType::OUT_OF_RANGE.to_raw()).unwrap();
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
    ee.set_event_enable(0x07);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    // 1) Evaluation drives a transition and persists the new Event_State.
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()),
        "evaluation must persist Event_State"
    );

    // 2) The network route rejects a direct EVENT_STATE write on the same
    //    object — proving evaluation did NOT go through the public write route.
    let obj = db.get_mut(&ee_oid).unwrap();
    let net_result = obj.write_property(
        PropertyIdentifier::EVENT_STATE,
        None,
        PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        None,
    );
    assert!(
        net_result.is_err(),
        "network EVENT_STATE write must be rejected (read-only over the network)"
    );
    // And the field is still the evaluator-set value, not the rejected write.
    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()),
        "rejected network write must not mutate Event_State"
    );
}
