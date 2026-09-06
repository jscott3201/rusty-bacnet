//! `Event_Detection_Enable` on EventEnrollment (ASHRAE 135-2020 Clause 12.12,
//! Clause 13.2.2.1).
//!
//! Split out to keep every file under the 700-LOC cap.

use super::super::*;

/// Clause 12.12 codes `Event_Detection_Enable` R, so the property must be
/// present and readable on every Event Enrollment.
///
/// The *value* TRUE is a project choice, not a spec requirement: 135-2020
/// specifies no default for this property anywhere, and Clause 15.3's CreateObject Service Procedure makes
/// the initial values of properties not named in a CreateObject request "a
/// local matter". TRUE is chosen because it preserves the always-detecting
/// behavior this object had before the property existed, and matches
/// `AlertEnrollmentObject`.
#[test]
fn event_detection_enable_defaults_true() {
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
}

#[test]
fn event_detection_enable_appears_in_property_list() {
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    assert!(
        ee.property_list()
            .contains(&PropertyIdentifier::EVENT_DETECTION_ENABLE),
        "a required property must be advertised, or PICS under-reports it"
    );
}

#[test]
fn event_detection_enable_is_writable() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
}

#[test]
fn event_detection_enable_rejects_wrong_type() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    assert!(
        ee.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Unsigned(0),
            None,
        )
        .is_err(),
        "a BOOLEAN property must reject a non-Boolean write"
    );
    // The rejected write must not have taken effect.
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
}

/// The core Clause 13.2.2.1 requirement: "If the Event_Detection_Enable
/// property is FALSE, then this state machine is not evaluated. In this case,
/// no transitions shall occur, Event_State shall be set to NORMAL".
///
/// Clause 12.12 states the same as an invariant rather than an action —
/// "When this property is FALSE, Event_State shall be NORMAL" — which is why
/// the reset happens on the write and not on some later evaluation pass.
#[test]
fn disabling_detection_resets_event_state_to_normal() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.set_event_state(EventState::HIGH_LIMIT.to_raw());
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()),
        "precondition: the enrollment is in an alarm state"
    );

    ee.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();

    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        "disabling detection must reset Event_State, not freeze it"
    );
}

/// `Acked_Transitions` must read its initial condition — every flag TRUE,
/// "if no event of that type has ever occurred" (Clause 12.12) — while
/// detection is disabled.
///
/// Weaker than it looks, and deliberately so: nothing on this object ever
/// clears `Acked_Transitions` (that is the Clause 13.2.3 alarm-acknowledgment
/// process, part of #123), so this would also pass without the reset. It is
/// here to pin the *encoding* of the initial condition, so that when #123 adds
/// a mutator the expected post-reset value is already written down.
#[test]
fn disabled_detection_reports_initial_acked_transitions() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();

    assert_eq!(
        ee.read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            // All three transition flags TRUE, MSB-first.
            data: vec![0xE0],
        }
    );
}

/// Clause 13.2.2.1's "no transitions shall occur" is enforced at the one place
/// that can change `Event_State`, so a caller that forgets to check the flag
/// cannot violate the invariant.
#[test]
fn disabled_detection_refuses_non_normal_internal_state() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();

    assert!(
        ee.set_event_state_internal(EventState::HIGH_LIMIT).is_err(),
        "no transition may occur while detection is disabled"
    );
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        "the refused write must leave Event_State at NORMAL"
    );
}

/// The guard is scoped to non-NORMAL states, not a blanket refusal — NORMAL is
/// the value the disabled condition requires, so setting it must stay legal.
#[test]
fn disabled_detection_still_permits_normal_internal_state() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();

    assert!(ee.set_event_state_internal(EventState::NORMAL).is_ok());
}

/// Re-enabling detection must not resurrect the pre-disable state. The reset
/// already happened, so evaluation resumes from NORMAL and the next transition
/// is derived afresh from the monitored value.
#[test]
fn re_enabling_detection_resumes_from_normal() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.set_event_state(EventState::LOW_LIMIT.to_raw());

    for enabled in [false, true] {
        ee.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(enabled),
            None,
        )
        .unwrap();
    }

    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        "re-enabling must not restore LOW_LIMIT"
    );
    // And detection works again.
    assert!(ee.set_event_state_internal(EventState::LOW_LIMIT).is_ok());
}

/// The `set_event_state` seeder honors the same invariant.
///
/// It is `pub` and bypasses both `write_property` and
/// `set_event_state_internal`, so without its own guard it would be a public
/// way around a rule the rest of the object enforces — the invariant would hold
/// only by convention, and the claim that it holds by construction would be
/// false.
#[test]
fn disabled_detection_ignores_non_normal_seed() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();

    ee.set_event_state(EventState::HIGH_LIMIT.to_raw());

    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        "a non-NORMAL seed must not take effect while detection is disabled"
    );
}

/// With detection enabled the seeder works normally — the guard is scoped, not
/// a blanket disable of the setter.
#[test]
fn enabled_detection_accepts_seed() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.set_event_state(EventState::HIGH_LIMIT.to_raw());
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
}
