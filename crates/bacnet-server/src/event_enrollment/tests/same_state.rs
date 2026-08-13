//! Same-state transition actions (#166; ASHRAE 135-2020 Clause 13.2.2.1.4).
//!
//! "The actions are the same for all transitions and they shall be executed
//! even if the transition does not change the event state (e.g., a transition
//! from the OFFNORMAL event state to the OFFNORMAL event state)." The pre-#166
//! evaluator dropped every evaluation whose result equaled the current state;
//! these tests pin the indication-driven replacement:
//!
//! - a genuine same-state indication (CHANGE_OF_STATE condition (c),
//!   CHANGE_OF_VALUE's only transition kind) *executes the actions*: the
//!   specific state is stored, the `Acked_Transitions` bit is maintained per
//!   the referenced Notification Class's `Ack_Required` (Clause 13.2.3), and
//!   the transition is emitted with its `Event_Enable`-scoped `distribute`;
//! - a *persisting* condition that satisfies no algorithm condition
//!   (OUT_OF_RANGE sitting in HIGH_LIMIT, COS sitting at the SAME alarm
//!   value) still emits nothing (Clause 13.3's "no condition evaluates to
//!   true → no transition").
//!
//! CHANGE_OF_VALUE's same-state coverage lives in `change_of_value.rs`.

use super::super::*;
use super::*;
use bacnet_objects::binary::BinaryInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
};

/// CHANGE_OF_STATE fixture: a BinaryInput monitored against `alarm_values`,
/// with `time_delay` 0 unless overridden.
fn setup_cos(
    present_value: u32,
    alarm_values: &[u32],
    time_delay: u32,
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
        time_delay,
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

/// Move the monitored binary value (input writes need Out_Of_Service, as in
/// the delays suite).
fn set_monitored(db: &mut ObjectDatabase, bi_oid: &ObjectIdentifier, value: u32) {
    let bi = db.get_mut(bi_oid).unwrap();
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
        PropertyValue::Enumerated(value),
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

fn acked_transitions(db: &ObjectDatabase, ee_oid: &ObjectIdentifier) -> u8 {
    match db
        .get(ee_oid)
        .unwrap()
        .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
        .unwrap()
    {
        PropertyValue::BitString { data, .. } => bacnet_types::bitstring::unpack_octet(&data, 3),
        other => panic!("ACKED_TRANSITIONS must read BitString, got {other:?}"),
    }
}

/// Clause 13.3.2 condition (c) — "Optional: ... equal to one of the values
/// contained in pAlarmValues that is DIFFERENT from the value that caused the
/// last transition to OFFNORMAL ... indicate a transition to the OFFNORMAL
/// event state" — implemented so the Clause 13.2.2.1.4 actions execute for
/// the OFFNORMAL→OFFNORMAL same-state transition: the transition is emitted
/// and `Event_State` stores the specific state.
#[test]
fn cos_moving_between_alarm_values_reindicates_offnormal() {
    let (mut db, ee_oid, _bi_oid) = setup_cos(1, &[1, 0], 0);

    // Value 1 is an alarm value: NORMAL -> OFFNORMAL (condition (a)).
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);

    // Holding at the SAME alarm value satisfies no condition: silence.
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "value unchanged at the causing alarm value: no condition true, no transition"
    );

    // Moving to a DIFFERENT alarm value re-indicates OFFNORMAL (condition (c)).
    set_monitored(&mut db, &_bi_oid, 0);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(
        transitions.len(),
        1,
        "condition (c): OFFNORMAL -> OFFNORMAL must be emitted"
    );
    assert_eq!(transitions[0].change.from, EventState::OFFNORMAL);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
    assert_eq!(transitions[0].event_type, EventType::CHANGE_OF_STATE);
    assert_eq!(
        event_state(&db, &ee_oid),
        EventState::OFFNORMAL,
        "the specific state is stored, unchanged in value"
    );

    // And the new causing value silences further passes until the value moves
    // again.
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "no re-fire while holding the NEW causing value"
    );
    set_monitored(&mut db, &_bi_oid, 1);
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1).len(),
        1,
        "moving back re-indicates again (causing value tracked per fire, not per config)"
    );
}

/// Condition (c)'s "for pTimeDelay" is honored too: with a nonzero delay the
/// same-state re-indication counts down like any offnormal indication, and a
/// flip back to the ORIGINAL alarm value mid-countdown re-seeds it (the
/// condition identity is the matched value).
#[test]
fn cos_same_state_reindication_is_delayed_and_value_discriminated() {
    let (mut db, _ee_oid, bi_oid) = setup_cos(1, &[1, 0], 3);
    for _ in 0..3 {
        assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    }
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::OFFNORMAL,
        "condition (a) fires after Time_Delay"
    );

    // Move to the other alarm value: (c)'s countdown starts, does not fire
    // immediately.
    set_monitored(&mut db, &bi_oid, 0);
    for pass in 1..=2 {
        assert!(
            evaluate_event_enrollments(&mut db, 1).is_empty(),
            "(c) pass {pass}: pTimeDelay countdown must gate the same-state re-indication"
        );
    }
    // Flip back mid-countdown: re-seed (condition identity changed), still no
    // premature fire.
    set_monitored(&mut db, &bi_oid, 1);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    set_monitored(&mut db, &bi_oid, 0);
    for pass in 1..=3 {
        assert!(
            evaluate_event_enrollments(&mut db, 1).is_empty(),
            "re-seeded (c) pass {pass}"
        );
    }
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::OFFNORMAL);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
}

/// OUT_OF_RANGE has no same-state condition (Clause 13.3.6 (a)–(h) are all
/// state-changing): sitting above the high limit emits exactly one
/// transition, then silence. This is the pin for the OTHER half of the
/// 13.2.2.1.4 fix — same-state actions run when the algorithm *indicates*,
/// never as a per-poll re-fire.
#[test]
fn oor_persisting_offnormal_emits_nothing() {
    let (mut db, ee_oid, _ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
    for pass in 1..=5 {
        assert!(
            evaluate_event_enrollments(&mut db, 1).is_empty(),
            "pass {pass}: persisting HIGH_LIMIT satisfies no condition \
             (Clause 13.3 introduction) — nothing may be emitted"
        );
    }
    assert_eq!(event_state(&db, &ee_oid), EventState::HIGH_LIMIT);
}

/// Clause 13.3.6 (d): HIGH_LIMIT → LOW_LIMIT across the band stores the
/// SPECIFIC returned state (13.2.2.1.4 forbids collapsing it to OFFNORMAL).
#[test]
fn oor_across_band_stores_the_specific_state() {
    let (mut db, ee_oid, ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);

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
        PropertyValue::Real(15.0),
        None,
    )
    .unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::HIGH_LIMIT);
    assert_eq!(
        transitions[0].change.to,
        EventState::LOW_LIMIT,
        "the specific state LOW_LIMIT is stored and reported, not collapsed"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::LOW_LIMIT);
}

/// Clause 13.2.3 on a received transition: with the referenced Notification
/// Class requiring acknowledgment of TO_OFFNORMAL, the corresponding
/// `Acked_Transitions` bit is CLEARED (ack owed) when the transition fires —
/// on the same-state re-indication too, because the actions "are the same for
/// all transitions".
#[test]
fn acked_transitions_bit_clears_when_notification_class_requires_ack() {
    let (mut db, ee_oid, bi_oid) = setup_cos(1, &[1, 0], 0);

    // Reference a Notification Class (instance 7) requiring TO_OFFNORMAL ack.
    let mut nc = NotificationClass::new(7, "NC-7").unwrap();
    nc.ack_required = [true, false, false];
    db.add(Box::new(nc)).unwrap();
    db.get_mut(&ee_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(7),
            None,
        )
        .unwrap();

    assert_eq!(
        acked_transitions(&db, &ee_oid),
        0b111,
        "initial condition: no event of any type has ever occurred (Clause 12.12)"
    );

    // NORMAL -> OFFNORMAL with ack required: bit 0 clears.
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
    assert_eq!(
        acked_transitions(&db, &ee_oid),
        0b110,
        "TO_OFFNORMAL ack owed -> Acked_Transitions bit 0 cleared (13.2.3)"
    );

    // The OFFNORMAL -> OFFNORMAL re-indication is a fresh transition received:
    // the bit is cleared again (it starts cleared; the assertion that matters
    // is it does not SET).
    set_monitored(&mut db, &bi_oid, 0);
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
    assert_eq!(acked_transitions(&db, &ee_oid), 0b110);
}

/// The other half of 13.2.3's sentence: "otherwise it is set." With no
/// Notification Class object resolvable, a fired transition leaves the bit at
/// the acknowledged state; with a class that requires nothing, the same — a
/// transition is never stranded unacknowledged for want of a class object.
#[test]
fn acked_transitions_bit_sets_when_no_ack_required() {
    let (mut db, ee_oid, _bi_oid) = setup_cos(1, &[1], 0);
    // No Notification Class object exists at all.
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
    assert_eq!(
        acked_transitions(&db, &ee_oid),
        0b111,
        "no Ack_Required available -> the bit is set (13.2.3)"
    );

    // Same with a Notification Class referenced but requiring nothing: the
    // bit is set — from a CLEARED start this pass, so the set half is the
    // one proven to run (starting from 0b111 would leave the test blind to
    // a missing action, exactly the false-green pairing the sibling test
    // guards against).
    let nc = NotificationClass::new(3, "NC-3").unwrap();
    db.add(Box::new(nc)).unwrap();
    {
        let obj = db.get_mut(&ee_oid).unwrap();
        obj.write_property(
            PropertyIdentifier::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(3),
            None,
        )
        .unwrap();
        // Internal channel, staging an already-owed TO_NORMAL ack.
        obj.set_acked_transitions_internal(0x04, false).unwrap();
    }
    assert_eq!(acked_transitions(&db, &ee_oid), 0b011);
    set_monitored(&mut db, &_bi_oid, 0);
    // Value 0 is not in the alarm list [1]: OFFNORMAL -> NORMAL, TO_NORMAL
    // is not ack-required, so its bit is SET by the transition.
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
    assert_eq!(
        acked_transitions(&db, &ee_oid),
        0b111,
        "TO_NORMAL with ack not required -> bit 2 set by the transition action"
    );
}

/// A cleared `Event_Enable` bit suppresses only distribution — Clause 12.12
/// scopes the property to "enabling and disabling the distribution of
/// notifications" — never the same-state transition actions: the transition
/// is still emitted (with `distribute == false`) and `Event_State` stored.
#[test]
fn event_enable_suppresses_distribution_not_same_state_actions() {
    let (mut db, ee_oid, bi_oid) = setup_cos(1, &[1, 0], 0);
    db.get_mut(&ee_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x60], // wire bits: TO_NORMAL + TO_FAULT, NOT TO_OFFNORMAL
            },
            None,
        )
        .unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert!(
        !transitions[0].distribute,
        "TO_OFFNORMAL cleared: distribution suppressed, transition reported"
    );
    assert_eq!(event_state(&db, &ee_oid), EventState::OFFNORMAL);

    set_monitored(&mut db, &bi_oid, 0);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(
        transitions.len(),
        1,
        "condition (c)'s same-state transition is emitted even with distribution off"
    );
    assert!(!transitions[0].distribute);
    assert_eq!(transitions[0].change.from, EventState::OFFNORMAL);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
}

/// The ack maintenace is direction-complete: a recovery (OFFNORMAL -> NORMAL)
/// whose Notification Class requires TO_NORMAL acknowledgment clears the
/// TO_NORMAL bit — the evaluator does not privilege the offnormal direction.
#[test]
fn acked_transitions_to_normal_clear_with_ack_required() {
    let (mut db, ee_oid, bi_oid) = setup_cos(1, &[1], 0);

    let mut nc = NotificationClass::new(9, "NC-9").unwrap();
    nc.ack_required = [false, false, true]; // TO_NORMAL
    db.add(Box::new(nc)).unwrap();
    db.get_mut(&ee_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(9),
            None,
        )
        .unwrap();

    // NORMAL -> OFFNORMAL (not ack-required): TO_OFFNORMAL bit stays set.
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
    assert_eq!(acked_transitions(&db, &ee_oid), 0b111);

    // OFFNORMAL -> NORMAL with TO_NORMAL ack required: bit 2 clears.
    set_monitored(&mut db, &bi_oid, 0);
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
    assert_eq!(
        acked_transitions(&db, &ee_oid),
        0b011,
        "TO_NORMAL ack owed -> bit 2 cleared (13.2.3 is direction-complete)"
    );
}

/// 13.2.3 gates the bit on Ack_Required, NEVER on Event_Enable (Clause
/// 12.12 scopes Event_Enable to external distribution): a transition with
/// distribution suppressed still clears its ack-owed bit.
#[test]
fn ack_bit_maintenance_is_independent_of_event_enable() {
    let (mut db, ee_oid, _bi_oid) = setup_cos(1, &[1], 0);

    let mut nc = NotificationClass::new(11, "NC-11").unwrap();
    nc.ack_required = [true, false, false];
    db.add(Box::new(nc)).unwrap();
    {
        let obj = db.get_mut(&ee_oid).unwrap();
        obj.write_property(
            PropertyIdentifier::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(11),
            None,
        )
        .unwrap();
        obj.write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x60], // TO_FAULT + TO_NORMAL only; TO_OFFNORMAL not distributed
            },
            None,
        )
        .unwrap();
    }

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert!(
        !transitions[0].distribute,
        "TO_OFFNORMAL distribution is suppressed"
    );
    assert_eq!(
        acked_transitions(&db, &ee_oid),
        0b110,
        "...while the ack-owed bit STILL clears — Event_Enable never scopes it"
    );
}
