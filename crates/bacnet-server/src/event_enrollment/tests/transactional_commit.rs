use super::integration::{indexed_reference_value, ReferenceValueObject};
use super::*;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, ChangeOfValueCriteria,
};
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};
use std::sync::atomic::Ordering;
use std::sync::Arc;

struct FixedClock(ClockFrame);

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(self.0)
    }
}

fn timestamp_at(
    db: &ObjectDatabase,
    enrollment_oid: &ObjectIdentifier,
    index: u32,
) -> BACnetTimeStamp {
    let PropertyValue::ApplicationData(bytes) = db
        .get(enrollment_oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(index))
        .unwrap()
    else {
        panic!("Event_Time_Stamps slot must be an encoded timestamp");
    };
    let (timestamp, end) = decode_timestamp_choice(&bytes, 0).unwrap();
    assert_eq!(end, bytes.len());
    timestamp
}

#[test]
fn stock_transition_commits_timestamp_before_report_token_escapes() {
    let (mut db, enrollment_oid, monitored_oid) = setup_out_of_range(90.0, 80.0, 20.0, 2.0);
    let mut notification_class = NotificationClass::new(7, "NC-7").unwrap();
    notification_class.ack_required = [true, false, false];
    db.add(Box::new(notification_class)).unwrap();
    db.get_mut(&enrollment_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(7),
            None,
        )
        .unwrap();

    let report = evaluate_event_enrollments_report(&mut db, 1);

    assert_eq!(report.transitions.len(), 1);
    assert_eq!(acked_transitions(&db, &enrollment_oid), 0b110);
    assert_eq!(
        timestamp_at(&db, &enrollment_oid, 1),
        BACnetTimeStamp::SequenceNumber(0)
    );
    assert_eq!(db.reserve_event_sequence_number().number(), 1);

    let monitored = db.get_mut(&monitored_oid).unwrap();
    monitored
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    monitored
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            None,
        )
        .unwrap();
    let report = evaluate_event_enrollments_report(&mut db, 1);
    assert_eq!(report.transitions.len(), 1);
    assert_eq!(report.transitions[0].change.to, EventState::NORMAL);
    assert_eq!(acked_transitions(&db, &enrollment_oid), 0b110);
    assert_eq!(
        timestamp_at(&db, &enrollment_oid, 2),
        BACnetTimeStamp::SequenceNumber(0)
    );
    assert_eq!(
        timestamp_at(&db, &enrollment_oid, 3),
        BACnetTimeStamp::SequenceNumber(1)
    );
    assert_eq!(db.reserve_event_sequence_number().number(), 2);
}

#[test]
fn stock_transition_commits_exact_device_clock_datetime() {
    let (mut db, enrollment_oid, _) = setup_out_of_range(90.0, 80.0, 20.0, 2.0);
    let frame = ClockFrame {
        local_date: Date {
            year: 124,
            month: 2,
            day: 29,
            day_of_week: 4,
        },
        local_time: Time {
            hour: 12,
            minute: 34,
            second: 56,
            hundredths: 78,
        },
        utc_offset: 300,
        daylight_savings_status: true,
    };
    db.set_clock_reader(Some(Arc::new(FixedClock(frame))));

    let report = evaluate_event_enrollments_report(&mut db, 1);

    assert_eq!(report.transitions.len(), 1);
    assert_eq!(
        timestamp_at(&db, &enrollment_oid, 1),
        BACnetTimeStamp::DateTime {
            date: frame.local_date,
            time: frame.local_time,
        }
    );
    assert_eq!(db.reserve_event_sequence_number().number(), 0);
}

fn acked_transitions(db: &ObjectDatabase, enrollment_oid: &ObjectIdentifier) -> u8 {
    let PropertyValue::BitString { data, .. } = db
        .get(enrollment_oid)
        .unwrap()
        .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
        .unwrap()
    else {
        panic!("Acked_Transitions must be a bit string");
    };
    bacnet_types::bitstring::unpack_octet(&data, 3)
}

#[test]
fn same_state_transition_still_commits_ack_and_history() {
    let mut db = ObjectDatabase::new();
    let mut monitored = AnalogInputObject::new(31, "AI-COV", 62).unwrap();
    monitored.set_present_value(10.0);
    let monitored_oid = monitored.object_identifier();
    db.add(Box::new(monitored)).unwrap();

    let mut enrollment =
        EventEnrollmentObject::new(31, "EE-COV", EventType::CHANGE_OF_VALUE.to_raw()).unwrap();
    enrollment.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        monitored_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    enrollment.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
    });
    enrollment.set_notification_class(31);
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let mut notification_class = NotificationClass::new(31, "NC-COV").unwrap();
    notification_class.ack_required = [false, false, true];
    db.add(Box::new(notification_class)).unwrap();

    assert!(evaluate_event_enrollments_report(&mut db, 1)
        .transitions
        .is_empty());
    let monitored = db.get_mut(&monitored_oid).unwrap();
    monitored
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    monitored
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(20.0),
            None,
        )
        .unwrap();

    let report = evaluate_event_enrollments_report(&mut db, 1);
    assert_eq!(report.transitions.len(), 1);
    assert_eq!(
        report.transitions[0].change,
        EventStateChange {
            from: EventState::NORMAL,
            to: EventState::NORMAL,
        }
    );
    assert_eq!(acked_transitions(&db, &enrollment_oid), 0b011);
    assert_eq!(
        timestamp_at(&db, &enrollment_oid, 3),
        BACnetTimeStamp::SequenceNumber(0)
    );
    assert_eq!(db.reserve_event_sequence_number().number(), 1);
}

fn setup_counted_delayed_enrollment() -> (
    ObjectDatabase,
    ObjectIdentifier,
    ObjectIdentifier,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let mut db = ObjectDatabase::new();
    let mut monitored = AnalogValueObject::new(41, "AV-counted", 62).unwrap();
    for index in [1, 2] {
        monitored
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(90.0),
                Some(index),
            )
            .unwrap();
    }
    let monitored_oid = monitored.object_identifier();
    db.add(Box::new(monitored)).unwrap();

    let enrollment = ReferenceValueObject::new(Some(indexed_reference_value(monitored_oid, 1)));
    let state_write_count = enrollment.state_write_count.clone();
    let parameters_readable = enrollment.event_parameters_readable.clone();
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    (
        db,
        enrollment_oid,
        monitored_oid,
        state_write_count,
        parameters_readable,
    )
}

#[test]
fn valid_retarget_coalesces_reset_and_full_delay_reseed_to_one_state_write() {
    let (mut db, enrollment_oid, monitored_oid, state_write_count, _) =
        setup_counted_delayed_enrollment();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    state_write_count.store(0, Ordering::SeqCst);

    db.get_mut(&enrollment_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            indexed_reference_value(monitored_oid, 2),
            None,
        )
        .unwrap();
    let report = evaluate_event_enrollments_report(&mut db, 1);

    assert!(report.transitions.is_empty());
    assert_eq!(state_write_count.load(Ordering::SeqCst), 1);
    let pending = db
        .get(&enrollment_oid)
        .unwrap()
        .enrollment_eval_state_internal()
        .unwrap()
        .pending
        .unwrap();
    assert_eq!(pending.remaining, 2, "retarget starts the complete delay");
}

#[test]
fn later_unusable_input_commits_cancellation_once() {
    let (mut db, enrollment_oid, _, state_write_count, parameters_readable) =
        setup_counted_delayed_enrollment();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    state_write_count.store(0, Ordering::SeqCst);
    parameters_readable.store(false, Ordering::SeqCst);

    let report = evaluate_event_enrollments_report(&mut db, 1);

    assert!(report.transitions.is_empty());
    assert_eq!(state_write_count.load(Ordering::SeqCst), 1);
    assert!(db
        .get(&enrollment_oid)
        .unwrap()
        .enrollment_eval_state_internal()
        .unwrap()
        .pending
        .is_none());
    assert!(report
        .diagnostics
        .contains(&EventEnrollmentEvaluationDiagnostic {
            enrollment_oid,
            stage: EventEnrollmentEvaluationStage::EvaluationState,
            outcome: EventEnrollmentEvaluationOutcome::CancellationCommitted,
        }));
}

#[test]
fn source_and_state_rejection_never_claims_cancellation_committed() {
    let mut db = ObjectDatabase::new();
    let enrollment = ReferenceValueObject::new(None);
    enrollment.source_writable.store(false, Ordering::SeqCst);
    enrollment.state_writable.store(false, Ordering::SeqCst);
    let state_write_count = Arc::clone(&enrollment.state_write_count);
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let mut update = EnrollmentUpdate::default();
    update.set_eval_source(Some((
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 9).unwrap(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    )));
    update.cancel_pending(EventEnrollmentEvalState::default());
    let mut updates = std::collections::HashMap::new();
    updates.insert(enrollment_oid, update);

    let report = apply_updates(
        &mut db,
        &[enrollment_oid],
        updates,
        &std::collections::HashSet::new(),
    );

    assert!(report.transitions.is_empty());
    assert_eq!(state_write_count.load(Ordering::SeqCst), 1);
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
    assert!(!report.diagnostics.iter().any(|diagnostic| {
        diagnostic.outcome == EventEnrollmentDetailedEvaluationOutcome::CancellationCommitted
    }));
    assert!(db.enrollment_eval_state_invalidated(&enrollment_oid));
}

#[test]
fn private_state_failure_is_reported_and_suppresses_transition() {
    let mut db = ObjectDatabase::new();
    let mut monitored = AnalogValueObject::new(42, "AV-state-failure", 62).unwrap();
    monitored
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
        )
        .unwrap();
    let monitored_oid = monitored.object_identifier();
    db.add(Box::new(monitored)).unwrap();
    let enrollment = ReferenceValueObject::new(Some(indexed_reference_value(monitored_oid, 1)));
    enrollment.state_writable.store(false, Ordering::SeqCst);
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let report = evaluate_event_enrollments_report(&mut db, 1);
    assert!(report.transitions.is_empty());
    assert!(report
        .diagnostics
        .contains(&EventEnrollmentEvaluationDiagnostic {
            enrollment_oid,
            stage: EventEnrollmentEvaluationStage::EvaluationState,
            outcome: EventEnrollmentEvaluationOutcome::Rejected,
        }));
    assert!(db.enrollment_eval_state_invalidated(&enrollment_oid));
}

#[test]
fn unsupported_atomic_hook_fails_closed_without_consuming_sequence() {
    let mut db = ObjectDatabase::new();
    let mut monitored = AnalogValueObject::new(43, "AV-unsupported-hook", 62).unwrap();
    monitored
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
        )
        .unwrap();
    let monitored_oid = monitored.object_identifier();
    db.add(Box::new(monitored)).unwrap();
    let mut enrollment = ReferenceValueObject::new(Some(indexed_reference_value(monitored_oid, 1)));
    enrollment
        .inner
        .set_event_parameters(BACnetEventParameter::OutOfRange {
            time_delay: 0,
            low_limit: 20.0,
            high_limit: 80.0,
            deadband: 2.0,
        });
    enrollment.atomic_commit_supported = false;
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let report = evaluate_event_enrollments_report(&mut db, 1);
    assert!(report.transitions.is_empty());
    assert!(report
        .diagnostics
        .contains(&EventEnrollmentEvaluationDiagnostic {
            enrollment_oid,
            stage: EventEnrollmentEvaluationStage::EventTransition,
            outcome: EventEnrollmentEvaluationOutcome::Rejected,
        }));
    assert!(db.enrollment_eval_state_invalidated(&enrollment_oid));
    assert_eq!(db.reserve_event_sequence_number().number(), 0);
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}
