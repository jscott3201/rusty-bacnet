use super::integration::{indexed_reference_value, ReferenceValueObject};
use super::*;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_types::constructed::FaultParameters;
use bacnet_types::enums::Reliability;
use bacnet_types::primitives::BACnetTimeStamp;
use std::sync::atomic::Ordering;

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

fn acked_transitions(db: &ObjectDatabase, oid: ObjectIdentifier) -> u8 {
    let PropertyValue::BitString { data, .. } = db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
        .unwrap()
    else {
        panic!("Acked_Transitions must be BitString");
    };
    bacnet_types::bitstring::unpack_octet(&data, 3)
}

#[test]
fn source_rejection_does_not_suppress_changed_reliability_fault_reentry() {
    let mut db = ObjectDatabase::new();
    let mut notification_class = NotificationClass::new(32, "NC-reliability-source").unwrap();
    notification_class.ack_required = [false, true, false];
    db.add(Box::new(notification_class)).unwrap();

    let mut target = AnalogValueObject::new(307, "AV-reliability-source", 62).unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(-1.0),
            Some(1),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 1)));
    enrollment.source_writable.store(false, Ordering::SeqCst);
    enrollment
        .inner
        .set_fault_parameters(Some(FaultParameters::FaultOutOfRange {
            min_normal: 0.0,
            max_normal: 10.0,
        }));
    enrollment
        .inner
        .write_property(
            PropertyIdentifier::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(32),
            None,
        )
        .unwrap();
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let first = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(first.reliability_results.len(), 1);
    assert_eq!(
        first.reliability_results[0].new_reliability,
        Reliability::UNDER_RANGE
    );
    assert!(first
        .diagnostics
        .contains(&EventEnrollmentDetailedEvaluationDiagnostic {
            enrollment_oid,
            stage: EventEnrollmentDetailedEvaluationStage::EvaluationSource,
            outcome: EventEnrollmentDetailedEvaluationOutcome::Rejected,
        }));
    assert_eq!(
        timestamp_at(&db, enrollment_oid, 2),
        BACnetTimeStamp::SequenceNumber(0)
    );
    assert_eq!(acked_transitions(&db, enrollment_oid), 0b101);

    db.get_mut(&enrollment_oid)
        .unwrap()
        .set_acked_transitions_internal(0x02, true)
        .unwrap();
    db.get_mut(&target_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(11.0),
            Some(1),
        )
        .unwrap();

    let second = evaluate_event_enrollments_detailed_report(&mut db, 1);
    assert_eq!(second.reliability_results.len(), 1);
    assert_eq!(
        second.reliability_results[0].previous_reliability,
        Reliability::UNDER_RANGE
    );
    assert_eq!(
        second.reliability_results[0].new_reliability,
        Reliability::OVER_RANGE
    );
    assert_eq!(
        second.reliability_results[0].state_change,
        Some(EventStateChange {
            from: EventState::FAULT,
            to: EventState::FAULT,
        })
    );
    assert!(second
        .diagnostics
        .contains(&EventEnrollmentDetailedEvaluationDiagnostic {
            enrollment_oid,
            stage: EventEnrollmentDetailedEvaluationStage::EvaluationSource,
            outcome: EventEnrollmentDetailedEvaluationOutcome::Rejected,
        }));
    assert_eq!(reliability(&db, enrollment_oid), Reliability::OVER_RANGE);
    assert_eq!(
        timestamp_at(&db, enrollment_oid, 2),
        BACnetTimeStamp::SequenceNumber(1)
    );
    assert_eq!(acked_transitions(&db, enrollment_oid), 0b101);
    assert_eq!(db.reserve_event_sequence_number().number(), 2);
}
