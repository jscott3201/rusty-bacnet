use super::*;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_types::constructed::{BACnetEventParameter, FaultParameters};
use bacnet_types::enums::Reliability;

fn assert_reliability_event_type(
    result: &EventEnrollmentReliabilityResult,
    from: EventState,
    to: EventState,
) {
    assert_eq!(result.state_change, Some(EventStateChange { from, to }));
    assert_eq!(
        result.event_type(EventType::OUT_OF_RANGE),
        Some(EventType::CHANGE_OF_RELIABILITY)
    );
}

#[test]
fn fault_entry_reentry_and_recovery_select_change_of_reliability() {
    let (mut db, enrollment_oid, target_oid) = setup_out_of_range(-1.0, 80.0, 20.0, 2.0);
    db.get_mut(&target_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    db.get_mut(&enrollment_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::FAULT_PARAMETERS,
            None,
            FaultParameters::FaultOutOfRange {
                min_normal: 0.0,
                max_normal: 10.0,
            }
            .encode_property_value(),
            None,
        )
        .unwrap();

    let entry = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(entry.reliability_results.len(), 1);
    assert_eq!(
        entry.reliability_results[0].new_reliability,
        Reliability::UNDER_RANGE
    );
    assert_reliability_event_type(
        &entry.reliability_results[0],
        EventState::NORMAL,
        EventState::FAULT,
    );

    db.get_mut(&target_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(11.0),
            None,
        )
        .unwrap();
    let reentry = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(reentry.reliability_results.len(), 1);
    assert_eq!(
        reentry.reliability_results[0].new_reliability,
        Reliability::OVER_RANGE
    );
    assert_reliability_event_type(
        &reentry.reliability_results[0],
        EventState::FAULT,
        EventState::FAULT,
    );

    db.get_mut(&target_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(5.0),
            None,
        )
        .unwrap();
    let recovery = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(recovery.reliability_results.len(), 1);
    assert_eq!(
        recovery.reliability_results[0].new_reliability,
        Reliability::NO_FAULT_DETECTED
    );
    assert_reliability_event_type(
        &recovery.reliability_results[0],
        EventState::FAULT,
        EventState::NORMAL,
    );
}

#[test]
fn malformed_reference_and_normal_transition_select_their_model_event_types() {
    let mut db = ObjectDatabase::new();
    let mut enrollment = EventEnrollmentObject::new(
        319,
        "EE-malformed-reference",
        EventType::OUT_OF_RANGE.to_raw(),
    )
    .unwrap();
    enrollment.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    db.add(Box::new(enrollment)).unwrap();

    let malformed = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(malformed.reliability_results.len(), 1);
    let result = &malformed.reliability_results[0];
    assert_eq!(result.monitored_oid, None);
    assert_eq!(result.new_reliability, Reliability::CONFIGURATION_ERROR);
    assert_reliability_event_type(result, EventState::NORMAL, EventState::FAULT);

    let (mut db, _, _) = setup_out_of_range(90.0, 80.0, 20.0, 2.0);
    let normal = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert!(normal.reliability_results.is_empty());
    assert_eq!(normal.transitions.len(), 1);
    assert_eq!(normal.transitions[0].event_type, EventType::OUT_OF_RANGE);
}
