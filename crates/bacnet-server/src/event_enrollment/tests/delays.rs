//! Time_Delay / Time_Delay_Normal honoring in the Event Enrollment evaluator
//! (#163; ASHRAE 135-2020 Clauses 13.2.4, 13.3).
//!
//! Delays are SECONDS in the standard; the pending countdown stores passes
//! via `ceil(delay_secs / interval_secs)` (never-fire-early). Most tests
//! here pass `interval_secs = 1`, where the conversion is the identity and
//! "N seconds" == "N passes" — each call to `evaluate_event_enrollments`
//! is then one evaluator pass (see the delay-model note in `mod.rs`, and
//! the lifecycle tests in `server/event_enrollment_task_tests.rs` proving
//! the spawned task drives these passes on `event_enrollment_interval_secs`
//! in wall time). Semantics mirror the intrinsic detectors' probe/tick
//! (#120/#225):
//!
//! - the indication-conditioned transition waits the seeded passes, firing
//!   when the countdown reaches zero;
//! - a reverted condition cancels the countdown without firing;
//! - a redundant qualifying observation never re-seeds the countdown;
//! - a changed target re-seeds with the new target's direction delay;
//! - a parameter (or monitored-reference) change mid-pending cancels the
//!   countdown and re-gates from the current configuration — and the
//!   cancellation is *persisted*, not just staged for a later write-back.

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
        let transitions = evaluate_event_enrollments(&mut db, 1);
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
    let transitions = evaluate_event_enrollments(&mut db, 1);
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
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);

    // Recovery (78 is the deadband boundary; 77 crosses it): gated by TDN=3.
    set_monitored(&mut db, &ai_oid, 77.0);
    for pass in 1..=3 {
        assert!(
            evaluate_event_enrollments(&mut db, 1).is_empty(),
            "recovery pass {pass}: TDN countdown must still be running"
        );
        assert_eq!(event_state(&db, &ee_oid), EventState::HIGH_LIMIT);
    }
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1, "TDN elapsed on recovery pass 4");
    assert_eq!(transitions[0].change.to, EventState::NORMAL);

    // And the asymmetry persists: a fresh excursion is still immediate.
    set_monitored(&mut db, &ai_oid, 90.0);
    let transitions = evaluate_event_enrollments(&mut db, 1);
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
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::HIGH_LIMIT
    );

    set_monitored(&mut db, &ai_oid, 50.0);
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "TDN absent: recovery waits the SAME Time_Delay (fallback)"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::HIGH_LIMIT);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(
        transitions.is_empty(),
        "second recovery pass: still counting (fallback TD=2, not immediate)"
    );
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
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
            evaluate_event_enrollments(&mut db, 1).is_empty(),
            "offnormal pass {pass}: TD=5 must still be counting"
        );
    }
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::HIGH_LIMIT
    );

    set_monitored(&mut db, &ai_oid, 50.0);
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "recovery pass 1: TDN=1 seeded, not yet fired"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::HIGH_LIMIT);
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
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
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // seed(3)
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // -> 2

    // Revert into the band before the delay elapses.
    set_monitored(&mut db, &ai_oid, 50.0);
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "reverted condition cancels without firing"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::NORMAL);

    // A fresh excursion must wait the FULL delay again: if the cancelled
    // countdown had been resumed, firing would arrive a pass early.
    set_monitored(&mut db, &ai_oid, 85.0);
    for pass in 1..=3 {
        assert!(
            evaluate_event_enrollments(&mut db, 1).is_empty(),
            "fresh excursion pass {pass}: full delay must re-run"
        );
    }
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
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
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // seed HIGH_LIMIT(3)
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // -> 2

    // Value crashes below the LOW limit: condition (b)'s indication has a
    // different target; the HIGH_LIMIT countdown must not fire.
    set_monitored(&mut db, &ai_oid, 15.0);
    for pass in 1..=3 {
        assert!(
            evaluate_event_enrollments(&mut db, 1).is_empty(),
            "post-re-target pass {pass}: the new LOW_LIMIT condition counts from 3"
        );
        assert_eq!(event_state(&db, &ee_oid), EventState::NORMAL);
    }
    let transitions = evaluate_event_enrollments(&mut db, 1);
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

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // seed(2)
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // -> 1

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
            evaluate_event_enrollments(&mut db, 1).is_empty(),
            "regated pass {pass}: cancelled+re-seeded countdown must not fire early"
        );
    }
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
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
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::HIGH_LIMIT,
        "TD=0: offnormal direction immediate"
    );

    set_monitored(&mut db, &ai_oid, 50.0);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // TDN seed(3)
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // -> 2

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
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "TDN change cancels the old countdown and re-seeds at the new delay"
    );
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
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
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
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
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "recovery seeded, not fired"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::OFFNORMAL);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::NORMAL
    );
}

// ---- seconds → passes conversion (PR-#290 review blocker 1) ----

/// `passes_for_delay` is the wall-clock anchor of the whole delay model:
/// ceiling division, never-fire-early, saturating.
#[test]
fn passes_for_delay_ceiling_semantics() {
    assert_eq!(super::super::passes_for_delay(5, 10), 1);
    assert_eq!(super::super::passes_for_delay(10, 10), 1);
    assert_eq!(super::super::passes_for_delay(15, 10), 2);
    assert_eq!(super::super::passes_for_delay(25, 10), 3);
    assert_eq!(
        super::super::passes_for_delay(1, 3600),
        1,
        "never fires early"
    );
    assert_eq!(
        super::super::passes_for_delay(u32::MAX, 1),
        u32::MAX,
        "huge delays saturate rather than wrap"
    );
}

/// The conversion driven end to end: with a ten-second evaluation interval
/// (the server default), Time_Delay=25 gates three passes of the out-of-range
/// condition — under the reviewed-before per-pass misreading it would have
/// gated twenty-five.
#[test]
fn delay_seconds_convert_to_passes_at_tick_boundaries() {
    let (mut db, ee_oid, _ai_oid) = setup_oor(85.0, 80.0, 20.0, 2.0, 25, None);
    for pass in 1..=3 {
        assert!(
            evaluate_event_enrollments(&mut db, 10).is_empty(),
            "interval=10s, TD=25s: pass {pass} of ceil(25/10)= normalized must not fire"
        );
    }
    assert_eq!(
        evaluate_event_enrollments(&mut db, 10)[0].change.to,
        EventState::HIGH_LIMIT,
        "the fourth pass (>= ceil boundary) fires"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::HIGH_LIMIT);
}

/// The cancellation of a pending countdown is PERSISTED on the pass that
/// observes the parameter change, even when nothing else about the pass can
/// complete (here: the monitored object has left the database). A params
/// round-trip A→B→A must therefore re-gate the full delay — never resume the
/// stale A countdown.
#[test]
fn params_round_trip_does_not_resume_stale_countdown() {
    let (mut db, ee_oid, ai_oid) = setup_oor(85.0, 80.0, 20.0, 2.0, 2, None);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // A: seed(2)
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // A: -> 1

    // Params A->B (different limits, same delay) while the monitored object
    // is GONE: the pass cannot complete — but the cancellation must stick.
    set_oor_params(&mut db, &ee_oid, 3, 21.0, 81.0);
    let removed = db.remove(&ai_oid).expect("fixture AI present");
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());

    // Params B->A, monitored object restored. The re-gated countdown must
    // run its FULL span: seed(2) now, fire on the THIRD pass from here. Had
    // the cancellation been lost, the stale A countdown (remaining=1) would
    // fire one pass early.
    set_oor_params(&mut db, &ee_oid, 2, 20.0, 80.0);
    db.add(removed).unwrap();
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "re-gate seeds a fresh countdown"
    );
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "full delay re-runs: second pass must not fire"
    );
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::HIGH_LIMIT,
        "fires only after the re-gated delay fully elapses"
    );
    let _ = ee_oid;
}

/// Rewrite the EE's Event_Parameters (framed, as a config client would).
fn set_oor_params(
    db: &mut ObjectDatabase,
    ee_oid: &ObjectIdentifier,
    td: u32,
    low: f32,
    high: f32,
) {
    let mut scratch = EventEnrollmentObject::new(1, "scratch", 0).unwrap();
    scratch.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: td,
        low_limit: low,
        high_limit: high,
        deadband: 2.0,
    });
    let reframed = scratch
        .read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
        .unwrap();
    db.get_mut(ee_oid)
        .unwrap()
        .write_property(PropertyIdentifier::EVENT_PARAMETERS, None, reframed, None)
        .unwrap();
}

/// The monitored object, property, and array index are folded into the
/// pending-condition fingerprint.
#[test]
fn fingerprint_covers_monitored_reference() {
    let params = BACnetEventParameter::OutOfRange {
        time_delay: 2,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    };
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let ai2 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
    let pv = PropertyIdentifier::PRESENT_VALUE;
    let cf = PropertyIdentifier::COV_INCREMENT;
    let monitored =
        |oid, property, index| super::super::MonitoredReference::local(oid, property, index);
    let base = super::super::params_fingerprint(
        &params,
        2,
        EventType::OUT_OF_RANGE.to_raw(),
        &monitored(ai1, pv, None),
    );
    assert_ne!(
        base,
        super::super::params_fingerprint(
            &params,
            2,
            EventType::OUT_OF_RANGE.to_raw(),
            &monitored(ai2, pv, None),
        ),
        "different monitored object must fingerprint differently"
    );
    assert_ne!(
        base,
        super::super::params_fingerprint(
            &params,
            2,
            EventType::OUT_OF_RANGE.to_raw(),
            &monitored(ai1, cf, None),
        ),
        "different monitored property must fingerprint differently"
    );
    assert_ne!(
        base,
        super::super::params_fingerprint(
            &params,
            2,
            EventType::OUT_OF_RANGE.to_raw(),
            &monitored(ai1, pv, Some(0)),
        ),
        "an omitted index must differ from index zero"
    );
    assert_ne!(
        super::super::params_fingerprint(
            &params,
            2,
            EventType::OUT_OF_RANGE.to_raw(),
            &monitored(ai1, pv, Some(0)),
        ),
        super::super::params_fingerprint(
            &params,
            2,
            EventType::OUT_OF_RANGE.to_raw(),
            &monitored(ai1, pv, Some(1)),
        ),
        "different array indexes must fingerprint differently"
    );
    assert_eq!(
        base,
        super::super::params_fingerprint(
            &params,
            2,
            EventType::OUT_OF_RANGE.to_raw(),
            &monitored(ai1, pv, None),
        ),
        "same configuration fingerprints stably"
    );
}

/// Retarget behaviorally: the pending countdown seeded while monitoring
/// object A does not survive a retarget to object B — it cancels and
/// re-gates. The stock EE object exposes no network/property route that
/// mutates `Object_Property_Reference` (Table 12-14 codes it R), so the
/// retarget is driven by transplanting the evaluation state into a
/// re-created enrollment through the internal channel — exactly the
/// situation a retarget would leave behind if one existed.
#[test]
fn retarget_mid_pending_cancels_and_regates() {
    let (mut db, ee_oid, ai_oid) = setup_oor(85.0, 80.0, 20.0, 2.0, 3, None);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // seed(3)
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // -> 2

    // Transplant: out-of-range second object + a fresh EE instance 1
    // targeting it, carrying the ORIGINAL pending countdown.
    let pending = db
        .get(&ee_oid)
        .unwrap()
        .enrollment_eval_state_internal()
        .unwrap()
        .pending
        .expect("countdown seeded");
    let mut ai2 = AnalogInputObject::new(2, "AI-2", 62).unwrap();
    ai2.set_present_value(86.0);
    let ai2_oid = ai2.object_identifier();
    db.add(Box::new(ai2)).unwrap();
    db.remove(&ee_oid);
    let mut ee = EventEnrollmentObject::new(1, "EE-OOR", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai2_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 3,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();
    // Inject the transplanted countdown as-is (fingerprint still names the
    // OLD monitored reference — the mismatch the evaluator must cancel).
    db.get_mut(&ee_oid)
        .unwrap()
        .set_enrollment_eval_state_internal(
            bacnet_objects::event_enrollment::EventEnrollmentEvalState {
                pending: Some(pending),
                ..Default::default()
            },
        )
        .unwrap();

    // The retargeted countdown regates the FULL delay: cancel+seed(3) on the
    // first pass, fire on the fourth. Resuming the stale one (remaining=2)
    // would fire one pass too early.
    for pass in 1..=3 {
        assert!(
            evaluate_event_enrollments(&mut db, 1).is_empty(),
            "post-retarget pass {pass}: full delay must re-run"
        );
    }
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::HIGH_LIMIT
    );
    let _ = ai_oid;
}
