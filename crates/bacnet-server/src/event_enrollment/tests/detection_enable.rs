//! `Event_Detection_Enable` gating of the Event Enrollment evaluator
//! (ASHRAE 135-2020 Clause 13.2.2.1).
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.
//!
//! What these tests pin: that a disabled enrollment produces no transition
//! and holds `Event_State` at NORMAL. Two guards serve Clause 13.2.2.1: the
//! evaluator skips a disabled enrollment, and Event Enrollment itself refuses
//! a non-NORMAL commit and any evaluation-state write while disabled. The
//! tests on a real Event Enrollment pass with either guard alone;
//! `the_evaluator_skips_an_enrollment_that_reads_disabled` isolates the
//! evaluator's, with an object whose own guards would let the pass through.

use super::super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::{BACnetObject, EnrollmentEval, EventReporting};
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
use bacnet_types::enums::EngineeringUnits;
use std::borrow::Cow;

/// A database holding an Analog Input above an enrollment's high limit, and
/// that enrollment, built by `enrollment` from the input's identifier.
fn setup_with(
    enrollment: impl FnOnce(EventEnrollmentObject) -> Box<dyn BACnetObject>,
) -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(20, "AI-20", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(85.0); // above high_limit
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee = EventEnrollmentObject::new(20, "EE-det", EventType::OUT_OF_RANGE).unwrap();
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
    db.add(enrollment(ee)).unwrap();

    (db, ee_oid)
}

/// Build an enrollment that would transition NORMAL -> HIGH_LIMIT, with
/// `Event_Detection_Enable` set from `detection_enabled`.
fn setup(detection_enabled: bool) -> (ObjectDatabase, ObjectIdentifier) {
    setup_with(|mut ee| {
        if !detection_enabled {
            ee.write_property(
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                None,
                PropertyValue::Boolean(false),
                None,
            )
            .unwrap();
        }
        Box::new(ee)
    })
}

/// Control: with detection enabled the same fixture does transition, so the
/// disabled cases below are showing the flag at work rather than a fixture
/// that never fires.
#[test]
fn detection_enabled_evaluates_normally() {
    let (mut db, _ee_oid) = setup(true);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
}

/// Clause 13.2.2.1 suspends evaluation and prohibits transitions while
/// Event_Detection_Enable is FALSE.
#[test]
fn detection_disabled_yields_no_transitions() {
    let (mut db, _ee_oid) = setup(false);
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "no transition may occur while Event_Detection_Enable is FALSE"
    );
}

/// Clause 12.12 requires Event_State to remain NORMAL throughout the disabled
/// period, so an evaluation pass over a
/// disabled enrollment must leave NORMAL in place, not merely decline to
/// report a transition it already applied.
#[test]
fn detection_disabled_holds_event_state_at_normal() {
    let (mut db, ee_oid) = setup(false);
    evaluate_event_enrollments(&mut db, 1);

    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

/// Re-enabling detection resumes evaluation, and the out-of-range condition
/// that was ignored while disabled is now detected afresh from NORMAL.
///
/// The standard is silent on the FALSE -> TRUE edge for this property — note
/// the contrast with `Event_Algorithm_Inhibit`, which the standard gives an
/// explicit re-arm rule: after switching to FALSE, a condition must persist
/// for its full applicable delay. Resuming from NORMAL is this implementation's
/// choice, and follows from the reset having already run.
#[test]
fn re_enabling_detection_resumes_evaluation() {
    let (mut db, ee_oid) = setup(false);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());

    db.get_mut(&ee_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1, "detection resumes once re-enabled");
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
}

/// An enabled Event Enrollment that reads Event_Detection_Enable as FALSE:
/// its own guards accept every commit, so only the evaluator's check of the
/// property keeps a pass from transitioning it.
struct ReadsDisabled {
    inner: EventEnrollmentObject,
}

impl BACnetObject for ReadsDisabled {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.inner.object_name()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, bacnet_types::error::Error> {
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            return Ok(PropertyValue::Boolean(false));
        }
        self.inner.read_property(property, array_index)
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), bacnet_types::error::Error> {
        self.inner
            .write_property(property, array_index, value, priority)
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.inner.property_metadata()
    }
    fn event_reporting(&self) -> Option<&dyn EventReporting> {
        self.inner.event_reporting()
    }
    fn event_reporting_mut(&mut self) -> Option<&mut dyn EventReporting> {
        self.inner.event_reporting_mut()
    }
    fn enrollment_eval(&self) -> Option<&dyn EnrollmentEval> {
        self.inner.enrollment_eval()
    }
    fn enrollment_eval_mut(&mut self) -> Option<&mut dyn EnrollmentEval> {
        self.inner.enrollment_eval_mut()
    }
}

#[test]
fn the_evaluator_skips_an_enrollment_that_reads_disabled() {
    let (mut db, ee_oid) = setup_with(|inner| Box::new(ReadsDisabled { inner }));
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "the evaluator must skip an enrollment whose Event_Detection_Enable reads FALSE"
    );
    assert_eq!(
        db.get(&ee_oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}
