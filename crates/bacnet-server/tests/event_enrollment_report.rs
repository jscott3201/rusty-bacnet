//! Public construction and lossless evaluation report contracts.

use bacnet_objects::event::EventStateChange;
use bacnet_server::event_enrollment::{
    EventEnrollmentEvaluationDiagnostic, EventEnrollmentEvaluationOutcome,
    EventEnrollmentEvaluationReport, EventEnrollmentEvaluationStage,
    EventEnrollmentReliabilityCause, EventEnrollmentReliabilityResult, EventEnrollmentTransition,
};
use bacnet_types::enums::{EventState, EventType, ObjectType, Reliability};
use bacnet_types::primitives::ObjectIdentifier;

#[test]
fn complete_report_is_publicly_constructible() {
    let diagnostic = EventEnrollmentEvaluationDiagnostic {
        enrollment_oid: ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap(),
        stage: EventEnrollmentEvaluationStage::Reliability,
        outcome: EventEnrollmentEvaluationOutcome::ObservationUnavailable,
    };
    let report = EventEnrollmentEvaluationReport {
        transitions: vec![],
        reliability_results: vec![],
        diagnostics: vec![diagnostic],
    };
    assert_eq!(report.diagnostics, vec![diagnostic]);
    assert!(!diagnostic.outcome.is_failure());
    assert!(EventEnrollmentEvaluationOutcome::Rejected.is_failure());
    assert!(EventEnrollmentEvaluationOutcome::LandedAfterError.is_failure());
}

#[test]
fn reliability_result_is_constructible_and_derives_event_type() {
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
fn transition_and_complete_report_are_publicly_constructible() {
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
    let report = EventEnrollmentEvaluationReport {
        transitions: vec![transition],
        reliability_results: vec![],
        diagnostics: vec![],
    };

    assert_eq!(report.transitions[0].enrollment_oid, enrollment_oid);
    assert_eq!(report.transitions[0].monitored_oid, monitored_oid);
    assert!(report.reliability_results.is_empty());
    assert!(report.diagnostics.is_empty());
}

#[test]
fn unavailable_observation_is_not_an_ordinary_no_transition() {
    use bacnet_objects::analog::AnalogInputObject;
    use bacnet_objects::database::ObjectDatabase;
    use bacnet_objects::event_enrollment::EventEnrollmentObject;
    use bacnet_objects::traits::BACnetObject;
    use bacnet_types::constructed::FaultParameters;
    use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
    use bacnet_types::enums::PropertyIdentifier;

    let mut db = ObjectDatabase::new();
    let missing_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 999).unwrap();
    let mut enrollment =
        EventEnrollmentObject::new(303, "EE-missing", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    enrollment.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        missing_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    enrollment.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 0.0,
        high_limit: 10.0,
        deadband: 1.0,
    });
    enrollment.set_fault_parameters(Some(FaultParameters::FaultNone));
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let public_properties = [
        PropertyIdentifier::RELIABILITY,
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyIdentifier::EVENT_TIME_STAMPS,
    ];
    let before: Vec<_> = public_properties
        .iter()
        .map(|p| {
            db.get(&enrollment_oid)
                .unwrap()
                .read_property(*p, None)
                .unwrap()
        })
        .collect();
    let report = bacnet_server::event_enrollment::evaluate_event_enrollments_report(&mut db, 1);
    assert!(report.transitions.is_empty());
    assert!(report.reliability_results.is_empty());
    assert_eq!(
        report.diagnostics,
        vec![EventEnrollmentEvaluationDiagnostic {
            enrollment_oid,
            stage: EventEnrollmentEvaluationStage::Reliability,
            outcome: EventEnrollmentEvaluationOutcome::ObservationUnavailable,
        }]
    );
    let after: Vec<_> = public_properties
        .iter()
        .map(|p| {
            db.get(&enrollment_oid)
                .unwrap()
                .read_property(*p, None)
                .unwrap()
        })
        .collect();
    assert_eq!(after, before);
    assert!(
        report
            .diagnostics
            .iter()
            .all(|d| d.outcome != EventEnrollmentEvaluationOutcome::NoTransition),
        "unavailable observation must not be projected as ordinary NoTransition: {report:?}"
    );

    // Restoring a valid in-range observation is an ordinary no-transition pass.
    let mut target = AnalogInputObject::new(999, "restored", 62).unwrap();
    target.set_present_value(5.0);
    db.add(Box::new(target)).unwrap();
    let restored = bacnet_server::event_enrollment::evaluate_event_enrollments_report(&mut db, 1);
    assert!(restored.transitions.is_empty());
    assert!(restored.reliability_results.is_empty());
    assert!(restored
        .diagnostics
        .iter()
        .any(|d| d.outcome == EventEnrollmentEvaluationOutcome::NoTransition));
    assert!(!restored
        .diagnostics
        .iter()
        .any(|d| d.outcome == EventEnrollmentEvaluationOutcome::ObservationUnavailable));
}
