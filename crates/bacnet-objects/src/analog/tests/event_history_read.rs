//! Regression coverage for shared analog event-history reads (#235).

use super::super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::primitives::{Date, Time};

macro_rules! assert_seeded_event_history {
    ($object:expr, $label:literal) => {{
        let mut object = $object;
        object.event_history.time_stamps = [
            BACnetTimeStamp::SequenceNumber(11),
            BACnetTimeStamp::DateTime {
                date: Date {
                    year: 126,
                    month: 7,
                    day: 30,
                    day_of_week: 4,
                },
                time: Time {
                    hour: 12,
                    minute: 34,
                    second: 56,
                    hundredths: 78,
                },
            },
            BACnetTimeStamp::SequenceNumber(33),
        ];
        object.event_history.message_texts = ["offnormal".into(), "fault".into(), "normal".into()];

        assert_eq!(
            object
                .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
                .unwrap(),
            PropertyValue::List(vec![
                PropertyValue::Unsigned(11),
                PropertyValue::Unsigned(0),
                PropertyValue::Unsigned(33),
            ]),
            "{} timestamp projection",
            $label
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS, None)
                .unwrap(),
            PropertyValue::List(vec![
                PropertyValue::CharacterString("offnormal".into()),
                PropertyValue::CharacterString("fault".into()),
                PropertyValue::CharacterString("normal".into()),
            ]),
            "{} messages",
            $label
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(0))
                .unwrap(),
            PropertyValue::Unsigned(3),
            "{} timestamp count",
            $label
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(2))
                .unwrap(),
            PropertyValue::Unsigned(0),
            "{} lossy indexed timestamp",
            $label
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS, Some(2))
                .unwrap(),
            PropertyValue::CharacterString("fault".into()),
            "{} indexed message",
            $label
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS, Some(0))
                .unwrap(),
            PropertyValue::Unsigned(3),
            "{} message count",
            $label
        );
        for property in [
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        ] {
            match object.read_property(property, Some(4)).unwrap_err() {
                Error::Protocol { class, code } => {
                    assert_eq!(
                        class,
                        ErrorClass::PROPERTY.to_raw() as u32,
                        "{} class",
                        $label
                    );
                    assert_eq!(
                        code,
                        ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32,
                        "{} code",
                        $label
                    );
                }
                other => panic!("{} expected INVALID_ARRAY_INDEX, got {other:?}", $label),
            }
        }
    }};
}

#[test]
fn analog_objects_delegate_seeded_event_history_reads() {
    assert_seeded_event_history!(
        AnalogInputObject::new(1, "AI-1", 62).unwrap(),
        "Analog Input"
    );
    assert_seeded_event_history!(
        AnalogOutputObject::new(1, "AO-1", 62).unwrap(),
        "Analog Output"
    );
    assert_seeded_event_history!(
        AnalogValueObject::new(1, "AV-1", 62).unwrap(),
        "Analog Value"
    );
}
