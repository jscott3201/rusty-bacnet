use super::*;

#[test]
fn invalid_indexed_configuration_remains_configuration_error() {
    let mut db = ObjectDatabase::new();
    let target = ObservationTarget::new(332, false, false);
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();
    let enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 17)));
    db.add(Box::new(enrollment)).unwrap();

    let report = evaluate_event_enrollments_report(&mut db, 1);
    assert!(report.transitions.is_empty());
    assert_eq!(report.reliability_results.len(), 1);
    assert_eq!(
        report.reliability_results[0].new_reliability,
        bacnet_types::enums::Reliability::CONFIGURATION_ERROR
    );
    assert!(!report.diagnostics.iter().any(|diagnostic| {
        diagnostic.outcome == EventEnrollmentEvaluationOutcome::ObservationUnavailable
    }));
}
