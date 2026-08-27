use super::super::*;
use crate::event::{
    EventStateChange, EventTransition, EventTransitionCommit, EventTransitionCommitError,
};
use bacnet_encoding::primitives::decode_timestamp_choice;
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

fn snapshot(object: &EventEnrollmentObject) -> (PropertyValue, PropertyValue, PropertyValue) {
    (
        object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
            .unwrap(),
    )
}

#[test]
fn stock_enrollment_atomically_commits_state_ack_and_exact_history_coordinate() {
    let mut object = EventEnrollmentObject::new(1, "EE-atomic", 0).unwrap();

    object
        .commit_event_transition_internal(EventTransitionCommit {
            change: EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            coordinate: EventTransition::ToOffnormal,
            ack_required: true,
            timestamp: BACnetTimeStamp::SequenceNumber(41),
            message_text: None,
        })
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x60],
        }
    );
    assert_eq!(
        timestamp_at(&object, 1),
        BACnetTimeStamp::SequenceNumber(41)
    );
    assert_eq!(timestamp_at(&object, 2), BACnetTimeStamp::SequenceNumber(0));
    assert_eq!(timestamp_at(&object, 3), BACnetTimeStamp::SequenceNumber(0));

    object
        .commit_event_transition_internal(EventTransitionCommit {
            change: EventStateChange {
                from: EventState::HIGH_LIMIT,
                to: EventState::HIGH_LIMIT,
            },
            coordinate: EventTransition::ToOffnormal,
            ack_required: false,
            timestamp: BACnetTimeStamp::SequenceNumber(42),
            message_text: None,
        })
        .unwrap();
    assert_eq!(
        timestamp_at(&object, 1),
        BACnetTimeStamp::SequenceNumber(42)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xE0],
        }
    );
}

#[test]
fn rejected_stock_commit_changes_none_of_the_three_properties() {
    let mut object = EventEnrollmentObject::new(2, "EE-reject", 0).unwrap();
    let before = snapshot(&object);

    assert_eq!(
        object.commit_event_transition_internal(EventTransitionCommit {
            change: EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            coordinate: EventTransition::ToNormal,
            ack_required: true,
            timestamp: BACnetTimeStamp::SequenceNumber(99),
            message_text: None,
        }),
        Err(EventTransitionCommitError::CoordinateTargetMismatch {
            coordinate: EventTransition::ToNormal,
            target: EventState::HIGH_LIMIT,
        })
    );
    assert_eq!(snapshot(&object), before);
}
