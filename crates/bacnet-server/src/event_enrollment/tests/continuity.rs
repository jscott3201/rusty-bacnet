//! Repository-local D4 observation-continuity policy (#305).

use super::super::*;
use super::integration::{
    indexed_reference_value, setup_qualified_reference, ReferenceValueObject,
};
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::binary::BinaryInputObject;
use bacnet_objects::event_enrollment::{
    EventEnrollmentEvalState, EventEnrollmentObject, EventEnrollmentPending,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
    ChangeOfValueCriteria, FaultParameters,
};
use bacnet_types::error::Error;
use std::borrow::Cow;
use std::sync::atomic::Ordering;

fn stale_state() -> EventEnrollmentEvalState {
    EventEnrollmentEvalState {
        pending: Some(EventEnrollmentPending {
            state: EventState::HIGH_LIMIT,
            remaining: 1,
            condition: 7,
            params_fingerprint: 11,
        }),
        cov_baseline: Some(PropertyValue::Real(33.0)),
        last_offnormal_value: Some(2),
    }
}

fn reference_value(oid: ObjectIdentifier, property: PropertyIdentifier) -> PropertyValue {
    PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(oid),
        PropertyValue::Unsigned(property.to_raw() as u64),
    ])
}

fn retarget(
    db: &mut ObjectDatabase,
    enrollment_oid: ObjectIdentifier,
    target_oid: ObjectIdentifier,
    property: PropertyIdentifier,
) {
    let mut enrollment = db.remove(&enrollment_oid).unwrap();
    enrollment
        .write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            reference_value(target_oid, property),
            None,
        )
        .unwrap();
    db.add(enrollment).unwrap();
}

fn set_input_value(object: &mut dyn BACnetObject, value: PropertyValue) {
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    object
        .write_property(PropertyIdentifier::PRESENT_VALUE, None, value, None)
        .unwrap();
}

#[derive(Debug, PartialEq)]
struct PublicSnapshot {
    reliability: PropertyValue,
    event_state: PropertyValue,
    acked_transitions: PropertyValue,
    event_time_stamps: PropertyValue,
    next_sequence: u16,
}

fn public_snapshot(db: &mut ObjectDatabase, oid: ObjectIdentifier) -> PublicSnapshot {
    let object = db.get(&oid).unwrap();
    let reliability = object
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    let event_state = object
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    let acked_transitions = object
        .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
        .unwrap();
    let event_time_stamps = object
        .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
        .unwrap();
    PublicSnapshot {
        reliability,
        event_state,
        acked_transitions,
        event_time_stamps,
        next_sequence: db.reserve_event_sequence_number().number(),
    }
}

fn assert_observation_gap(
    report: &EventEnrollmentDetailedEvaluationReport,
    enrollment_oid: ObjectIdentifier,
) {
    assert!(report.transitions.is_empty());
    assert!(report.reliability_results.is_empty());
    assert!(report
        .diagnostics
        .contains(&EventEnrollmentDetailedEvaluationDiagnostic {
            enrollment_oid,
            stage: EventEnrollmentDetailedEvaluationStage::Reliability,
            outcome: EventEnrollmentDetailedEvaluationOutcome::ObservationUnavailable,
        }));
}

fn assert_private_reset(db: &ObjectDatabase, enrollment_oid: ObjectIdentifier) {
    let enrollment = db.get(&enrollment_oid).unwrap();
    assert_eq!(
        enrollment.enrollment_eval_state_internal(),
        Some(EventEnrollmentEvalState::default())
    );
    assert_eq!(enrollment.enrollment_eval_source_internal(), Some(None));
}

fn add_out_of_range_enrollment(
    db: &mut ObjectDatabase,
    instance: u32,
    target_oid: ObjectIdentifier,
    delay: u32,
) -> ObjectIdentifier {
    let mut enrollment = EventEnrollmentObject::new(
        instance,
        format!("EE-continuity-{instance}"),
        EventType::OUT_OF_RANGE.to_raw(),
    )
    .unwrap();
    enrollment.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        target_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    enrollment.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: delay,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    enrollment.set_event_enable(0x07);
    let oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    oid
}

#[test]
fn removed_same_target_clears_all_continuity_and_restarts_full_delay() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogInputObject::new(320, "AI-remove-restore", 62).unwrap();
    target.set_present_value(90.0);
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();
    let enrollment_oid = add_out_of_range_enrollment(&mut db, 320, target_oid, 3);
    let enrollment = db.get_mut(&enrollment_oid).unwrap();
    enrollment
        .set_enrollment_eval_state_internal(stale_state())
        .unwrap();
    enrollment
        .set_enrollment_eval_source_internal(Some((
            target_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )))
        .unwrap();

    let removed = db.remove(&target_oid).unwrap();
    let before = public_snapshot(&mut db, enrollment_oid);
    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_observation_gap(&report, enrollment_oid);
    assert_private_reset(&db, enrollment_oid);
    assert_eq!(public_snapshot(&mut db, enrollment_oid), before);

    db.add(removed).unwrap();
    for pass in 1..=3 {
        assert!(
            evaluate_event_enrollments(&mut db, 1).is_empty(),
            "restored pass {pass} must run the fresh delay"
        );
    }
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::HIGH_LIMIT
    );
}

#[test]
fn cov_restore_and_valid_retarget_each_establish_a_fresh_baseline() {
    let mut db = ObjectDatabase::new();
    let mut first = AnalogInputObject::new(321, "AI-COV-first", 62).unwrap();
    first.set_present_value(10.0);
    let first_oid = first.object_identifier();
    db.add(Box::new(first)).unwrap();
    let mut second = AnalogInputObject::new(322, "AI-COV-second", 62).unwrap();
    second.set_present_value(100.0);
    let second_oid = second.object_identifier();
    db.add(Box::new(second)).unwrap();

    let mut enrollment = ReferenceValueObject::new_for_event_type(
        Some(reference_value(
            first_oid,
            PropertyIdentifier::PRESENT_VALUE,
        )),
        EventType::CHANGE_OF_VALUE,
    );
    enrollment
        .inner
        .set_event_parameters(BACnetEventParameter::ChangeOfValue {
            time_delay: 0,
            criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
        });
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());

    let mut removed = db.remove(&first_oid).unwrap();
    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_observation_gap(&report, enrollment_oid);
    assert_private_reset(&db, enrollment_oid);
    set_input_value(removed.as_mut(), PropertyValue::Real(30.0));
    db.add(removed).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_state_internal()
            .unwrap()
            .cov_baseline,
        Some(PropertyValue::Real(30.0))
    );

    retarget(
        &mut db,
        enrollment_oid,
        second_oid,
        PropertyIdentifier::PRESENT_VALUE,
    );
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    let enrollment = db.get(&enrollment_oid).unwrap();
    assert_eq!(
        enrollment
            .enrollment_eval_state_internal()
            .unwrap()
            .cov_baseline,
        Some(PropertyValue::Real(100.0))
    );
    assert_eq!(
        enrollment.enrollment_eval_source_internal(),
        Some(Some((second_oid, PropertyIdentifier::PRESENT_VALUE, None)))
    );
}

#[test]
fn change_of_state_does_not_reuse_pre_gap_last_offnormal_identity() {
    let mut db = ObjectDatabase::new();
    let mut target = BinaryInputObject::new(323, "BI-COS-continuity").unwrap();
    target.set_present_value(1);
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();
    let mut enrollment = EventEnrollmentObject::new(
        323,
        "EE-COS-continuity",
        EventType::CHANGE_OF_STATE.to_raw(),
    )
    .unwrap();
    enrollment.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        target_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    enrollment.set_event_parameters(BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![
            BACnetPropertyStates::BinaryValue(1),
            BACnetPropertyStates::BinaryValue(0),
        ],
    });
    enrollment.set_event_enable(0x07);
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);

    let mut removed = db.remove(&target_oid).unwrap();
    set_input_value(removed.as_mut(), PropertyValue::Enumerated(0));
    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_observation_gap(&report, enrollment_oid);
    assert_private_reset(&db, enrollment_oid);
    db.add(removed).unwrap();

    let restored = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert!(restored.transitions.is_empty());
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_state_internal()
            .unwrap()
            .last_offnormal_value,
        None
    );
}

#[test]
fn retarget_to_missing_then_restore_clears_old_owner_and_restarts_delay() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogInputObject::new(324, "AI-retarget-missing", 62).unwrap();
    target.set_present_value(90.0);
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();
    let enrollment = ReferenceValueObject::new(Some(reference_value(
        target_oid,
        PropertyIdentifier::PRESENT_VALUE,
    )));
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());

    let missing_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 325).unwrap();
    retarget(
        &mut db,
        enrollment_oid,
        missing_oid,
        PropertyIdentifier::PRESENT_VALUE,
    );
    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_observation_gap(&report, enrollment_oid);
    assert_private_reset(&db, enrollment_oid);

    retarget(
        &mut db,
        enrollment_oid,
        target_oid,
        PropertyIdentifier::PRESENT_VALUE,
    );
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
}

#[test]
fn foreign_target_gap_clears_private_continuity_without_public_transition() {
    let (mut db, enrollment_oid, target_oid) = setup_qualified_reference(&[100], 200);
    let enrollment = db.get_mut(&enrollment_oid).unwrap();
    enrollment
        .set_enrollment_eval_state_internal(stale_state())
        .unwrap();
    enrollment
        .set_enrollment_eval_source_internal(Some((
            target_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )))
        .unwrap();
    let before = public_snapshot(&mut db, enrollment_oid);
    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_observation_gap(&report, enrollment_oid);
    assert_private_reset(&db, enrollment_oid);
    assert_eq!(public_snapshot(&mut db, enrollment_oid), before);
}

struct ObservationTarget {
    inner: AnalogValueObject,
    fail_reliability: bool,
    fail_indexed_value: bool,
}

impl ObservationTarget {
    fn new(instance: u32, fail_reliability: bool, fail_indexed_value: bool) -> Self {
        let mut inner = AnalogValueObject::new(instance, format!("AV-gap-{instance}"), 62).unwrap();
        inner
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(90.0),
                Some(1),
            )
            .unwrap();
        Self {
            inner,
            fail_reliability,
            fail_indexed_value,
        }
    }
}

impl BACnetObject for ObservationTarget {
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
    ) -> Result<PropertyValue, Error> {
        if self.fail_reliability && property == PropertyIdentifier::RELIABILITY {
            return Err(Error::Encoding("transient Reliability read".into()));
        }
        if self.fail_indexed_value
            && property == PropertyIdentifier::PRIORITY_ARRAY
            && array_index.is_some()
        {
            return Err(Error::Encoding("transient indexed observation".into()));
        }
        self.inner.read_property(property, array_index)
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.inner
            .write_property(property, array_index, value, priority)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.inner.property_list()
    }

    fn is_array_property(&self, property: PropertyIdentifier) -> bool {
        self.inner.is_array_property(property)
    }
}

fn add_custom_enrollment_with_stale_state(
    db: &mut ObjectDatabase,
    reference: PropertyValue,
    source: (ObjectIdentifier, PropertyIdentifier, Option<u32>),
) -> (
    ObjectIdentifier,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let mut enrollment = ReferenceValueObject::new(Some(reference));
    enrollment
        .inner
        .set_enrollment_eval_state_internal(stale_state())
        .unwrap();
    enrollment
        .inner
        .set_enrollment_eval_source_internal(Some(source))
        .unwrap();
    let writes = enrollment.state_write_count.clone();
    let oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    (oid, writes)
}

#[test]
fn transient_indexed_read_resets_once_and_preserves_public_coordinates() {
    let mut db = ObjectDatabase::new();
    let target = ObservationTarget::new(326, false, true);
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();
    let source = (target_oid, PropertyIdentifier::PRIORITY_ARRAY, Some(1));
    let (enrollment_oid, writes) = add_custom_enrollment_with_stale_state(
        &mut db,
        indexed_reference_value(target_oid, 1),
        source,
    );
    let before = public_snapshot(&mut db, enrollment_oid);

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_observation_gap(&report, enrollment_oid);
    assert_private_reset(&db, enrollment_oid);
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert_eq!(public_snapshot(&mut db, enrollment_oid), before);
}

#[test]
fn transient_monitored_reliability_read_clears_continuity() {
    let mut db = ObjectDatabase::new();
    let target = ObservationTarget::new(327, true, false);
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();
    let source = (target_oid, PropertyIdentifier::PRESENT_VALUE, None);
    let (enrollment_oid, writes) = add_custom_enrollment_with_stale_state(
        &mut db,
        reference_value(target_oid, PropertyIdentifier::PRESENT_VALUE),
        source,
    );

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_observation_gap(&report, enrollment_oid);
    assert_private_reset(&db, enrollment_oid);
    assert_eq!(writes.load(Ordering::SeqCst), 1);
}

#[test]
fn missing_fault_status_flags_observation_clears_continuity() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogInputObject::new(328, "AI-fault-flags", 62).unwrap();
    target.set_present_value(90.0);
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();
    let flags_oid = ObjectIdentifier::new(ObjectType::BINARY_INPUT, 328).unwrap();
    let mut enrollment =
        EventEnrollmentObject::new(328, "EE-fault-flags", EventType::OUT_OF_RANGE.to_raw())
            .unwrap();
    enrollment.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        target_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    enrollment.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    enrollment.set_fault_parameters(Some(FaultParameters::FaultStatusFlags {
        reference: BACnetDeviceObjectPropertyReference::new_local(
            flags_oid,
            PropertyIdentifier::STATUS_FLAGS.to_raw(),
        ),
    }));
    enrollment
        .set_enrollment_eval_state_internal(stale_state())
        .unwrap();
    enrollment
        .set_enrollment_eval_source_internal(Some((
            target_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )))
        .unwrap();
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_observation_gap(&report, enrollment_oid);
    assert_private_reset(&db, enrollment_oid);
}

#[test]
fn floating_setpoint_gap_overwrites_queued_ownership_and_resets_once() {
    let mut db = ObjectDatabase::new();
    let mut monitored = AnalogInputObject::new(329, "AI-floating-gap", 62).unwrap();
    monitored.set_present_value(65.0);
    let monitored_oid = monitored.object_identifier();
    db.add(Box::new(monitored)).unwrap();
    let missing_setpoint = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 330).unwrap();
    let mut enrollment = ReferenceValueObject::new_for_event_type(
        Some(reference_value(
            monitored_oid,
            PropertyIdentifier::PRESENT_VALUE,
        )),
        EventType::FLOATING_LIMIT,
    );
    enrollment
        .inner
        .set_event_parameters(BACnetEventParameter::FloatingLimit {
            time_delay: 2,
            setpoint_reference: BACnetDeviceObjectPropertyReference::new_local(
                missing_setpoint,
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            ),
            low_diff_limit: 10.0,
            high_diff_limit: 10.0,
            deadband: 2.0,
        });
    enrollment
        .inner
        .set_enrollment_eval_state_internal(stale_state())
        .unwrap();
    enrollment
        .inner
        .set_enrollment_eval_source_internal(Some((
            monitored_oid,
            PropertyIdentifier::DESCRIPTION,
            None,
        )))
        .unwrap();
    let writes = enrollment.state_write_count.clone();
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    let before = public_snapshot(&mut db, enrollment_oid);

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_observation_gap(&report, enrollment_oid);
    assert_private_reset(&db, enrollment_oid);
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert_eq!(public_snapshot(&mut db, enrollment_oid), before);
}

#[test]
fn ownerless_floating_gap_discards_staged_ownership_without_source_write() {
    let mut db = ObjectDatabase::new();
    let mut monitored = AnalogInputObject::new(333, "AI-ownerless-floating-gap", 62).unwrap();
    monitored.set_present_value(65.0);
    let monitored_oid = monitored.object_identifier();
    db.add(Box::new(monitored)).unwrap();
    let missing_setpoint = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 334).unwrap();
    let mut enrollment = ReferenceValueObject::new_for_event_type(
        Some(reference_value(
            monitored_oid,
            PropertyIdentifier::PRESENT_VALUE,
        )),
        EventType::FLOATING_LIMIT,
    );
    enrollment
        .inner
        .set_event_parameters(BACnetEventParameter::FloatingLimit {
            time_delay: 2,
            setpoint_reference: BACnetDeviceObjectPropertyReference::new_local(
                missing_setpoint,
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            ),
            low_diff_limit: 10.0,
            high_diff_limit: 10.0,
            deadband: 2.0,
        });
    enrollment
        .inner
        .set_enrollment_eval_state_internal(stale_state())
        .unwrap();
    enrollment.source_writable.store(false, Ordering::SeqCst);
    let writes = enrollment.state_write_count.clone();
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    let before = public_snapshot(&mut db, enrollment_oid);

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_observation_gap(&report, enrollment_oid);
    assert_private_reset(&db, enrollment_oid);
    assert!(!report.diagnostics.iter().any(|diagnostic| {
        diagnostic.stage == EventEnrollmentDetailedEvaluationStage::EvaluationSource
            && diagnostic.outcome == EventEnrollmentDetailedEvaluationOutcome::Rejected
    }));
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert!(!db.enrollment_eval_state_invalidated(&enrollment_oid));
    assert_eq!(public_snapshot(&mut db, enrollment_oid), before);
}

#[test]
fn rejected_gap_resets_remain_visible_alongside_observation_diagnostic() {
    let mut db = ObjectDatabase::new();
    let mut enrollment = ReferenceValueObject::new(None);
    let stale_source = (
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 331).unwrap(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    enrollment
        .inner
        .set_enrollment_eval_state_internal(stale_state())
        .unwrap();
    enrollment
        .inner
        .set_enrollment_eval_source_internal(Some(stale_source))
        .unwrap();
    enrollment.source_writable.store(false, Ordering::SeqCst);
    enrollment.state_writable.store(false, Ordering::SeqCst);
    let writes = enrollment.state_write_count.clone();
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_observation_gap(&report, enrollment_oid);
    assert!(report
        .diagnostics
        .contains(&EventEnrollmentDetailedEvaluationDiagnostic {
            enrollment_oid,
            stage: EventEnrollmentDetailedEvaluationStage::EvaluationSource,
            outcome: EventEnrollmentDetailedEvaluationOutcome::Rejected,
        }));
    assert!(report
        .diagnostics
        .contains(&EventEnrollmentDetailedEvaluationDiagnostic {
            enrollment_oid,
            stage: EventEnrollmentDetailedEvaluationStage::EvaluationState,
            outcome: EventEnrollmentDetailedEvaluationOutcome::Rejected,
        }));
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert!(db.enrollment_eval_state_invalidated(&enrollment_oid));
}

#[test]
fn ownerless_gap_skips_redundant_rejected_source_reset() {
    let mut db = ObjectDatabase::new();
    let mut enrollment = ReferenceValueObject::new(None);
    enrollment
        .inner
        .set_enrollment_eval_state_internal(stale_state())
        .unwrap();
    enrollment.source_writable.store(false, Ordering::SeqCst);
    let writes = enrollment.state_write_count.clone();
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_observation_gap(&report, enrollment_oid);
    assert_private_reset(&db, enrollment_oid);
    assert!(!report.diagnostics.iter().any(|diagnostic| {
        diagnostic.stage == EventEnrollmentDetailedEvaluationStage::EvaluationSource
            && diagnostic.outcome == EventEnrollmentDetailedEvaluationOutcome::Rejected
    }));
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert!(!db.enrollment_eval_state_invalidated(&enrollment_oid));
}

#[test]
fn invalid_indexed_configuration_remains_configuration_error() {
    let mut db = ObjectDatabase::new();
    let target = ObservationTarget::new(332, false, false);
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();
    let enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 17)));
    db.add(Box::new(enrollment)).unwrap();

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert!(report.transitions.is_empty());
    assert_eq!(report.reliability_results.len(), 1);
    assert_eq!(
        report.reliability_results[0].new_reliability,
        bacnet_types::enums::Reliability::CONFIGURATION_ERROR
    );
    assert!(!report.diagnostics.iter().any(|diagnostic| {
        diagnostic.outcome == EventEnrollmentDetailedEvaluationOutcome::ObservationUnavailable
    }));
}
