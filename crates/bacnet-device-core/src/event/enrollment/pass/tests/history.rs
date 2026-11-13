//! What a delivery carries for its notification (#2319 PR 2b): the
//! timestamp and message text its commit wrote, read back under the same
//! guard, so a host sends it without reading the database again. One whose
//! history doesn't read back is suppressed, with the transition named.

use super::super::*;
use super::integration::{ReferenceValueObject, reference_value};
use super::*;
use crate::event::{MessageTexts, NotificationHistorySource, read_committed_history_snapshot};
use bacnet_objects::event::EventTransition;
use bacnet_types::constructed::FaultParameters;
use bacnet_types::primitives::BACnetTimeStamp;

#[test]
fn a_delivery_carries_the_history_its_commit_wrote() {
    let (mut db, ee_oid, _) = setup_out_of_range(90.0, 80.0, 20.0, 2.0);
    // Use up sequence number 0, which the unwritten slots also hold, so
    // the commit's own number tells them apart.
    let staged = crate::event::stage_event_timestamp(&db);
    crate::event::confirm_event_timestamp(&mut db, staged);
    let mut batch = evaluate_event_enrollments_for_delivery(&mut db, 1);
    let delivery = batch
        .deliveries
        .pop()
        .expect("HIGH_LIMIT with a REAL to send");
    let object = db.get(&ee_oid).unwrap();
    let written = read_committed_history_snapshot(
        object,
        EventTransition::ToOffnormal,
        MessageTexts::Optional,
    );
    assert_eq!(Some(&delivery.history_snapshot), written.as_ref());
    assert_eq!(
        delivery.history_snapshot.timestamp,
        BACnetTimeStamp::SequenceNumber(1)
    );
    assert_eq!(delivery.enrollment_oid(), ee_oid);
    assert!(delivery.distribute());

    let (snapshot, clock) = (delivery.history_snapshot.clone(), delivery.recipient_clock);
    let (values, ack_required) = (delivery.event_values.clone(), delivery.ack_required);
    let notification = delivery.into_notification().expect("a normal transition");
    assert_eq!(notification.change.to, EventState::HIGH_LIMIT);
    assert_eq!(notification.event_type, EventType::OUT_OF_RANGE);
    assert_eq!(notification.event_values, Some(values));
    assert_eq!(notification.ack_required, Some(ack_required));
    match notification.history_source {
        NotificationHistorySource::Committed {
            snapshot: sent,
            recipient_clock,
        } => {
            assert_eq!(sent, snapshot);
            assert_eq!(recipient_clock, clock);
        }
        _ => panic!("a delivery sends the history it committed"),
    }
}

/// An enrollment watching AI-1's Present_Value (90) with OUT_OF_RANGE over
/// 80, through the test object with its history reads as given, and with
/// `fault` as its Fault_Parameters.
fn probe(
    timestamps_readable: bool,
    message_texts_served: bool,
    fault: Option<FaultParameters>,
) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(90.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();
    let present_value = PropertyIdentifier::PRESENT_VALUE.to_raw();
    let mut ee =
        ReferenceValueObject::new(Some(reference_value(ai_oid, present_value, None, None)));
    ee.inner
        .set_event_parameters(BACnetEventParameter::OutOfRange {
            time_delay: 0,
            low_limit: 20.0,
            high_limit: 80.0,
            deadband: 2.0,
        });
    ee.timestamps_readable = timestamps_readable;
    ee.message_texts_served = message_texts_served;
    ee.inner.set_fault_parameters(fault);
    db.add(Box::new(ee)).unwrap();
    db
}

fn probe_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 999).unwrap()
}

#[test]
fn a_history_that_does_not_read_back_is_suppressed() {
    let mut db = probe(false, true, None);
    let batch = evaluate_event_enrollments_for_delivery(&mut db, 1);
    assert_eq!(batch.report.transitions.len(), 1, "it still commits");
    assert!(batch.deliveries.is_empty());
    let expected = SuppressedDelivery {
        enrollment_oid: probe_oid(),
        event_type: Some(EventType::OUT_OF_RANGE),
        cause: SuppressionCause::History(EventTransition::ToOffnormal),
    };
    assert_eq!(batch.suppressed, [expected]);
}

/// Event Enrollment serves Event_Message_Texts as an optional property: one
/// that doesn't serve it sends no text rather than nothing.
#[test]
fn a_delivery_without_message_texts_sends_no_text() {
    let mut db = probe(true, false, None);
    let batch = evaluate_event_enrollments_for_delivery(&mut db, 1);
    assert_eq!(batch.deliveries.len(), 1, "{:?}", batch.suppressed);
    assert_eq!(batch.deliveries[0].history_snapshot.message_text, None);
}

/// The Reliability path reads its history back too, after its commit: the
/// delivery carries what the enrollment reads at TO_FAULT after the pass.
#[test]
fn a_fault_whose_history_does_not_read_back_is_suppressed() {
    for timestamps_readable in [true, false] {
        let fault = FaultParameters::FaultOutOfRange {
            min_normal: 0.0,
            max_normal: 10.0,
        };
        let mut db = probe(timestamps_readable, true, Some(fault));
        // As above: sequence number 0 is used up, so the commit's is 1.
        let staged = crate::event::stage_event_timestamp(&db);
        crate::event::confirm_event_timestamp(&mut db, staged);
        let batch = evaluate_event_enrollments_for_delivery(&mut db, 1);
        assert_eq!(batch.report.reliability_results.len(), 1);
        if timestamps_readable {
            assert_eq!(batch.deliveries.len(), 1, "{:?}", batch.suppressed);
            let object = db.get(&probe_oid()).unwrap();
            let written = read_committed_history_snapshot(
                object,
                EventTransition::ToFault,
                MessageTexts::Optional,
            );
            let carried = &batch.deliveries[0].history_snapshot;
            assert_eq!(Some(carried), written.as_ref());
            assert_eq!(carried.timestamp, BACnetTimeStamp::SequenceNumber(1));
            continue;
        }
        assert!(batch.deliveries.is_empty());
        let expected = SuppressedDelivery {
            enrollment_oid: probe_oid(),
            event_type: None,
            cause: SuppressionCause::History(EventTransition::ToFault),
        };
        assert_eq!(batch.suppressed, [expected]);
    }
}
