use super::*;

#[test]
fn intrinsic_proposals_and_commit_results_survive_wrapping() {
    use bacnet_objects::analog::AnalogInputObject;
    use bacnet_objects::event::{EventTransitionCommit, EventTransitionCommitError};
    use bacnet_types::enums::EventState;
    use bacnet_types::primitives::BACnetTimeStamp;

    for delay in [0, 1] {
        let mut input = AnalogInputObject::new(1, "wrapped input", 0).unwrap();
        input.set_present_value(81.0);
        let mut object: Box<dyn BACnetObject> = Box::new(input);
        for (property, value) in [
            (PropertyIdentifier::HIGH_LIMIT, PropertyValue::Real(80.0)),
            (
                PropertyIdentifier::LIMIT_ENABLE,
                PropertyValue::BitString {
                    unused_bits: 6,
                    data: vec![0xc0],
                },
            ),
            (
                PropertyIdentifier::TIME_DELAY,
                PropertyValue::Unsigned(delay),
            ),
        ] {
            object.write_property(property, None, value, None).unwrap();
        }
        let owner = bacnet_objects::database::AuditOwnership::for_source(
            oid(ObjectType::DEVICE, 123),
            selected(),
        );
        source_reporter::install(&mut object, &owner).unwrap();
        let evaluated = object.evaluate_intrinsic_reporting();
        let proposal = if delay == 0 {
            evaluated.unwrap()
        } else {
            assert!(evaluated.is_none());
            object.tick_intrinsic_reporting().unwrap()
        };
        assert_eq!(
            read(object.as_ref(), PropertyIdentifier::EVENT_STATE),
            PropertyValue::Enumerated(EventState::NORMAL.to_raw())
        );
        let mut commit = EventTransitionCommit {
            coordinate: proposal.change.transition(),
            change: proposal.change,
            ack_required: true,
            timestamp: BACnetTimeStamp::SequenceNumber(42),
            message_text: Some("wrapped commit".into()),
        };
        commit.change.from = EventState::FAULT;
        assert_eq!(
            object.commit_event_transition_internal(commit.clone()),
            Err(EventTransitionCommitError::CurrentStateMismatch {
                expected: EventState::FAULT,
                actual: EventState::NORMAL
            })
        );
        assert_eq!(
            read(object.as_ref(), PropertyIdentifier::EVENT_STATE),
            PropertyValue::Enumerated(EventState::NORMAL.to_raw())
        );
        commit.change.from = EventState::NORMAL;
        object.commit_event_transition_internal(commit).unwrap();
        assert_eq!(
            read(object.as_ref(), PropertyIdentifier::EVENT_STATE),
            PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS, Some(1))
                .unwrap(),
            PropertyValue::CharacterString("wrapped commit".into())
        );
        assert!(object.evaluate_intrinsic_reporting().is_none());
        assert!(object.tick_intrinsic_reporting().is_none());
    }
}
