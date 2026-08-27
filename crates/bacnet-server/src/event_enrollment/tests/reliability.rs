use super::integration::{indexed_reference_value, ReferenceValueObject};
use super::*;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
    FaultParameters,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, Reliability};
use bacnet_types::primitives::BACnetTimeStamp;
use std::borrow::Cow;
use std::sync::atomic::Ordering;

struct OptionalReliabilityTarget {
    oid: ObjectIdentifier,
    malformed_reliability: bool,
}

impl OptionalReliabilityTarget {
    fn new(instance: u32, malformed_reliability: bool) -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
            malformed_reliability,
        }
    }
}

impl BACnetObject for OptionalReliabilityTarget {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "optional-reliability-target"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, bacnet_types::error::Error> {
        if property == PropertyIdentifier::PRESENT_VALUE {
            Ok(PropertyValue::Real(90.0))
        } else if property == PropertyIdentifier::RELIABILITY && self.malformed_reliability {
            Ok(PropertyValue::Real(0.0))
        } else {
            Err(bacnet_types::error::Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            })
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), bacnet_types::error::Error> {
        Err(bacnet_types::error::Error::Encoding(
            "read-only test target".into(),
        ))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
}

fn setup(
    present_value: f32,
    fault_parameters: FaultParameters,
) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogInputObject::new(301, "AI-fault-target", 62).unwrap();
    target.set_present_value(present_value);
    target
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut enrollment =
        EventEnrollmentObject::new(301, "EE-fault", EventType::OUT_OF_RANGE.to_raw()).unwrap();
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
    enrollment.set_fault_parameters(Some(fault_parameters));
    enrollment.set_event_enable(0x07);
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    (db, enrollment_oid, target_oid)
}

fn write_target(
    db: &mut ObjectDatabase,
    target_oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) {
    db.get_mut(&target_oid)
        .unwrap()
        .write_property(property, None, value, None)
        .unwrap();
}

fn reliability(db: &ObjectDatabase, oid: ObjectIdentifier) -> Reliability {
    let PropertyValue::Enumerated(raw) = db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap()
    else {
        panic!("Reliability must be Enumerated");
    };
    Reliability::from_raw(raw)
}

fn event_state(db: &ObjectDatabase, oid: ObjectIdentifier) -> EventState {
    let PropertyValue::Enumerated(raw) = db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap()
    else {
        panic!("Event_State must be Enumerated");
    };
    EventState::from_raw(raw)
}

fn timestamp_at(db: &ObjectDatabase, oid: ObjectIdentifier, index: u32) -> BACnetTimeStamp {
    let PropertyValue::ApplicationData(bytes) = db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(index))
        .unwrap()
    else {
        panic!("Event_Time_Stamps slot must be encoded application data");
    };
    let (timestamp, end) = decode_timestamp_choice(&bytes, 0).unwrap();
    assert_eq!(end, bytes.len());
    timestamp
}

#[test]
fn configuration_precedes_monitored_reliability_and_monitored_precedes_algorithm() {
    let (mut db, enrollment_oid, target_oid) = setup(
        -1.0,
        FaultParameters::FaultCharacterString {
            fault_values: vec!["unsupported".into()],
        },
    );
    write_target(
        &mut db,
        target_oid,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw()),
    );

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(report.reliability_results.len(), 1);
    assert_eq!(
        report.reliability_results[0].new_reliability,
        Reliability::CONFIGURATION_ERROR
    );
    assert_eq!(
        report.reliability_results[0].cause,
        EventEnrollmentReliabilityCause::Configuration
    );
    assert_eq!(event_state(&db, enrollment_oid), EventState::FAULT);
    assert_eq!(
        timestamp_at(&db, enrollment_oid, 2),
        BACnetTimeStamp::SequenceNumber(0)
    );

    write_target(
        &mut db,
        target_oid,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
    );
    db.get_mut(&enrollment_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::FAULT_PARAMETERS,
            None,
            FaultParameters::FaultNone.encode_property_value(),
            None,
        )
        .unwrap();
    let recovery = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(recovery.reliability_results.len(), 1);
    assert_eq!(
        recovery.reliability_results[0].new_reliability,
        Reliability::NO_FAULT_DETECTED
    );
    assert_eq!(
        recovery.reliability_results[0].cause,
        EventEnrollmentReliabilityCause::Configuration
    );
    assert_eq!(event_state(&db, enrollment_oid), EventState::NORMAL);
    assert_eq!(
        timestamp_at(&db, enrollment_oid, 3),
        BACnetTimeStamp::SequenceNumber(1)
    );

    let (mut db, enrollment_oid, target_oid) = setup(
        -1.0,
        FaultParameters::FaultOutOfRange {
            min_normal: 0.0,
            max_normal: 10.0,
        },
    );
    write_target(
        &mut db,
        target_oid,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw()),
    );
    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(
        report.reliability_results[0].new_reliability,
        Reliability::MONITORED_OBJECT_FAULT
    );
    assert_eq!(
        report.reliability_results[0].cause,
        EventEnrollmentReliabilityCause::MonitoredObject
    );

    write_target(
        &mut db,
        target_oid,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
    );
    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(report.reliability_results.len(), 1);
    assert_eq!(
        report.reliability_results[0].new_reliability,
        Reliability::UNDER_RANGE
    );
    assert_eq!(
        report.reliability_results[0].state_change,
        Some(EventStateChange {
            from: EventState::FAULT,
            to: EventState::FAULT,
        })
    );
    assert_eq!(reliability(&db, enrollment_oid), Reliability::UNDER_RANGE);
}

#[test]
fn every_deferred_fault_alternative_commits_configuration_error() {
    let reference = BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 301).unwrap(),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    );
    let alternatives = [
        FaultParameters::FaultCharacterString {
            fault_values: vec!["unsupported".into()],
        },
        FaultParameters::FaultExtended {
            vendor_id: 1,
            extended_fault_type: 2,
            parameters: vec![0x21, 0x03],
        },
        FaultParameters::FaultLifeSafety {
            fault_values: vec![1],
            mode_for_reference: reference.clone(),
        },
        FaultParameters::FaultState {
            fault_values: vec![BACnetPropertyStates::BooleanValue(true)],
        },
        FaultParameters::FaultListed { reference },
    ];

    for parameters in alternatives {
        let (mut db, enrollment_oid, _) = setup(50.0, parameters);
        let report = evaluate_event_enrollments_detailed_report(&mut db, 1);

        assert_eq!(report.reliability_results.len(), 1);
        assert_eq!(
            report.reliability_results[0].new_reliability,
            Reliability::CONFIGURATION_ERROR
        );
        assert_eq!(
            report.reliability_results[0].cause,
            EventEnrollmentReliabilityCause::Configuration
        );
        assert_eq!(event_state(&db, enrollment_oid), EventState::FAULT);
    }
}

#[test]
fn fault_none_falls_through_to_the_normal_event_algorithm() {
    let (mut db, enrollment_oid, _) = setup(90.0, FaultParameters::FaultNone);

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);

    assert!(report.reliability_results.is_empty());
    assert_eq!(report.transitions.len(), 1);
    assert_eq!(report.transitions[0].change.to, EventState::HIGH_LIMIT);
    assert_eq!(
        reliability(&db, enrollment_oid),
        Reliability::NO_FAULT_DETECTED
    );
}

#[test]
fn absent_optional_fault_parameters_preserves_custom_normal_event_evaluation() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(321, "AV-no-fault-parameters", 62).unwrap();
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
    enrollment.fault_parameters_supported = false;
    enrollment
        .inner
        .set_event_parameters(BACnetEventParameter::OutOfRange {
            time_delay: 0,
            low_limit: 20.0,
            high_limit: 80.0,
            deadband: 2.0,
        });
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);

    assert!(report.reliability_results.is_empty());
    assert_eq!(report.transitions.len(), 1);
    assert_eq!(report.transitions[0].change.to, EventState::HIGH_LIMIT);
    assert_eq!(
        reliability(&db, enrollment_oid),
        Reliability::NO_FAULT_DETECTED
    );
}

#[test]
fn absent_optional_target_reliability_falls_through_but_malformed_type_is_configuration_error() {
    for malformed in [false, true] {
        let mut db = ObjectDatabase::new();
        let target = OptionalReliabilityTarget::new(320 + u32::from(malformed), malformed);
        let target_oid = target.object_identifier();
        db.add(Box::new(target)).unwrap();

        let mut enrollment = EventEnrollmentObject::new(
            320 + u32::from(malformed),
            "EE-optional-reliability",
            EventType::OUT_OF_RANGE.to_raw(),
        )
        .unwrap();
        enrollment.set_object_property_reference(Some(
            BACnetDeviceObjectPropertyReference::new_local(
                target_oid,
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            ),
        ));
        enrollment.set_event_parameters(BACnetEventParameter::OutOfRange {
            time_delay: 0,
            low_limit: 20.0,
            high_limit: 80.0,
            deadband: 2.0,
        });
        enrollment.set_fault_parameters(Some(FaultParameters::FaultNone));
        let enrollment_oid = enrollment.object_identifier();
        db.add(Box::new(enrollment)).unwrap();

        let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
        if malformed {
            assert_eq!(
                report.reliability_results[0].new_reliability,
                Reliability::CONFIGURATION_ERROR
            );
        } else {
            assert!(report.reliability_results.is_empty());
            assert_eq!(report.transitions.len(), 1);
            assert_eq!(event_state(&db, enrollment_oid), EventState::HIGH_LIMIT);
        }
    }
}

#[test]
fn local_status_flags_fault_enters_member_fault_and_recovers() {
    let mut status_source = AnalogInputObject::new(302, "AI-status-source", 62).unwrap();
    status_source
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    status_source
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::PROCESS_ERROR.to_raw()),
            None,
        )
        .unwrap();
    let status_oid = status_source.object_identifier();

    let (mut db, enrollment_oid, _) = setup(
        50.0,
        FaultParameters::FaultStatusFlags {
            reference: BACnetDeviceObjectPropertyReference::new_local(
                status_oid,
                PropertyIdentifier::STATUS_FLAGS.to_raw(),
            ),
        },
    );
    db.add(Box::new(status_source)).unwrap();

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(
        report.reliability_results[0].new_reliability,
        Reliability::MEMBER_FAULT
    );
    assert_eq!(event_state(&db, enrollment_oid), EventState::FAULT);

    write_target(
        &mut db,
        status_oid,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
    );
    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(
        report.reliability_results[0].new_reliability,
        Reliability::NO_FAULT_DETECTED
    );
    assert_eq!(event_state(&db, enrollment_oid), EventState::NORMAL);
}

#[test]
fn normalized_out_of_range_holds_reindicates_changed_cause_and_recovers() {
    let (mut db, enrollment_oid, target_oid) = setup(
        -1.0,
        FaultParameters::FaultOutOfRange {
            min_normal: 0.0,
            max_normal: 10.0,
        },
    );

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(
        report.reliability_results[0].new_reliability,
        Reliability::UNDER_RANGE
    );
    assert_eq!(
        timestamp_at(&db, enrollment_oid, 2),
        BACnetTimeStamp::SequenceNumber(0)
    );
    assert!(evaluate_event_enrollments_detailed_report(&mut db, 1)
        .reliability_results
        .is_empty());
    assert_eq!(db.reserve_event_sequence_number().number(), 1);

    write_target(
        &mut db,
        target_oid,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(11.0),
    );
    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(
        report.reliability_results[0].state_change,
        Some(EventStateChange {
            from: EventState::FAULT,
            to: EventState::FAULT,
        })
    );
    assert_eq!(
        report.reliability_results[0].new_reliability,
        Reliability::OVER_RANGE
    );
    assert_eq!(
        timestamp_at(&db, enrollment_oid, 2),
        BACnetTimeStamp::SequenceNumber(1)
    );

    write_target(
        &mut db,
        target_oid,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(5.0),
    );
    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(
        report.reliability_results[0].new_reliability,
        Reliability::NO_FAULT_DETECTED
    );
    assert_eq!(
        timestamp_at(&db, enrollment_oid, 3),
        BACnetTimeStamp::SequenceNumber(2)
    );
}

#[test]
fn missing_target_is_observation_unavailable_without_mutation() {
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

    let before_timestamp = timestamp_at(&db, enrollment_oid, 2);
    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);

    assert!(report.reliability_results.is_empty());
    assert!(report.transitions.is_empty());
    assert!(report
        .diagnostics
        .contains(&EventEnrollmentDetailedEvaluationDiagnostic {
            enrollment_oid,
            stage: EventEnrollmentDetailedEvaluationStage::Reliability,
            outcome: EventEnrollmentDetailedEvaluationOutcome::ObservationUnavailable,
        }));
    assert_eq!(
        reliability(&db, enrollment_oid),
        Reliability::NO_FAULT_DETECTED
    );
    assert_eq!(event_state(&db, enrollment_oid), EventState::NORMAL);
    assert_eq!(timestamp_at(&db, enrollment_oid, 2), before_timestamp);
}

#[test]
fn fault_recovery_resets_cached_event_state_once_and_restarts_full_delay_next_pass() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(304, "AV-recovery", 62).unwrap();
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
    let state_writes = enrollment.state_write_count.clone();
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    assert!(evaluate_event_enrollments_detailed_report(&mut db, 1)
        .transitions
        .is_empty());

    write_target(
        &mut db,
        target_oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    );
    write_target(
        &mut db,
        target_oid,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw()),
    );
    assert_eq!(
        evaluate_event_enrollments_detailed_report(&mut db, 1).reliability_results[0]
            .new_reliability,
        Reliability::MONITORED_OBJECT_FAULT
    );

    write_target(
        &mut db,
        target_oid,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
    );
    state_writes.store(0, Ordering::SeqCst);
    let recovery = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(recovery.reliability_results.len(), 1);
    assert_eq!(state_writes.load(Ordering::SeqCst), 1);
    assert!(db
        .get(&enrollment_oid)
        .unwrap()
        .enrollment_eval_state_internal()
        .unwrap()
        .pending
        .is_none());

    let next = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert!(next.transitions.is_empty());
    assert!(next.reliability_results.is_empty());
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_state_internal()
            .unwrap()
            .pending
            .unwrap()
            .remaining,
        2
    );
}

#[test]
fn rejected_and_mutating_custom_hooks_do_not_escape_tokens_or_consume_sequences() {
    for mutate_then_error in [false, true] {
        let mut db = ObjectDatabase::new();
        let mut target = AnalogValueObject::new(305, "AV-hook", 62).unwrap();
        target
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(90.0),
                Some(1),
            )
            .unwrap();
        target
            .write_property(
                PropertyIdentifier::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(true),
                None,
            )
            .unwrap();
        target
            .write_property(
                PropertyIdentifier::RELIABILITY,
                None,
                PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw()),
                None,
            )
            .unwrap();
        let target_oid = target.object_identifier();
        db.add(Box::new(target)).unwrap();

        let mut enrollment =
            ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 1)));
        enrollment.reliability_commit_supported = mutate_then_error;
        enrollment.reliability_error_after_write = mutate_then_error;
        let enrollment_oid = enrollment.object_identifier();
        db.add(Box::new(enrollment)).unwrap();

        let report = evaluate_event_enrollments_detailed_report(&mut db, 1);
        assert!(report.reliability_results.is_empty());
        assert!(report.transitions.is_empty());
        assert!(report
            .diagnostics
            .contains(&EventEnrollmentDetailedEvaluationDiagnostic {
                enrollment_oid,
                stage: EventEnrollmentDetailedEvaluationStage::Reliability,
                outcome: if mutate_then_error {
                    EventEnrollmentDetailedEvaluationOutcome::LandedAfterError
                } else {
                    EventEnrollmentDetailedEvaluationOutcome::Rejected
                },
            }));
        assert!(db.enrollment_eval_state_invalidated(&enrollment_oid));
        assert_eq!(db.reserve_event_sequence_number().number(), 0);
    }
}

#[test]
fn held_configuration_error_clears_landed_after_error_invalidation_once() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(306, "AV-held-config", 62).unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(1),
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

    let first = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert!(first
        .diagnostics
        .contains(&EventEnrollmentDetailedEvaluationDiagnostic {
            enrollment_oid,
            stage: EventEnrollmentDetailedEvaluationStage::Reliability,
            outcome: EventEnrollmentDetailedEvaluationOutcome::LandedAfterError,
        }));
    assert!(db.enrollment_eval_state_invalidated(&enrollment_oid));
    assert_eq!(
        reliability(&db, enrollment_oid),
        Reliability::CONFIGURATION_ERROR
    );
    assert_eq!(event_state(&db, enrollment_oid), EventState::FAULT);

    state_writes.store(0, Ordering::SeqCst);
    let held = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert!(held.reliability_results.is_empty());
    assert_eq!(state_writes.load(Ordering::SeqCst), 1);
    assert!(!db.enrollment_eval_state_invalidated(&enrollment_oid));

    let held_again = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert!(held_again.reliability_results.is_empty());
    assert_eq!(state_writes.load(Ordering::SeqCst), 1);
    assert!(!db.enrollment_eval_state_invalidated(&enrollment_oid));
}
