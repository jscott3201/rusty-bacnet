use super::super::*;
use super::integration::{indexed_reference_value, ReferenceValueObject};
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetEventParameter, BACnetPropertyStates, ChangeOfValueCriteria,
};
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[test]
fn unreadable_parameters_cancel_pending_countdown() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(97, "AV-unreadable-params", 62).unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 1)));
    let parameters_readable = Arc::clone(&enrollment.event_parameters_readable);
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());

    parameters_readable.store(false, Ordering::SeqCst);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(db
        .get(&enrollment_oid)
        .unwrap()
        .enrollment_eval_state_internal()
        .unwrap()
        .pending
        .is_none());

    parameters_readable.store(true, Ordering::SeqCst);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::HIGH_LIMIT
    );
}

#[test]
fn source_less_indexed_cov_retains_its_baseline_and_retargets_safely() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(98, "AV-custom-source", 62).unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(10.0),
            Some(1),
        )
        .unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(2),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut enrollment = ReferenceValueObject::new_for_event_type(
        Some(indexed_reference_value(target_oid, 1)),
        EventType::CHANGE_OF_VALUE,
    );
    enrollment
        .inner
        .set_event_parameters(BACnetEventParameter::ChangeOfValue {
            time_delay: 0,
            criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
        });
    enrollment.source_supported = false;
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_state_internal(),
        Some(bacnet_objects::event_enrollment::EventEnrollmentEvalState {
            pending: None,
            cov_baseline: Some(PropertyValue::Real(10.0)),
            last_offnormal_value: None,
        })
    );

    db.get_mut(&target_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(20.0),
            Some(1),
        )
        .unwrap();
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);

    db.get_mut(&enrollment_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            indexed_reference_value(target_oid, 2),
            None,
        )
        .unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());

    db.get_mut(&target_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(100.0),
            Some(2),
        )
        .unwrap();
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
}

#[test]
fn failed_indexed_reset_cannot_resume_after_restoring_the_reference() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(103, "AV-reset-retry", 62).unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 1)));
    let state_writable = Arc::clone(&enrollment.state_writable);
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    state_writable.store(false, Ordering::SeqCst);
    db.get_mut(&enrollment_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            indexed_reference_value(target_oid, 17),
            None,
        )
        .unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_source_internal(),
        Some(None)
    );

    state_writable.store(true, Ordering::SeqCst);
    db.get_mut(&enrollment_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            indexed_reference_value(target_oid, 1),
            None,
        )
        .unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
}

#[test]
fn source_less_unindexed_cov_does_not_reuse_a_retargeted_baseline() {
    let mut db = ObjectDatabase::new();
    let mut first = AnalogInputObject::new(104, "AI-COV-first", 62).unwrap();
    first.set_present_value(10.0);
    let first_oid = first.object_identifier();
    db.add(Box::new(first)).unwrap();
    let mut second = AnalogInputObject::new(105, "AI-COV-second", 62).unwrap();
    second.set_present_value(90.0);
    let second_oid = second.object_identifier();
    db.add(Box::new(second)).unwrap();

    let reference = PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(first_oid),
        PropertyValue::Unsigned(PropertyIdentifier::PRESENT_VALUE.to_raw() as u64),
    ]);
    let mut enrollment =
        ReferenceValueObject::new_for_event_type(Some(reference), EventType::CHANGE_OF_VALUE);
    enrollment
        .inner
        .set_event_parameters(BACnetEventParameter::ChangeOfValue {
            time_delay: 0,
            criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
        });
    enrollment.source_supported = false;
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    db.get_mut(&enrollment_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(second_oid),
                PropertyValue::Unsigned(PropertyIdentifier::PRESENT_VALUE.to_raw() as u64),
            ]),
            None,
        )
        .unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());

    db.get_mut(&second_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    db.get_mut(&second_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(100.0),
            None,
        )
        .unwrap();
    db.get_mut(&second_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
}

#[test]
fn source_write_failure_allows_one_immediate_change_of_state_transition() {
    let mut db = ObjectDatabase::new();
    let mut target = BinaryValueObject::new(106, "BV-source-failure").unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(1),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut enrollment = ReferenceValueObject::new_for_event_type(
        Some(indexed_reference_value(target_oid, 1)),
        EventType::CHANGE_OF_STATE,
    );
    enrollment
        .inner
        .set_event_parameters(BACnetEventParameter::ChangeOfState {
            time_delay: 0,
            list_of_values: vec![BACnetPropertyStates::BinaryValue(1)],
        });
    enrollment.source_writable.store(false, Ordering::SeqCst);
    db.add(Box::new(enrollment)).unwrap();

    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
}

#[test]
fn invalidation_survives_failed_state_and_source_writes() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(107, "AV-double-failure", 62).unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 1)));
    let state_writable = Arc::clone(&enrollment.state_writable);
    let source_writable = Arc::clone(&enrollment.source_writable);
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    state_writable.store(false, Ordering::SeqCst);
    source_writable.store(false, Ordering::SeqCst);
    db.get_mut(&enrollment_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            indexed_reference_value(target_oid, 17),
            None,
        )
        .unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(db.enrollment_eval_state_invalidated(&enrollment_oid));

    db.get_mut(&enrollment_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            indexed_reference_value(target_oid, 1),
            None,
        )
        .unwrap();
    state_writable.store(true, Ordering::SeqCst);
    source_writable.store(true, Ordering::SeqCst);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(!db.enrollment_eval_state_invalidated(&enrollment_oid));
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
}

#[test]
fn source_less_custom_enrollment_retains_unindexed_delay_behavior() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogInputObject::new(102, "AI-unindexed-source", 62).unwrap();
    target.set_present_value(90.0);
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let reference = PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(target_oid),
        PropertyValue::Unsigned(PropertyIdentifier::PRESENT_VALUE.to_raw() as u64),
    ]);
    let mut enrollment = ReferenceValueObject::new(Some(reference));
    enrollment.source_supported = false;
    db.add(Box::new(enrollment)).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
}

#[test]
fn source_less_custom_enrollment_retains_indexed_delay_behavior() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(108, "AV-indexed-source", 62).unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 1)));
    enrollment.source_supported = false;
    db.add(Box::new(enrollment)).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
}

#[test]
fn replacing_the_monitored_object_restarts_an_indexed_delay() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(109, "AV-replaced-source", 62).unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 1)));
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());

    let mut replacement = AnalogValueObject::new(109, "AV-replacement", 62).unwrap();
    replacement
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
        )
        .unwrap();
    db.add(Box::new(replacement)).unwrap();
    assert!(db.enrollment_eval_state_invalidated(&enrollment_oid));

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
}

#[test]
fn failed_eval_state_write_cannot_leak_an_unreported_event_state() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(110, "AV-event-state-order", 62).unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 1)));
    enrollment.normal_event_state_writable = false;
    let state_writable = Arc::clone(&enrollment.state_writable);
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    state_writable.store(false, Ordering::SeqCst);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

#[test]
fn same_state_transition_does_not_depend_on_rewriting_event_state() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(111, "AV-same-state-write", 62).unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(10.0),
            Some(1),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut enrollment = ReferenceValueObject::new_for_event_type(
        Some(indexed_reference_value(target_oid, 1)),
        EventType::CHANGE_OF_VALUE,
    );
    enrollment
        .inner
        .set_event_parameters(BACnetEventParameter::ChangeOfValue {
            time_delay: 0,
            criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
        });
    enrollment.normal_event_state_writable = false;
    db.add(Box::new(enrollment)).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    db.get_mut(&target_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(20.0),
            Some(1),
        )
        .unwrap();
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
}

#[test]
fn mutation_then_error_is_visible_but_never_reports_a_transition() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(112, "AV-landed-error", 62).unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 1)));
    enrollment
        .inner
        .set_event_parameters(BACnetEventParameter::OutOfRange {
            time_delay: 0,
            low_limit: 20.0,
            high_limit: 80.0,
            deadband: 2.0,
        });
    enrollment.event_state_error_after_write = true;
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let report = evaluate_event_enrollments_report(&mut db, 1);
    assert!(report.transitions.is_empty());
    assert!(report
        .diagnostics
        .contains(&EventEnrollmentEvaluationDiagnostic {
            enrollment_oid,
            stage: EventEnrollmentEvaluationStage::EventTransition,
            outcome: EventEnrollmentEvaluationOutcome::LandedAfterError,
        }));
    assert!(db.enrollment_eval_state_invalidated(&enrollment_oid));
    assert_eq!(db.reserve_event_sequence_number().number(), 0);
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
}

#[test]
fn detached_enrollment_does_not_resume_state_after_target_replacement() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(113, "AV-detached-source", 62).unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 1)));
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());

    let enrollment = db.remove(&enrollment_oid).unwrap();
    db.remove(&target_oid).unwrap();
    let mut replacement = AnalogValueObject::new(113, "AV-detached-replacement", 62).unwrap();
    replacement
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
        )
        .unwrap();
    db.add(Box::new(replacement)).unwrap();
    db.add(enrollment).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
}
