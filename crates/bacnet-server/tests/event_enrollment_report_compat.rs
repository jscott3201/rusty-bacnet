//! Public source-compatibility regressions for the legacy evaluation report.

use bacnet_objects::event::EventStateChange;
use bacnet_server::event_enrollment::{
    EventEnrollmentEvaluationOutcome, EventEnrollmentEvaluationReport,
    EventEnrollmentEvaluationStage, EventEnrollmentReliabilityCause,
    EventEnrollmentReliabilityResult,
};
use bacnet_types::enums::{EventState, EventType, ObjectType, Reliability};
use bacnet_types::primitives::ObjectIdentifier;

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

#[test]
fn detailed_reliability_result_remains_constructible_and_derives_event_type() {
    let result = EventEnrollmentReliabilityResult {
        enrollment_oid: ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap(),
        monitored_oid: None,
        previous_reliability: Reliability::NO_FAULT_DETECTED,
        new_reliability: Reliability::CONFIGURATION_ERROR,
        state_change: Some(EventStateChange {
            from: EventState::NORMAL,
            to: EventState::FAULT,
        }),
        distribute: true,
        cause: EventEnrollmentReliabilityCause::Configuration,
    };

    assert_eq!(
        result.event_type(EventType::OUT_OF_RANGE),
        Some(EventType::CHANGE_OF_RELIABILITY)
    );

    let no_state_change = EventEnrollmentReliabilityResult {
        state_change: None,
        ..result
    };
    assert_eq!(no_state_change.event_type(EventType::OUT_OF_RANGE), None);
}
