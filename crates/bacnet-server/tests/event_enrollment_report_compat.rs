//! Public source-compatibility regressions for the legacy evaluation report.

use bacnet_objects::event::EventStateChange;
use bacnet_server::event_enrollment::{
    EventEnrollmentDetailedEvaluationReport, EventEnrollmentEvaluationOutcome,
    EventEnrollmentEvaluationReport, EventEnrollmentEvaluationStage,
    EventEnrollmentReliabilityCause, EventEnrollmentReliabilityResult, EventEnrollmentTransition,
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

#[test]
fn public_transition_and_detailed_report_keep_their_original_fields() {
    let enrollment_oid = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 2).unwrap();
    let monitored_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
    let transition = EventEnrollmentTransition {
        enrollment_oid,
        monitored_oid,
        change: EventStateChange {
            from: EventState::NORMAL,
            to: EventState::HIGH_LIMIT,
        },
        event_type: EventType::OUT_OF_RANGE,
        distribute: true,
    };
    let report = EventEnrollmentDetailedEvaluationReport {
        transitions: vec![transition],
        reliability_results: vec![],
        diagnostics: vec![],
    };

    assert_eq!(report.transitions[0].enrollment_oid, enrollment_oid);
    assert_eq!(report.transitions[0].monitored_oid, monitored_oid);
    assert!(report.reliability_results.is_empty());
    assert!(report.diagnostics.is_empty());
}
