use super::super::*;
use crate::event::{
    EventStateChange, EventTransition, EventTransitionCommit, EventTransitionCommitError,
};
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_types::enums::Reliability;
use bacnet_types::primitives::BACnetTimeStamp;

fn timestamp_at(object: &EventEnrollmentObject, index: u32) -> BACnetTimeStamp {
    let PropertyValue::ApplicationData(bytes) = object
        .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(index))
        .unwrap()
    else {
        panic!("Event_Time_Stamps slot must be encoded application data");
    };
    let (timestamp, end) = decode_timestamp_choice(&bytes, 0).unwrap();
    assert_eq!(end, bytes.len());
    timestamp
}

fn snapshot(object: &EventEnrollmentObject) -> [PropertyValue; 5] {
    [
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
            .unwrap(),
    ]
}

fn transition(from: EventState, to: EventState, timestamp: u16) -> EventTransitionCommit {
    EventTransitionCommit {
        change: EventStateChange { from, to },
        coordinate: EventTransition::for_target_state(to),
        ack_required: true,
        timestamp: BACnetTimeStamp::SequenceNumber(timestamp),
        message_text: None,
    }
}

#[test]
fn stock_reliability_commit_is_atomic_across_fault_reentry_and_recovery() {
    let mut object = EventEnrollmentObject::new(31, "EE-reliability", 0).unwrap();

    object
        .commit_event_enrollment_reliability_internal(EventEnrollmentReliabilityCommit {
            reliability: Reliability::UNDER_RANGE,
            transition: Some(transition(EventState::NORMAL, EventState::FAULT, 11)),
        })
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::UNDER_RANGE.to_raw())
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0xC0],
        }
    );
    assert_eq!(
        timestamp_at(&object, 2),
        BACnetTimeStamp::SequenceNumber(11)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xA0],
        }
    );

    object
        .commit_event_enrollment_reliability_internal(EventEnrollmentReliabilityCommit {
            reliability: Reliability::OVER_RANGE,
            transition: Some(transition(EventState::FAULT, EventState::FAULT, 12)),
        })
        .unwrap();
    assert_eq!(
        timestamp_at(&object, 2),
        BACnetTimeStamp::SequenceNumber(12)
    );

    object
        .commit_event_enrollment_reliability_internal(EventEnrollmentReliabilityCommit {
            reliability: Reliability::NO_FAULT_DETECTED,
            transition: Some(transition(EventState::FAULT, EventState::NORMAL, 13)),
        })
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0],
        }
    );
    assert_eq!(
        timestamp_at(&object, 3),
        BACnetTimeStamp::SequenceNumber(13)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x80],
        }
    );
}

#[test]
fn rejected_stock_reliability_commit_changes_nothing() {
    let mut object = EventEnrollmentObject::new(32, "EE-reliability-reject", 0).unwrap();
    let before = snapshot(&object);

    let mut invalid = transition(EventState::NORMAL, EventState::FAULT, 99);
    invalid.coordinate = EventTransition::ToNormal;
    assert_eq!(
        object.commit_event_enrollment_reliability_internal(EventEnrollmentReliabilityCommit {
            reliability: Reliability::CONFIGURATION_ERROR,
            transition: Some(invalid),
        }),
        Err(EventTransitionCommitError::CoordinateTargetMismatch {
            coordinate: EventTransition::ToNormal,
            target: EventState::FAULT,
        })
    );
    assert_eq!(snapshot(&object), before);
}

#[test]
fn network_reliability_write_remains_denied() {
    let mut object = EventEnrollmentObject::new(33, "EE-network-reliability", 0).unwrap();
    assert!(object
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw()),
            None,
        )
        .is_err());
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
}
