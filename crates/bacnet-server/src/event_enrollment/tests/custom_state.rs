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
fn stateful_custom_enrollment_without_source_channel_runs_statelessly() {
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
        Some(bacnet_objects::event_enrollment::EventEnrollmentEvalState::default())
    );

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
            list_of_values: vec![BACnetPropertyStates::UnsignedValue(1)],
        });
    enrollment.source_writable = false;
    db.add(Box::new(enrollment)).unwrap();

    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
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
