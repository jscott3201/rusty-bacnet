//! Public source-compatibility regressions for the legacy evaluation report.

use bacnet_server::event_enrollment::{
    EventEnrollmentEvaluationOutcome, EventEnrollmentEvaluationReport,
    EventEnrollmentEvaluationStage,
};

fn legacy_stage_is_exhaustive(stage: EventEnrollmentEvaluationStage) -> u8 {
    match stage {
        EventEnrollmentEvaluationStage::Evaluation => 0,
        EventEnrollmentEvaluationStage::EvaluationSource => 1,
        EventEnrollmentEvaluationStage::EvaluationState => 2,
        EventEnrollmentEvaluationStage::EventTransition => 3,
    }
}

fn legacy_outcome_is_exhaustive(outcome: EventEnrollmentEvaluationOutcome) -> u8 {
    match outcome {
        EventEnrollmentEvaluationOutcome::NoTransition => 0,
        EventEnrollmentEvaluationOutcome::CancellationCommitted => 1,
        EventEnrollmentEvaluationOutcome::Rejected => 2,
        EventEnrollmentEvaluationOutcome::LandedAfterError => 3,
    }
}

#[test]
fn legacy_report_remains_constructible_with_its_original_fields() {
    let report = EventEnrollmentEvaluationReport {
        transitions: vec![],
        diagnostics: vec![],
    };

    assert!(report.transitions.is_empty());
    assert!(report.diagnostics.is_empty());
    assert_eq!(
        legacy_stage_is_exhaustive(EventEnrollmentEvaluationStage::Evaluation),
        0
    );
    assert_eq!(
        legacy_outcome_is_exhaustive(EventEnrollmentEvaluationOutcome::NoTransition),
        0
    );
}
