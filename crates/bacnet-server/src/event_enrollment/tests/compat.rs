//! Extended, Opaque (legacy raw-octet) and unevaluated-algorithm
//! compatibility tests.
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
use bacnet_types::enums::EngineeringUnits;

// ---- Extended / Opaque (compat) tests ----

#[test]
fn extended_algorithm_produces_no_transition() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(93, "AI-93", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(85.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee = EventEnrollmentObject::new(93, "EE-ext", EventType::OUT_OF_RANGE).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    // Extended [9] is preserved but not evaluated — no transition.
    ee.set_event_parameters(BACnetEventParameter::Extended {
        vendor_id: 42,
        extended_event_type: 99,
        parameters: vec![0x21, 0x07],
    });
    ee.set_event_enable(EventTransitionBits::all());
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
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

    let mut ai = AnalogInputObject::new(94, "AI-94", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(85.0); // above high_limit 80 → HIGH_LIMIT
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee = EventEnrollmentObject::new(94, "EE-leg", EventType::OUT_OF_RANGE).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    // Simulate an old client writing raw little-endian octets.
    ee.write_property(
        PropertyIdentifier::EVENT_PARAMETERS,
        None,
        PropertyValue::OctetString(encode_out_of_range_params(80.0, 20.0, 2.0)),
        None,
    )
    .unwrap();
    ee.set_event_enable(EventTransitionBits::all());
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
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

    let mut ee = EventEnrollmentObject::new(95, "EE-legcos", EventType::CHANGE_OF_STATE).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        bi_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    ee.set_event_parameters(BACnetEventParameter::Opaque {
        tag: 0xFF,
        data: encode_change_of_state_params(&[1]),
    });
    ee.set_event_enable(EventTransitionBits::all());
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
}

/// A conformant framed write of an algorithm the evaluator doesn't run
/// (access-event [13]) is stored and read back, but must NEVER route to the
/// legacy little-endian evaluator: the TLV body is not an LE limits payload,
/// and reinterpreting it as one fabricates spurious transitions. (Review
/// blocker: pre-fix, an opaque access-event produced a HIGH_LIMIT
/// transition.) Since #2090 the alternative decodes to its own variant, and
/// it still produces no transition.
#[test]
fn framed_unevaluated_alternative_is_never_le_evaluated() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(96, "AI-96", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(85.0); // would exceed a band decoded from the octets
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee = EventEnrollmentObject::new(96, "EE-unevaluated", EventType::OUT_OF_RANGE).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    ee.set_event_enable(EventTransitionBits::all());
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    // Hand-built access-event alternative (tag 13): member [0] lists six
    // access events, member [1] is the time-reference property reference.
    let framed: Vec<u8> = vec![
        0xDE, // opening tag [13]
        0x0E, 0x91, 0x01, 0x91, 0x02, 0x91, 0x03, 0x91, 0x04, 0x91, 0x05, 0x91, 0x06,
        0x0F, // [0] list-of-access-events
        0x1E, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x1F, // [1] time reference
        0xDF, // closing tag [13]
    ];
    let obj = db.get_mut(&ee_oid).unwrap();
    obj.write_property(
        PropertyIdentifier::EVENT_PARAMETERS,
        None,
        PropertyValue::ApplicationData(framed.clone()),
        None,
    )
    .unwrap();

    // The value READS BACK byte-identical…
    let val = db
        .get(&ee_oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::ApplicationData(framed.clone()),
        "the framed alternative must round-trip verbatim"
    );
    assert!(matches!(
        bacnet_encoding::constructed::decode_event_parameter(&framed, 0)
            .unwrap()
            .0,
        BACnetEventParameter::AccessEvent { .. }
    ));

    // …but produces NO event transition: it is not evaluated as LE limits.
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(
        transitions.is_empty(),
        "an unevaluated alternative must not be fed to eval_legacy_le: {transitions:?}"
    );
}

/// Every algorithm #2090 decoded that the evaluator doesn't run behaves as
/// it did while it was opaque: stored, no transition, the state left NORMAL.
/// Its unset Time_Delay_Normal reads back its own time delay, or 0 for the
/// two with none (buffer-ready, access-event).
#[test]
fn newly_decoded_algorithms_stay_unevaluated() {
    use bacnet_types::enums::{AccessEvent, LifeSafetyState, TimerState};
    use bacnet_types::primitives::StatusFlags;
    let reference = |instance| {
        BACnetDeviceObjectPropertyReference::new_local(
            ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap(),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )
    };
    use BACnetEventParameter as EP;
    let algorithms = [
        EP::CommandFailure {
            time_delay: 7,
            feedback_property_reference: reference(1),
        },
        EP::ChangeOfLifeSafety {
            time_delay: 7,
            list_of_life_safety_alarm_values: vec![LifeSafetyState::ALARM],
            list_of_alarm_values: vec![LifeSafetyState::PRE_ALARM],
            mode_property_reference: reference(2),
        },
        EP::BufferReady {
            notification_threshold: 1,
            previous_notification_count: 0,
        },
        EP::UnsignedRange {
            time_delay: 7,
            low_limit: 0,
            high_limit: 1,
        },
        EP::AccessEvent {
            list_of_access_events: vec![AccessEvent::GRANTED],
            access_event_time_reference: reference(3),
        },
        EP::DoubleOutOfRange {
            time_delay: 7,
            low_limit: 0.0,
            high_limit: 1.0,
            deadband: 0.0,
        },
        EP::SignedOutOfRange {
            time_delay: 7,
            low_limit: 0,
            high_limit: 1,
            deadband: 0,
        },
        EP::UnsignedOutOfRange {
            time_delay: 7,
            low_limit: 0,
            high_limit: 1,
            deadband: 0,
        },
        EP::ChangeOfCharacterstring {
            time_delay: 7,
            list_of_alarm_values: vec!["85".into()],
        },
        EP::ChangeOfStatusFlags {
            time_delay: 7,
            selected_flags: StatusFlags::all(),
        },
        EP::ChangeOfDiscreteValue { time_delay: 7 },
        EP::ChangeOfTimer {
            time_delay: 7,
            alarm_values: vec![TimerState::EXPIRED],
            update_time_reference: reference(4),
        },
    ];
    for algorithm in algorithms {
        let mut db = ObjectDatabase::new();
        let mut ai =
            AnalogInputObject::new(97, "AI-97", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        ai.set_present_value(85.0); // outside every band above
        let ai_oid = ai.object_identifier();
        db.add(Box::new(ai)).unwrap();
        let mut ee = EventEnrollmentObject::new(97, "EE-97", EventType::OUT_OF_RANGE).unwrap();
        ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            ai_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )))
        .unwrap();
        ee.set_event_parameters(algorithm.clone());
        ee.set_event_enable(EventTransitionBits::all());
        let ee_oid = ee.object_identifier();
        db.add(Box::new(ee)).unwrap();

        for tick in 1..=3 {
            let transitions = evaluate_event_enrollments(&mut db, tick);
            assert!(transitions.is_empty(), "{algorithm:?}: {transitions:?}");
        }
        let state = db
            .get(&ee_oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap();
        assert_eq!(
            state,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
            "{algorithm:?}"
        );
        let expected = match algorithm {
            EP::BufferReady { .. } | EP::AccessEvent { .. } => 0,
            _ => 7,
        };
        let delay = db
            .get(&ee_oid)
            .unwrap()
            .read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None)
            .unwrap();
        assert_eq!(delay, PropertyValue::Unsigned(expected), "{algorithm:?}");
    }
}

/// CHANGE_OF_VALUE with a `bitmask` criteria indicates NORMAL→NORMAL when any
/// *significant (masked) bit changes state* against the detection baseline
/// (Clause 13.3.3, BIT STRING form) — never OFFNORMAL (Figure 13-10 admits no
/// offnormal transition). The first sample establishes the baseline.
#[test]
fn change_of_value_bitmask_criteria() {
    let mut db = ObjectDatabase::new();

    // Target object exposing a bitstring property (EVENT_ENABLE, 3 bits).
    let mut target = EventEnrollmentObject::new(96, "Tgt", EventType::NONE).unwrap();
    target.set_event_enable(EventTransitionBits::all()); // wire 0xE0 (MSB-first)
    let target_oid = target.object_identifier();
    // Keep this Event Enrollment target itself healthy now that Reliability
    // evaluation applies to every Event Enrollment object in the database.
    target
        .set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            target_oid,
            PropertyIdentifier::EVENT_ENABLE.to_raw(),
        )))
        .unwrap();
    target.set_event_parameters(BACnetEventParameter::Extended {
        vendor_id: 42,
        extended_event_type: 1,
        parameters: Vec::new(),
    });
    db.add(Box::new(target)).unwrap();

    let mut ee = EventEnrollmentObject::new(97, "EE-covbm", EventType::CHANGE_OF_VALUE).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        target_oid,
        PropertyIdentifier::EVENT_ENABLE.to_raw(),
    )))
    .unwrap();
    // Bitmask criterion: bit 0x80 is the significant one.
    ee.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: ChangeOfValueCriteria::Bitmask {
            unused_bits: 5,
            data: vec![0x80],
        },
    });
    ee.set_event_enable(EventTransitionBits::all());
    db.add(Box::new(ee)).unwrap();

    // First sample: baseline masked value = 0xE0 & 0x80 = 0x80; no transition
    // is indicated even though the significant bit is set — COV detects
    // CHANGE, not state.
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "first sample initializes the baseline; it must not indicate"
    );

    // An insignificant bit changing state indicates nothing: mask 0x80 makes
    // the 0xE0 -> 0xA0 flip invisible (masked value stays 0x80).
    db.get_mut(&target_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xA0],
            },
            None,
        )
        .unwrap();
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "only masked bits are significant (13.3.3: a BIT STRING change counts \
         only in bits the bitmask selects)"
    );

    // The significant bit clearing is a change: NORMAL -> NORMAL, actions run.
    db.get_mut(&target_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x20],
            },
            None,
        )
        .unwrap();
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);

    // Baseline advanced: holding the new masked value indicates nothing.
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
}

/// A CHANGE_OF_VALUE enrollment whose monitored value is the wrong type for
/// its criterion must skip evaluation rather than spuriously transitioning
/// to NORMAL (which could emit a false TO_NORMAL notification).
#[test]
fn change_of_value_wrong_type_monitored_value_skips() {
    let mut db = ObjectDatabase::new();

    // Target object whose EVENT_ENABLE is a BitString (not a Real) — the
    // ReferencedPropertyIncrement criterion needs a Real.
    let mut target = EventEnrollmentObject::new(98, "Tgt2", EventType::NONE).unwrap();
    target.set_event_enable(EventTransitionBits::all());
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut ee = EventEnrollmentObject::new(99, "EE-covwt", EventType::CHANGE_OF_VALUE).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        target_oid,
        PropertyIdentifier::EVENT_ENABLE.to_raw(),
    )))
    .unwrap();
    ee.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
    });
    ee.set_event_enable(EventTransitionBits::all());
    // Force the enrollment into OFFNORMAL so a spurious NORMAL transition
    // would otherwise be emitted. Seeded via the internal builder, not a
    // network write — `Event_State` is read-only over the network (issue #130).
    ee.set_event_state(EventState::OFFNORMAL);
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    // Wrong-type monitored value → skip, no transition to NORMAL.
    assert!(
        transitions.is_empty(),
        "wrong-type monitored value must skip"
    );
}

#[test]
fn no_reference_is_skipped() {
    let mut db = ObjectDatabase::new();

    let ee = EventEnrollmentObject::new(91, "EE-noref", EventType::OUT_OF_RANGE).unwrap();
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(transitions.is_empty());
}

#[test]
fn empty_parameters_is_skipped() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(92, "AI-92", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(100.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee = EventEnrollmentObject::new(92, "EE-noparam", EventType::OUT_OF_RANGE).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    // No parameters set — should remain at current state
    ee.set_event_enable(EventTransitionBits::all());
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
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
    use bacnet_types::enums::EngineeringUnits;

    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(77, "AI-77", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(85.0); // above high_limit 80 → HIGH_LIMIT
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee = EventEnrollmentObject::new(77, "EE-77", EventType::OUT_OF_RANGE).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(EventTransitionBits::all());
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    // 1) Evaluation drives a transition and persists the new Event_State.
    let transitions = evaluate_event_enrollments(&mut db, 1);
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
