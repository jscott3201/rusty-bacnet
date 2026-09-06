use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::event::{EventStateChange, EventTransition, EventTransitionCommit};
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use crate::traits::BACnetObject;
use bacnet_types::enums::{EventState, EventType};
use bacnet_types::primitives::BACnetTimeStamp;

#[test]
fn all_nine_intrinsic_families_report_implied_algorithm_and_shared_commit_coordinate() {
    let mut objects: Vec<(Box<dyn BACnetObject>, EventType)> = vec![
        (
            Box::new(AnalogInputObject::new(1, "AI", 0).unwrap()),
            EventType::OUT_OF_RANGE,
        ),
        (
            Box::new(AnalogOutputObject::new(1, "AO", 0).unwrap()),
            EventType::OUT_OF_RANGE,
        ),
        (
            Box::new(AnalogValueObject::new(1, "AV", 0).unwrap()),
            EventType::OUT_OF_RANGE,
        ),
        (
            Box::new(BinaryInputObject::new(1, "BI").unwrap()),
            EventType::CHANGE_OF_STATE,
        ),
        (
            Box::new(BinaryOutputObject::new(1, "BO").unwrap()),
            EventType::COMMAND_FAILURE,
        ),
        (
            Box::new(BinaryValueObject::new(1, "BV").unwrap()),
            EventType::CHANGE_OF_STATE,
        ),
        (
            Box::new(MultiStateInputObject::new(1, "MSI", 3).unwrap()),
            EventType::CHANGE_OF_STATE,
        ),
        (
            Box::new(MultiStateOutputObject::new(1, "MSO", 3).unwrap()),
            EventType::COMMAND_FAILURE,
        ),
        (
            Box::new(MultiStateValueObject::new(1, "MSV", 3).unwrap()),
            EventType::CHANGE_OF_STATE,
        ),
    ];

    for (object, expected_event_type) in &mut objects {
        let capability = object
            .enrollment_summary_capability_internal()
            .expect("built-in family must opt in");
        assert_eq!(capability.event_type, *expected_event_type);
        assert_eq!(capability.last_transition, None);

        object
            .commit_event_transition_internal(EventTransitionCommit {
                change: EventStateChange {
                    from: EventState::NORMAL,
                    to: EventState::OFFNORMAL,
                },
                coordinate: EventTransition::ToOffnormal,
                ack_required: false,
                timestamp: BACnetTimeStamp::SequenceNumber(1),
                message_text: None,
            })
            .unwrap();
        let capability = object.enrollment_summary_capability_internal().unwrap();
        assert_eq!(capability.event_type, *expected_event_type);
        assert_eq!(
            capability.last_transition,
            Some(EventTransition::ToOffnormal)
        );
    }
}
