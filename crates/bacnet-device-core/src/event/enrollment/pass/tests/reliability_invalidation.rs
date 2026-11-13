use super::*;

#[test]
fn held_configuration_error_clears_landed_after_error_invalidation_once() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(306, "AV-held-config", CELSIUS).unwrap();
    target
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(1),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 1)));
    enrollment
        .inner
        .set_fault_parameters(Some(FaultParameters::FaultCharacterString {
            fault_values: vec!["unsupported".into()],
        }));
    enrollment.reliability_error_after_write = true;
    let state_writes = enrollment.state_write_count.clone();
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let first = evaluate_event_enrollments_report(&mut db, 1);
    assert!(
        first
            .diagnostics
            .contains(&EventEnrollmentEvaluationDiagnostic {
                enrollment_oid,
                stage: EventEnrollmentEvaluationStage::Reliability,
                outcome: EventEnrollmentEvaluationOutcome::LandedAfterError,
            })
    );
    assert!(db.enrollment_eval_state_invalidated(&enrollment_oid));
    assert_eq!(
        reliability(&db, enrollment_oid),
        Reliability::CONFIGURATION_ERROR
    );
    assert_eq!(event_state(&db, enrollment_oid), EventState::FAULT);

    state_writes.store(0, Ordering::SeqCst);
    let held = evaluate_event_enrollments_report(&mut db, 1);
    assert!(held.reliability_results.is_empty());
    assert_eq!(state_writes.load(Ordering::SeqCst), 1);
    assert!(!db.enrollment_eval_state_invalidated(&enrollment_oid));

    let held_again = evaluate_event_enrollments_report(&mut db, 1);
    assert!(held_again.reliability_results.is_empty());
    assert_eq!(state_writes.load(Ordering::SeqCst), 1);
    assert!(!db.enrollment_eval_state_invalidated(&enrollment_oid));
}
