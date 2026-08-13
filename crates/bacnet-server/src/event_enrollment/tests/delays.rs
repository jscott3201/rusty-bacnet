//! Time_Delay / Time_Delay_Normal honoring in the Event Enrollment evaluator
//! (#163; ASHRAE 135-2020 Clauses 13.2.4, 13.3).
//!
//! Each call to `evaluate_event_enrollments` is one evaluator pass — one
//! "tick" of the pending countdown (see the delay-model note in `mod.rs`, and
//! the lifecycle test in `server/event_enrollment_task_tests.rs` proving the
//! spawned task drives these passes on `event_enrollment_interval_secs`).
//! Semantics mirror the intrinsic detectors' probe/tick (#120/#225):
//!
//! - the indication-conditioned transition waits N seeded passes, firing when
//!   the countdown reaches zero;
//! - a reverted condition cancels the countdown without firing;
//! - a redundant qualifying observation never re-seeds the countdown;
//! - a changed target re-seeds with the new target's direction delay;
//! - a parameter change mid-pending (Time_Delay, limits, Time_Delay_Normal)
//!   cancels the countdown and re-gates from the current parameters.

use super::super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
};

/// OUT_OF_RANGE fixture with configurable delays. `tdn` writes the EE
/// object's optional Time_Delay_Normal property (`None` = absent, the
/// fallback case).
fn setup_oor(
    present_value: f32,
    high_limit: f32,
    low_limit: f32,
    deadband: f32,
    time_delay: u32,
    tdn: Option<u32>,
) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.set_present_value(present_value);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee = EventEnrollmentObject::new(1, "EE-OOR", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay,
        low_limit,
        high_limit,
        deadband,
    });
    ee.set_event_enable(0x07);
    ee.set_time_delay_normal(tdn);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    (db, ee_oid, ai_oid)
}

/// Move the monitored analog value (input `Present_Value` writes require
/// `Out_Of_Service` — the route the out_of_range suite established).
fn set_monitored(db: &mut ObjectDatabase, ai_oid: &ObjectIdentifier, value: f32) {
    let ai = db.get_mut(ai_oid).unwrap();
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
        PropertyValue::Real(value),
        None,
    )
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

/// TD=3: the indication-conditioned transition waits three seeded passes and
/// fires on the fourth — and the observable `Event_State` holds at NORMAL
/// while the delay counts down (Clause 13.2.4).
///
/// Delay 3 firing exactly at pass 4 — never later, on a path where every
/// pass is a fresh qualifying observation — also pins the no-restart rule:
/// a countdown re-seeded by each redundant observation would never reach
/// zero at all.
#[test]
fn out_of_range_time_delay_gates_offnormal_transition() {
    let (mut db, ee_oid, _ai_oid) = setup_oor(85.0, 80.0, 20.0, 2.0, 3, None);

    for pass in 1..=3 {
        let transitions = evaluate_event_enrollments(&mut db);
        assert!(
            transitions.is_empty(),
            "pass {pass}: countdown must still be running, got {transitions:?}"
        );
        assert_eq!(
            event_state(&db, &ee_oid),
            EventState::NORMAL,
            "pass {pass}: observable Event_State holds at the confirmed state while delayed"
        );
    }
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1, "pass 4: the delay elapsed");
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
    assert_eq!(event_state(&db, &ee_oid), EventState::HIGH_LIMIT);
}

/// Time_Delay_Normal (TDN=3) gates the return to NORMAL while Time_Delay=0
/// leaves the offnormal direction immediate — the Clause 13.3 direction
/// asymmetry, via the EE object's own property (Table 12-14 O).
#[test]
fn time_delay_normal_gates_only_the_return_to_normal() {
    let (mut db, ee_oid, ai_oid) = setup_oor(50.0, 80.0, 20.0, 2.0, 0, Some(3));

    // Offnormal direction: TD=0 → immediate.
    set_monitored(&mut db, &ai_oid, 85.0);
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);

    // Recovery (78 is the deadband boundary; 77 crosses it): gated by TDN=3.
    set_monitored(&mut db, &ai_oid, 77.0);
    for pass in 1..=3 {
        assert!(
            evaluate_event_enrollments(&mut db).is_empty(),
            "recovery pass {pass}: TDN countdown must still be running"
        );
        assert_eq!(event_state(&db, &ee_oid), EventState::HIGH_LIMIT);
    }
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1, "TDN elapsed on recovery pass 4");
    assert_eq!(transitions[0].change.to, EventState::NORMAL);

    // And the asymmetry persists: a fresh excursion is still immediate.
    set_monitored(&mut db, &ai_oid, 90.0);
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
}

/// Absent Time_Delay_Normal, NORMAL-direction transitions wait pTimeDelay
/// (the normative fallback: "it takes on the value of the pTimeDelay
/// parameter").
#[test]
fn absent_time_delay_normal_falls_back_to_time_delay() {
    let (mut db, ee_oid, ai_oid) = setup_oor(85.0, 80.0, 20.0, 2.0, 2, None);
    // Two seeded passes on the OFFNORMAL direction, then fire.
    assert!(evaluate_event_enrollments(&mut db).is_empty());
    assert!(evaluate_event_enrollments(&mut db).is_empty());
    assert_eq!(
        evaluate_event_enrollments(&mut db)[0].change.to,
        EventState::HIGH_LIMIT
    );

    set_monitored(&mut db, &ai_oid, 50.0);
    assert!(
        evaluate_event_enrollments(&mut db).is_empty(),
        "TDN absent: recovery waits the SAME Time_Delay (fallback)"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::HIGH_LIMIT);
    let transitions = evaluate_event_enrollments(&mut db);
    assert!(
        transitions.is_empty(),
        "second recovery pass: still counting (fallback TD=2, not immediate)"
    );
    assert_eq!(
        evaluate_event_enrollments(&mut db)[0].change.to,
        EventState::NORMAL,
        "fallback delay elapsed"
    );
}

/// TD=5, TDN=1: both directions wait their own delay, not the other's.
#[test]
fn time_delay_and_time_delay_normal_are_independent() {
    let (mut db, ee_oid, ai_oid) = setup_oor(50.0, 80.0, 20.0, 2.0, 5, Some(1));

    set_monitored(&mut db, &ai_oid, 85.0);
    for pass in 1..=5 {
        assert!(
            evaluate_event_enrollments(&mut db).is_empty(),
            "offnormal pass {pass}: TD=5 must still be counting"
        );
    }
    assert_eq!(
        evaluate_event_enrollments(&mut db)[0].change.to,
        EventState::HIGH_LIMIT
    );

    set_monitored(&mut db, &ai_oid, 50.0);
    assert!(
        evaluate_event_enrollments(&mut db).is_empty(),
        "recovery pass 1: TDN=1 seeded, not yet fired"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::HIGH_LIMIT);
    assert_eq!(
        evaluate_event_enrollments(&mut db)[0].change.to,
        EventState::NORMAL,
        "recovery pass 2: TDN=1 elapsed — the offnormal delay plays no part"
    );
}

/// A condition that reverts before the delay elapses cancels the countdown:
/// nothing fires, and a later excursion starts a FULL new delay rather than
/// resuming the interrupted one.
#[test]
fn pending_cancels_when_condition_reverts() {
    let (mut db, ee_oid, ai_oid) = setup_oor(50.0, 80.0, 20.0, 2.0, 3, None);

    set_monitored(&mut db, &ai_oid, 85.0);
    assert!(evaluate_event_enrollments(&mut db).is_empty()); // seed(3)
    assert!(evaluate_event_enrollments(&mut db).is_empty()); // -> 2

    // Revert into the band before the delay elapses.
    set_monitored(&mut db, &ai_oid, 50.0);
    assert!(
        evaluate_event_enrollments(&mut db).is_empty(),
        "reverted condition cancels without firing"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::NORMAL);

    // A fresh excursion must wait the FULL delay again: if the cancelled
    // countdown had been resumed, firing would arrive a pass early.
    set_monitored(&mut db, &ai_oid, 85.0);
    for pass in 1..=3 {
        assert!(
            evaluate_event_enrollments(&mut db).is_empty(),
            "fresh excursion pass {pass}: full delay must re-run"
        );
    }
    assert_eq!(
        evaluate_event_enrollments(&mut db)[0].change.to,
        EventState::HIGH_LIMIT
    );
}

/// The target changing mid-delay (NORMAL→HIGH_LIMIT pending, then a plunge
/// straight through the band indicating NORMAL→LOW_LIMIT) re-seeds with the
/// NEW condition, not the cancelled one.
#[test]
fn condition_target_change_mid_delay_reseeds() {
    let (mut db, ee_oid, ai_oid) = setup_oor(50.0, 80.0, 20.0, 2.0, 3, None);

    set_monitored(&mut db, &ai_oid, 85.0);
    assert!(evaluate_event_enrollments(&mut db).is_empty()); // seed HIGH_LIMIT(3)
    assert!(evaluate_event_enrollments(&mut db).is_empty()); // -> 2

    // Value crashes below the LOW limit: condition (b)'s indication has a
    // different target; the HIGH_LIMIT countdown must not fire.
    set_monitored(&mut db, &ai_oid, 15.0);
    for pass in 1..=3 {
        assert!(
            evaluate_event_enrollments(&mut db).is_empty(),
            "post-re-target pass {pass}: the new LOW_LIMIT condition counts from 3"
        );
        assert_eq!(event_state(&db, &ee_oid), EventState::NORMAL);
    }
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::LOW_LIMIT);
}

/// A parameter change mid-pending (here: Time_Delay itself, observed via the
/// framed EVENT_PARAMETERS write a config client would use) cancels the
/// in-flight countdown and re-gates from the new parameters — no partial
/// countdown resumes.
#[test]
fn parameter_change_mid_pending_cancels_and_regates() {
    let (mut db, ee_oid, _ai_oid) = setup_oor(85.0, 80.0, 20.0, 2.0, 2, None);

    assert!(evaluate_event_enrollments(&mut db).is_empty()); // seed(2)
    assert!(evaluate_event_enrollments(&mut db).is_empty()); // -> 1

    // Rewrite Event_Parameters with a longer delay, as a config client's
    // framed wire write would deliver it (the write arm also accepts the
    // structured value directly; the framed path is the network-faithful one).
    let mut scratch = EventEnrollmentObject::new(1, "scratch", 0).unwrap();
    scratch.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 5,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    let reframed = scratch
        .read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
        .unwrap();
    db.get_mut(&ee_oid)
        .unwrap()
        .write_property(PropertyIdentifier::EVENT_PARAMETERS, None, reframed, None)
        .unwrap();

    // Under the OLD countdown the next pass fired. Under the regate, the
    // fresh TD=5 must hold for its full seeded span.
    for pass in 1..=5 {
        assert!(
            evaluate_event_enrollments(&mut db).is_empty(),
            "regated pass {pass}: cancelled+re-seeded countdown must not fire early"
        );
    }
    assert_eq!(
        evaluate_event_enrollments(&mut db)[0].change.to,
        EventState::HIGH_LIMIT,
        "the regated delay (5) eventually elapses"
    );
}

/// A Time_Delay_Normal *property* change mid-pending is a parameter change
/// too (it feeds pTimeDelayNormal), and regates the same way.
#[test]
fn time_delay_normal_change_mid_pending_regates() {
    let (mut db, ee_oid, ai_oid) = setup_oor(85.0, 80.0, 20.0, 2.0, 0, Some(3));
    assert_eq!(
        evaluate_event_enrollments(&mut db)[0].change.to,
        EventState::HIGH_LIMIT,
        "TD=0: offnormal direction immediate"
    );

    set_monitored(&mut db, &ai_oid, 50.0);
    assert!(evaluate_event_enrollments(&mut db).is_empty()); // TDN seed(3)
    assert!(evaluate_event_enrollments(&mut db).is_empty()); // -> 2

    db.get_mut(&ee_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::TIME_DELAY_NORMAL,
            None,
            PropertyValue::Unsigned(1),
            None,
        )
        .unwrap();

    assert!(
        evaluate_event_enrollments(&mut db).is_empty(),
        "TDN change cancels the old countdown and re-seeds at the new delay"
    );
    assert_eq!(
        evaluate_event_enrollments(&mut db)[0].change.to,
        EventState::NORMAL,
        "the regated TDN=1 delay elapses and fires"
    );
}

/// CHANGE_OF_STATE honors both directions too — the offnormal indication
/// gates on Time_Delay, the recovery on the Time_Delay fallback absent TDN
/// (13.3.2 conditions (a)/(b)).
#[test]
fn change_of_state_delays_both_directions() {
    let mut db = ObjectDatabase::new();

    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    bi.set_present_value(1);
    let bi_oid = bi.object_identifier();
    db.add(Box::new(bi)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(3, "EE-COS", EventType::CHANGE_OF_STATE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        bi_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::ChangeOfState {
        time_delay: 2,
        list_of_values: vec![BACnetPropertyStates::UnsignedValue(1)],
    });
    ee.set_event_enable(0x07);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    // Value already in the alarm list at the first pass: seed(2), no fire.
    assert!(evaluate_event_enrollments(&mut db).is_empty());
    assert!(evaluate_event_enrollments(&mut db).is_empty());
    assert_eq!(
        evaluate_event_enrollments(&mut db)[0].change.to,
        EventState::OFFNORMAL
    );

    // Recovery gated by the fallback (TD): seeded pass, then fire. (An
    // input's Present_Value accepts writes only while Out_Of_Service.)
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
    assert!(
        evaluate_event_enrollments(&mut db).is_empty(),
        "recovery seeded, not fired"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::OFFNORMAL);
    assert!(evaluate_event_enrollments(&mut db).is_empty());
    assert_eq!(
        evaluate_event_enrollments(&mut db)[0].change.to,
        EventState::NORMAL
    );
}
