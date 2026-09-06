//! Regression coverage for shared analog event-history reads (#235).

use super::super::*;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::primitives::{Date, Time};

fn assert_timestamp_value(actual: PropertyValue, expected: &BACnetTimeStamp, context: &str) {
    let PropertyValue::ApplicationData(bytes) = actual else {
        panic!("{context}: expected timestamp ApplicationData");
    };
    let (decoded, end) = decode_timestamp_choice(&bytes, 0).unwrap();
    assert_eq!(&decoded, expected, "{context}: timestamp choice");
    assert_eq!(end, bytes.len(), "{context}: trailing timestamp bytes");
}

fn assert_timestamp_array(actual: PropertyValue, expected: &[BACnetTimeStamp; 3], context: &str) {
    let PropertyValue::List(elements) = actual else {
        panic!("{context}: expected timestamp list");
    };
    assert_eq!(elements.len(), expected.len(), "{context}: array length");
    for (slot, (actual, expected)) in elements.into_iter().zip(expected).enumerate() {
        assert_timestamp_value(actual, expected, &format!("{context} slot {}", slot + 1));
    }
}

macro_rules! assert_seeded_event_history {
    ($object:expr, $label:literal) => {{
        let mut object = $object;
        let expected_timestamps = [
            BACnetTimeStamp::Time(Time {
                hour: 1,
                minute: 2,
                second: 3,
                hundredths: 4,
            }),
            BACnetTimeStamp::SequenceNumber(22),
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
        ];
        object.event_history.time_stamps = expected_timestamps.clone();
        object.event_history.message_texts = ["offnormal".into(), "fault".into(), "normal".into()];

        assert_timestamp_array(
            object
                .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
                .unwrap(),
            &expected_timestamps,
            concat!($label, " timestamps"),
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
        for property in [
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        ] {
            assert_eq!(
                object.read_property(property, Some(0)).unwrap(),
                PropertyValue::Unsigned(3),
                "{} {property:?} count",
                $label
            );
        }
        for (slot, expected) in expected_timestamps.iter().enumerate() {
            assert_timestamp_value(
                object
                    .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(slot as u32 + 1))
                    .unwrap(),
                expected,
                &format!("{} timestamp slot {}", $label, slot + 1),
            );
        }
        for (slot, expected) in ["offnormal", "fault", "normal"].into_iter().enumerate() {
            assert_eq!(
                object
                    .read_property(
                        PropertyIdentifier::EVENT_MESSAGE_TEXTS,
                        Some(slot as u32 + 1),
                    )
                    .unwrap(),
                PropertyValue::CharacterString(expected.into()),
                "{} message slot {}",
                $label,
                slot + 1
            );
        }
        for property in [
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        ] {
            for index in [4, u32::MAX] {
                match object.read_property(property, Some(index)).unwrap_err() {
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
