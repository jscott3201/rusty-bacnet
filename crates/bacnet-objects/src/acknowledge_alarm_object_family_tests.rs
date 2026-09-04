use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::event::{EventStateChange, EventTransition, EventTransitionCommit};
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use crate::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};

#[derive(Debug, PartialEq)]
struct EventSnapshot {
    event_state: PropertyValue,
    acked_transitions: PropertyValue,
    time_stamps: PropertyValue,
    message_texts: PropertyValue,
}

fn target_objects() -> Vec<Box<dyn BACnetObject>> {
    vec![
        Box::new(BinaryInputObject::new(1, "BI-ack").unwrap()),
        Box::new(BinaryOutputObject::new(1, "BO-ack").unwrap()),
        Box::new(BinaryValueObject::new(1, "BV-ack").unwrap()),
        Box::new(MultiStateInputObject::new(1, "MSI-ack", 3).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "MSO-ack", 3).unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV-ack", 3).unwrap()),
    ]
}

fn snapshot(object: &dyn BACnetObject) -> EventSnapshot {
    EventSnapshot {
        event_state: object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        acked_transitions: object
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        time_stamps: object
            .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
            .unwrap(),
        message_texts: object
            .read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS, None)
            .unwrap(),
    }
}

fn acked_transitions(object: &dyn BACnetObject) -> u8 {
    let PropertyValue::BitString { data, .. } = object
        .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
        .unwrap()
    else {
        panic!("Acked_Transitions must be a bit string");
    };
    bacnet_types::bitstring::unpack_octet(&data, 3)
}

fn assert_protocol(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(matches!(
        error,
        Error::Protocol {
            class: actual_class,
            code: actual_code,
        } if actual_class == class.to_raw() as u32 && actual_code == code.to_raw() as u32
    ));
}

#[test]
fn binary_and_multistate_families_require_exact_idempotent_correlation() {
    let stamp = BACnetTimeStamp::SequenceNumber(42);

    for mut object in target_objects() {
        let object_name = object.object_name().to_owned();
        object
            .write_property(
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                None,
                PropertyValue::Boolean(true),
                None,
            )
            .unwrap();
        object
            .commit_event_transition_internal(EventTransitionCommit {
                change: EventStateChange {
                    from: EventState::NORMAL,
                    to: EventState::OFFNORMAL,
                },
                coordinate: EventTransition::ToOffnormal,
                ack_required: true,
                timestamp: stamp.clone(),
                message_text: Some(format!("{object_name} committed")),
            })
            .unwrap();

        assert_eq!(acked_transitions(&*object), 0b110, "{object_name}");
        let before = snapshot(&*object);

        let state_error = object
            .acknowledge_alarm_correlated_internal(
                EventState::HIGH_LIMIT,
                &BACnetTimeStamp::SequenceNumber(99),
            )
            .unwrap_err();
        assert_protocol(
            state_error,
            ErrorClass::SERVICES,
            ErrorCode::INVALID_EVENT_STATE,
        );
        assert_eq!(snapshot(&*object), before, "{object_name}");

        let timestamp_error = object
            .acknowledge_alarm_correlated_internal(
                EventState::OFFNORMAL,
                &BACnetTimeStamp::SequenceNumber(41),
            )
            .unwrap_err();
        assert_protocol(
            timestamp_error,
            ErrorClass::SERVICES,
            ErrorCode::INVALID_TIME_STAMP,
        );
        assert_eq!(snapshot(&*object), before, "{object_name}");

        object
            .acknowledge_alarm_correlated_internal(EventState::OFFNORMAL, &stamp)
            .unwrap();
        assert_eq!(acked_transitions(&*object), 0b111, "{object_name}");
        let after = snapshot(&*object);
        assert_eq!(after.event_state, before.event_state, "{object_name}");
        assert_eq!(after.time_stamps, before.time_stamps, "{object_name}");
        assert_eq!(after.message_texts, before.message_texts, "{object_name}");

        object
            .acknowledge_alarm_correlated_internal(EventState::OFFNORMAL, &stamp)
            .unwrap();
        assert_eq!(snapshot(&*object), after, "{object_name}");

        object
            .write_property(
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                None,
                PropertyValue::Boolean(false),
                None,
            )
            .unwrap();
        let disabled = snapshot(&*object);
        let error = object
            .acknowledge_alarm_correlated_internal(EventState::OFFNORMAL, &stamp)
            .unwrap_err();
        assert_protocol(error, ErrorClass::OBJECT, ErrorCode::NO_ALARM_CONFIGURED);
        assert_eq!(snapshot(&*object), disabled, "{object_name}");
    }
}
