//! Foreign-`Event_State` recovery (PR-#290 review blocker 2) and the
//! zero-padded CHANGE_OF_BITSTRING comparison width (review F2).
//!
//! Rewriting `Event_Parameters` to a DIFFERENT algorithm can leave the
//! enrollment holding a state the new algorithm's conditions never name
//! (13.3's per-algorithm `pCurrentState` letters): a HIGH_LIMIT left by
//! OUT_OF_RANGE, say, under new CHANGE_OF_STATE parameters whose conditions
//! only speak of NORMAL/OFFNORMAL. Keyed strictly, no condition ever
//! matches and the ghost state persists forever — a regression against the
//! stateless base evaluators, which computed unconditionally. Each arm now
//! recovers per [`ArmEvaluation`]'s documented rule: evaluate as from
//! NORMAL and indicate the computed state, through the ordinary actions
//! path INCLUDING the direction rule's delay gating.
//!
//! The monitored property's datatype must satisfy the new algorithm for a
//! recovery to be observable at all, so the fixtures monitor a property
//! every arm can read (`NOTIFICATION_CLASS`, an Unsigned readable by
//! extract_real and extract_enumerated alike; a second object's
//! `EVENT_ENABLE` bitstring for the COBS case).

use super::super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
    ChangeOfValueCriteria,
};

/// EE (instance 1) monitoring `AI-1.NOTIFICATION_CLASS` (an Unsigned), with
/// the given event type + parameters.
fn setup_on_notification_class(
    event_type: EventType,
    params: BACnetEventParameter,
    notification_class: u32,
) -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.write_property(
        PropertyIdentifier::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(notification_class as u64),
        None,
    )
    .unwrap();
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee = EventEnrollmentObject::new(1, "EE-1", event_type.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::NOTIFICATION_CLASS.to_raw(),
    )));
    ee.set_event_parameters(params);
    ee.set_event_enable(0x07);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();
    (db, ee_oid)
}

/// Rewrite `Event_Parameters` framed, the network-faithful way.
fn rewrite_params(
    db: &mut ObjectDatabase,
    ee_oid: &ObjectIdentifier,
    params: BACnetEventParameter,
) {
    let mut scratch = EventEnrollmentObject::new(1, "scratch", 0).unwrap();
    scratch.set_event_parameters(params);
    let reframed = scratch
        .read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
        .unwrap();
    db.get_mut(ee_oid)
        .unwrap()
        .write_property(PropertyIdentifier::EVENT_PARAMETERS, None, reframed, None)
        .unwrap();
}

fn event_state(db: &ObjectDatabase, ee_oid: &ObjectIdentifier) -> EventState {
    match db
        .get(ee_oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap()
    {
        PropertyValue::Enumerated(v) => EventState::from_raw(v),
        other => panic!("EVENT_STATE must read Enumerated, got {other:?}"),
    }
}

/// HIGH_LIMIT left by OUT_OF_RANGE under rewritten CHANGE_OF_STATE
/// parameters: the value is in NO alarm list, so the algorithm settles at
/// NORMAL — reached through the actions path (state stored), gated by the
/// normal-direction delay (here Time_Delay=2 via the absent-TDN fallback),
/// NOT by an unwedging dodge.
#[test]
fn foreign_high_limit_recovers_under_cos_params() {
    let (mut db, ee_oid) = setup_on_notification_class(
        EventType::OUT_OF_RANGE,
        BACnetEventParameter::OutOfRange {
            time_delay: 0,
            low_limit: 20.0,
            high_limit: 80.0,
            deadband: 2.0,
        },
        90, // unreadable as a bool — monitored as Unsigned(90) > 80 -> HIGH_LIMIT
    );
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::HIGH_LIMIT,
        "OOR drives the enrollment to HIGH_LIMIT first"
    );

    rewrite_params(
        &mut db,
        &ee_oid,
        BACnetEventParameter::ChangeOfState {
            time_delay: 2,
            list_of_values: vec![BACnetPropertyStates::UnsignedValue(999)],
        },
    );
    // HIGH_LIMIT is outside COS's {NORMAL, OFFNORMAL}: recovery is
    // indicated, gated by pTimeDelayNormal(=TD fallback)=2.
    for pass in 1..=2 {
        assert!(
            evaluate_event_enrollments(&mut db, 1).is_empty(),
            "recovery pass {pass}: gated by the normal-direction delay"
        );
        assert_eq!(event_state(&db, &ee_oid), EventState::HIGH_LIMIT);
    }
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::HIGH_LIMIT);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);
    // The transition is labeled by the enrollment's own EVENT_TYPE
    // (constructor-configured, OUT_OF_RANGE here) — the evaluator dispatches
    // on the params tag but reports the enrollment's declared identity.
    assert_eq!(transitions[0].event_type, EventType::OUT_OF_RANGE);
    assert_eq!(event_state(&db, &ee_oid), EventState::NORMAL);

    // And the arm is alive afterwards from inside its own state space.
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
}

/// OFFNORMAL left by CHANGE_OF_STATE under rewritten OUT_OF_RANGE
/// parameters, value in band: recovers to NORMAL immediately (TD=0).
#[test]
fn foreign_offnormal_recovers_under_oor_params() {
    let (mut db, ee_oid) = setup_on_notification_class(
        EventType::CHANGE_OF_STATE,
        BACnetEventParameter::ChangeOfState {
            time_delay: 0,
            list_of_values: vec![BACnetPropertyStates::UnsignedValue(1)],
        },
        1,
    );
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::OFFNORMAL
    );

    // low_limit 0 keeps the monitored value (1) in the OOR band.
    rewrite_params(
        &mut db,
        &ee_oid,
        BACnetEventParameter::OutOfRange {
            time_delay: 0,
            low_limit: 0.0,
            high_limit: 80.0,
            deadband: 2.0,
        },
    );
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::OFFNORMAL);
    assert_eq!(
        transitions[0].change.to,
        EventState::NORMAL,
        "in-band value under the new OOR params settles NORMAL"
    );
    // Labeled by the enrollment's configured EVENT_TYPE (CHANGE_OF_STATE) —
    // the params tag drives dispatch, the property drives identity.
    assert_eq!(transitions[0].event_type, EventType::CHANGE_OF_STATE);
    assert_eq!(event_state(&db, &ee_oid), EventState::NORMAL);
}

/// OFFNORMAL under a COV enrollment's increment criterion: Figure 13-10's
/// only arrow is ToNormal — the recovery indicates NORMAL and installs the
/// current sample as the detection baseline (13.3.3's rule), afterwards
/// unchanged.
#[test]
fn foreign_offnormal_under_cov_recovers_and_establishes_baseline() {
    let (mut db, ee_oid) = setup_on_notification_class(
        EventType::CHANGE_OF_STATE,
        BACnetEventParameter::ChangeOfState {
            time_delay: 0,
            list_of_values: vec![BACnetPropertyStates::UnsignedValue(3)],
        },
        3,
    );
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::OFFNORMAL
    );

    rewrite_params(
        &mut db,
        &ee_oid,
        BACnetEventParameter::ChangeOfValue {
            time_delay: 0,
            criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
        },
    );
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(
        transitions.len(),
        1,
        "COV's only target is NORMAL: the foreign state recovers"
    );
    assert_eq!(transitions[0].change.from, EventState::OFFNORMAL);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);
    assert_eq!(event_state(&db, &ee_oid), EventState::NORMAL);

    // Baseline installed at the recovered value (3.0): a +6 move crosses the
    // increment against IT and indicates NORMAL->NORMAL.
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    db.get_mut(&ai_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(9),
            None,
        )
        .unwrap();
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);
}

/// HIGH_LIMIT under COBS parameters (recovery to NORMAL because the alarm
/// pattern does not match). The prior state is seeded through the internal
/// channel — the only in-tree route that leaves HIGH_LIMIT on a COBS
/// enrollment, since no algorithm combination produces it there (bitstring
/// monitored property, no crosswise-readable predecessor).
#[test]
fn foreign_high_limit_recovers_under_cobs_params() {
    let mut db = ObjectDatabase::new();

    // Target exposing a bitstring property: EVENT_ENABLE = internal 0x07 →
    // wire 0xE0.
    let mut target = EventEnrollmentObject::new(96, "Target", EventType::NONE.to_raw()).unwrap();
    target.set_event_enable(0x07);
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(97, "EE-cobs", EventType::CHANGE_OF_BITSTRING.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        target_oid,
        PropertyIdentifier::EVENT_ENABLE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::ChangeOfBitstring {
        time_delay: 0,
        bitmask: (5, vec![0x80]), // significant: TO_OFFNORMAL bit
        list_of_values: vec![(5, vec![0x00])], // alarm when that bit CLEAR
    });
    ee.set_event_enable(0x07);
    // Internal channel: simulate the state an OOR predecessor would have left.
    ee.set_event_state_internal(EventState::HIGH_LIMIT).unwrap();
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1, "COBS recovers the foreign HIGH_LIMIT");
    assert_eq!(transitions[0].change.from, EventState::HIGH_LIMIT);
    assert_eq!(
        transitions[0].change.to,
        EventState::NORMAL,
        "bit set while the alarm pattern wants it clear -> no match -> settles NORMAL"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::NORMAL);
}

/// Zero-padded comparison width (review F2): mask [FF FF], alarm [00 01],
/// monitored [00] — the alarm's second significant byte never observed — is
/// NOT a match ("equals a listed alarm value" over the whole width). The
/// truncating comparison reported OFFNORMAL on the shared first byte.
#[test]
fn cobs_mask_wider_than_monitored_value_is_not_a_match() {
    let mut db = ObjectDatabase::new();

    let mut ee =
        EventEnrollmentObject::new(97, "EE-cobs-w", EventType::CHANGE_OF_BITSTRING.to_raw())
            .unwrap();
    // Monitor a 1-byte bitstring (EVENT_ENABLE of a target with internal
    // 0x00 → wire 0x00).
    let mut target = EventEnrollmentObject::new(96, "Target", EventType::NONE.to_raw()).unwrap();
    target.set_event_enable(0x00);
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        target_oid,
        PropertyIdentifier::EVENT_ENABLE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::ChangeOfBitstring {
        time_delay: 0,
        bitmask: (0, vec![0xFF, 0xFF]), // two significant BYTES
        list_of_values: vec![(0, vec![0x00, 0x01])], // alarm: second byte's low bit set
    });
    ee.set_event_enable(0x07);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    // Monitored [0x00] zero-pads to [0x00, 0x00]; the alarm wants
    // [0x00, 0x01] under mask [0xFF, 0xFF] -> byte 1 disagrees -> NORMAL.
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "the alarm's second byte disagrees with the (zero-filled) monitored width"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::NORMAL);
}

/// Foreign-state recovery respects OUT_OF_SERVICE like any other pass (the
/// gate runs before evaluation).
#[test]
fn out_of_service_skips_evaluation_even_for_a_wedged_state() {
    let (mut db, ee_oid) = setup_on_notification_class(
        EventType::CHANGE_OF_STATE,
        BACnetEventParameter::ChangeOfState {
            time_delay: 0,
            list_of_values: vec![BACnetPropertyStates::UnsignedValue(1)],
        },
        1,
    );
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::OFFNORMAL
    );
    db.get_mut(&ee_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    rewrite_params(
        &mut db,
        &ee_oid,
        BACnetEventParameter::OutOfRange {
            time_delay: 0,
            low_limit: 20.0,
            high_limit: 80.0,
            deadband: 2.0,
        },
    );
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "OOS gate precedes evaluation — no recovery while out of service"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::OFFNORMAL);
}

/// FLOATING_LIMIT's reachable set is the same {NORMAL, HIGH_LIMIT,
/// LOW_LIMIT} triple, and its normalization is shared with OUT_OF_RANGE —
/// pin it independently anyway: a HIGH_LIMIT left under FL parameters whose
/// band (setpoint ± diffs) CONTAINS the value recovers to NORMAL.
#[test]
fn foreign_high_limit_recovers_under_floating_limit_params() {
    // Start OOR-typed so the wedge state (HIGH_LIMIT) is produced
    // organically: NC=90 > 80.
    let (mut db, ee_oid) = setup_on_notification_class(
        EventType::OUT_OF_RANGE,
        BACnetEventParameter::OutOfRange {
            time_delay: 0,
            low_limit: 20.0,
            high_limit: 80.0,
            deadband: 2.0,
        },
        90,
    );
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::HIGH_LIMIT
    );

    // Rewrite to FLOATING_LIMIT with the setpoint reference pointing at the
    // monitored object's own PRESENT_VALUE (50.0): band = 50 ± (high_diff 60,
    // low_diff 60) = [-10, 110] contains NC=90 -> settles NORMAL. TD=0 keeps
    // the recovery immediate per the test-speed convention.
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    db.get_mut(&ai_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    db.get_mut(&ai_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            None,
        )
        .unwrap();
    rewrite_params(
        &mut db,
        &ee_oid,
        BACnetEventParameter::FloatingLimit {
            time_delay: 0,
            setpoint_reference: BACnetDeviceObjectPropertyReference::new_local(
                ai_oid,
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            ),
            low_diff_limit: 60.0,
            high_diff_limit: 60.0,
            deadband: 0.5,
        },
    );

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::HIGH_LIMIT);
    assert_eq!(
        transitions[0].change.to,
        EventState::NORMAL,
        "FL from a foreign HIGH_LIMIT: value inside the band -> NORMAL"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::NORMAL);
}

/// Foreign state AND the monitored value IS in the alarm list: the COS
/// arm's foreign-recovery branch indicates OFFNORMAL — through the actions
/// path (Event_State stored as OFFNORMAL, delay gated by Time_Delay since
/// the target is offnormal) — rather than sitting silent. This pins the
/// `matched` half of the branch the other recovery tests leave unlit.
#[test]
fn foreign_high_limit_with_matching_alarm_indicates_offnormal() {
    // Produce the foreign state organically: OOR-typed, NC=90 > high_limit.
    let (mut db, ee_oid) = setup_on_notification_class(
        EventType::OUT_OF_RANGE,
        BACnetEventParameter::OutOfRange {
            time_delay: 0,
            low_limit: 20.0,
            high_limit: 80.0,
            deadband: 2.0,
        },
        90,
    );
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::HIGH_LIMIT
    );

    // Rewrite to COS whose alarm list CONTAINS the monitored value (90).
    // TD=2: the offnormal-direction delay gates the recovery.
    rewrite_params(
        &mut db,
        &ee_oid,
        BACnetEventParameter::ChangeOfState {
            time_delay: 2,
            list_of_values: vec![BACnetPropertyStates::UnsignedValue(90)],
        },
    );
    for pass in 1..=2 {
        assert!(
            evaluate_event_enrollments(&mut db, 1).is_empty(),
            "recovery-to-OFFNORMAL pass {pass}: gated by pTimeDelay"
        );
        assert_eq!(
            event_state(&db, &ee_oid),
            EventState::HIGH_LIMIT,
            "the ghost state persists until the delay elapses"
        );
    }
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::HIGH_LIMIT);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
    assert_eq!(
        event_state(&db, &ee_oid),
        EventState::OFFNORMAL,
        "the SPECIFIC indicated state is stored (13.2.2.1.4), not left at the ghost"
    );
}
