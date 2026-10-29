//! Property-aware input: what the formatter prints parses back to the same
//! value, and a mistyped name is refused with the choices.

use bacnet_encoding::primitives::{encode_property_value, encode_timestamp_choice};
use bacnet_types::enums::{ObjectType, PropertyIdentifier, ResolvedEnum};
use bacnet_types::primitives::{BACnetTimeStamp, Date, PropertyValue, Time};
use bytes::BytesMut;

use super::typed::bit_string;
use super::{parse_value_for, parse_value_for_with_priority};
use crate::format::{Floats, StateTexts, ValueFormat};

type O = ObjectType;
type P = PropertyIdentifier;

const ANY: u8 = 0xFF;

fn encode(value: &PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, value).unwrap();
    buf.to_vec()
}

/// The formatter's text for `value`, as the CLI prints it.
fn printed(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    texts: &StateTexts,
    value: &PropertyValue,
) -> String {
    ValueFormat::new(object_type, texts, Floats::Exact)
        .whole_lists()
        .value_at(property, index, &encode(value))
}

/// Format `value`, parse the text back for the same property, and expect
/// the same value.
#[track_caller]
fn round_trips(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: PropertyValue,
) {
    round_trips_with(object_type, property, index, &StateTexts::default(), value);
}

#[track_caller]
fn round_trips_with(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    texts: &StateTexts,
    value: PropertyValue,
) {
    let text = printed(object_type, property, index, texts, &value);
    assert_eq!(
        parse_value_for(object_type, property, index, &text),
        Ok(value),
        "{object_type} {property} printed as {text:?}"
    );
}

#[test]
fn every_named_value_of_every_enumerated_property_round_trips() {
    let mut checked = 0;
    for &(_, property) in P::ALL_NAMED {
        let Some((_, names)) = ResolvedEnum::from_property(property, 0).enumeration() else {
            continue;
        };
        // Tracking_Value is enumerated only on the life safety objects.
        let object_type = if property == P::TRACKING_VALUE {
            O::LIFE_SAFETY_POINT
        } else {
            O::ANALOG_VALUE
        };
        for (_, value) in names {
            round_trips(
                object_type,
                property,
                None,
                PropertyValue::Enumerated(value),
            );
            checked += 1;
        }
        // A number the enumeration doesn't name prints, and parses, as itself.
        round_trips(object_type, property, None, PropertyValue::Enumerated(250));
    }
    assert!(checked > 500, "only {checked} names checked");
}

#[test]
fn names_take_any_spelling_and_numbers() {
    let parse = |text| parse_value_for(O::ANALOG_INPUT, P::EVENT_STATE, None, text);
    for text in ["offnormal", "OFFNORMAL", "Offnormal", "2", "off_normal"] {
        assert_eq!(parse(text), Ok(PropertyValue::Enumerated(2)), "{text}");
    }
    assert_eq!(
        parse_value_for(O::ANALOG_INPUT, P::UNITS, None, "degrees-fahrenheit"),
        Ok(PropertyValue::Enumerated(64))
    );
    // A list of names, as the formatter prints a list.
    assert_eq!(
        parse_value_for(
            O::LIFE_SAFETY_POINT,
            P::ALARM_VALUES,
            None,
            "[alarm, fault]"
        ),
        Ok(PropertyValue::List(vec![
            PropertyValue::Enumerated(2),
            PropertyValue::Enumerated(3),
        ]))
    );
}

#[test]
fn binary_states_take_their_names_and_the_objects_text() {
    for object_type in [O::BINARY_INPUT, O::BINARY_OUTPUT, O::BINARY_VALUE] {
        for property in [P::PRESENT_VALUE, P::RELINQUISH_DEFAULT, P::ALARM_VALUE] {
            for raw in [0, 1] {
                round_trips(object_type, property, None, PropertyValue::Enumerated(raw));
            }
        }
    }
    let texts = StateTexts {
        active: Some("On".into()),
        inactive: Some("Off".into()),
        states: Vec::new(),
    };
    for raw in [0, 1] {
        round_trips_with(
            O::BINARY_VALUE,
            P::PRESENT_VALUE,
            None,
            &texts,
            PropertyValue::Enumerated(raw),
        );
    }
    // A Priority_Array element is a state too.
    assert_eq!(
        parse_value_for(O::BINARY_OUTPUT, P::PRIORITY_ARRAY, Some(8), "ACTIVE"),
        Ok(PropertyValue::Enumerated(1))
    );
    // A multi-state state is its number, with its text or without.
    let texts = StateTexts {
        states: vec!["Off".into(), "Heat".into()],
        ..StateTexts::default()
    };
    round_trips_with(
        O::MULTI_STATE_VALUE,
        P::PRESENT_VALUE,
        None,
        &texts,
        PropertyValue::Unsigned(2),
    );
    // Index 0 of an array is its size, never a state.
    assert_eq!(
        parse_value_for(O::BINARY_OUTPUT, P::PRIORITY_ARRAY, Some(0), "16"),
        Ok(PropertyValue::Unsigned(16))
    );
}

#[test]
fn named_bit_strings_round_trip() {
    for flags in 0..16u8 {
        let set: Vec<bool> = (0..4).map(|bit| flags & (1 << bit) != 0).collect();
        round_trips(O::ANALOG_INPUT, P::STATUS_FLAGS, None, bit_string(&set));
    }
    for property in [P::EVENT_ENABLE, P::ACKED_TRANSITIONS, P::ACK_REQUIRED] {
        for flags in 0..8u8 {
            let set: Vec<bool> = (0..3).map(|bit| flags & (1 << bit) != 0).collect();
            round_trips(O::ANALOG_INPUT, property, None, bit_string(&set));
        }
    }
    for flags in 0..4u8 {
        let set: Vec<bool> = (0..2).map(|bit| flags & (1 << bit) != 0).collect();
        round_trips(O::ANALOG_INPUT, P::LIMIT_ENABLE, None, bit_string(&set));
    }
    // A long list: some services, and some object types.
    let services = parse_value_for(
        O::DEVICE,
        P::PROTOCOL_SERVICES_SUPPORTED,
        None,
        "read-property,write-property,who-is",
    )
    .unwrap();
    round_trips(O::DEVICE, P::PROTOCOL_SERVICES_SUPPORTED, None, services);
    let types = parse_value_for(
        O::DEVICE,
        P::PROTOCOL_OBJECT_TYPES_SUPPORTED,
        None,
        "analog-input | device",
    )
    .unwrap();
    round_trips(O::DEVICE, P::PROTOCOL_OBJECT_TYPES_SUPPORTED, None, types);
    // A bit string without names round-trips as its bits.
    let plain = bit_string(&[false, true, false, true, true]);
    round_trips(O::BITSTRING_VALUE, P::PRESENT_VALUE, None, plain);
}

#[test]
fn bit_strings_take_names_digits_or_none() {
    let parse = |text| parse_value_for(O::ANALOG_INPUT, P::STATUS_FLAGS, None, text);
    let in_alarm_fault = bit_string(&[true, true, false, false]);
    for text in [
        "in-alarm,fault",
        "in-alarm | fault",
        "FAULT|IN_ALARM",
        "1100",
    ] {
        assert_eq!(parse(text), Ok(in_alarm_fault.clone()), "{text}");
    }
    for text in ["none", "NONE", "0000"] {
        assert_eq!(parse(text), Ok(bit_string(&[false; 4])), "{text:?}");
    }
    // An empty value is refused, so a blank field never clears every bit.
    for text in ["", "  ", "()"] {
        assert!(parse(text).is_err(), "{text:?}");
    }
    assert!(parse("").unwrap_err().contains("or none"));
    // Digits give exactly the bits typed.
    assert_eq!(parse("10"), Ok(bit_string(&[true, false])));
}

#[test]
fn dates_times_and_date_times_round_trip() {
    let dates = [
        Date {
            year: 126,
            month: 10,
            day: 19,
            day_of_week: ANY,
        },
        Date {
            year: 126,
            month: 10,
            day: 19,
            day_of_week: 1,
        },
        Date {
            year: ANY,
            month: 14,
            day: 32,
            day_of_week: ANY,
        },
        Date {
            year: ANY,
            month: 13,
            day: 33,
            day_of_week: 3,
        },
        Date {
            year: ANY,
            month: ANY,
            day: ANY,
            day_of_week: 5,
        },
        Date {
            year: ANY,
            month: ANY,
            day: ANY,
            day_of_week: ANY,
        },
    ];
    let times = [
        Time {
            hour: 7,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        Time {
            hour: 14,
            minute: 30,
            second: 5,
            hundredths: 25,
        },
        Time {
            hour: 7,
            minute: 0,
            second: 0,
            hundredths: 50,
        },
        Time {
            hour: ANY,
            minute: 30,
            second: ANY,
            hundredths: 0,
        },
        Time {
            hour: ANY,
            minute: ANY,
            second: ANY,
            hundredths: ANY,
        },
    ];
    for date in dates {
        round_trips(O::DEVICE, P::LOCAL_DATE, None, PropertyValue::Date(date));
        for time in times {
            round_trips(
                O::DATETIME_VALUE,
                P::PRESENT_VALUE,
                None,
                PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)]),
            );
        }
    }
    for time in times {
        round_trips(O::DEVICE, P::LOCAL_TIME, None, PropertyValue::Time(time));
    }
    // A DateTime property takes a date alone, its time unspecified.
    assert_eq!(
        parse_value_for(O::TREND_LOG, P::START_TIME, None, "2026-10-19"),
        Ok(PropertyValue::List(vec![
            PropertyValue::Date(dates[0]),
            PropertyValue::Time(times[4]),
        ]))
    );
}

#[test]
fn timestamps_round_trip() {
    let stamps = [
        BACnetTimeStamp::SequenceNumber(42),
        BACnetTimeStamp::Time(Time {
            hour: 14,
            minute: 30,
            second: 5,
            hundredths: 0,
        }),
        BACnetTimeStamp::DateTime {
            date: Date {
                year: 126,
                month: 10,
                day: 25,
                day_of_week: 7,
            },
            time: Time {
                hour: 14,
                minute: 30,
                second: 5,
                hundredths: 0,
            },
        },
    ];
    for stamp in stamps {
        let mut buf = BytesMut::new();
        encode_timestamp_choice(&mut buf, &stamp).unwrap();
        let value = PropertyValue::ApplicationData(buf.to_vec());
        let texts = StateTexts::default();
        let format = ValueFormat::new(O::DEVICE, &texts, Floats::Exact);
        let text = format.value(P::LAST_RESTORE_TIME, &buf);
        assert_eq!(
            parse_value_for(O::DEVICE, P::LAST_RESTORE_TIME, None, &text),
            Ok(value.clone()),
            "{text}"
        );
        let text = format.value_at(P::EVENT_TIME_STAMPS, Some(2), &buf);
        assert_eq!(
            parse_value_for(O::ANALOG_INPUT, P::EVENT_TIME_STAMPS, Some(2), &text),
            Ok(value),
            "{text}"
        );
    }
}

#[test]
fn explicit_forms_and_priorities_still_work() {
    let parse = |text| parse_value_for_with_priority(O::BINARY_VALUE, P::PRESENT_VALUE, None, text);
    assert_eq!(
        parse("enumerated:3"),
        Ok((PropertyValue::Enumerated(3), None))
    );
    assert_eq!(parse("null@8"), Ok((PropertyValue::Null, Some(8))));
    assert_eq!(
        parse("active@8"),
        Ok((PropertyValue::Enumerated(1), Some(8)))
    );
    assert!(parse("active@17").unwrap_err().contains("priority"));
    // An untyped property reads the untyped grammar.
    assert_eq!(
        parse_value_for_with_priority(O::ANALOG_VALUE, P::PRESENT_VALUE, None, "72.5@8"),
        Ok((PropertyValue::Real(72.5), Some(8)))
    );
}

#[test]
fn a_mistyped_name_lists_the_choices() {
    let refused = parse_value_for(O::BINARY_VALUE, P::PRESENT_VALUE, None, "actve").unwrap_err();
    assert_eq!(
        refused,
        "unknown BinaryPV 'actve'; expected one of: inactive, active (or a number)"
    );
    let refused =
        parse_value_for(O::ANALOG_INPUT, P::STATUS_FLAGS, None, "in-alarm,fualt").unwrap_err();
    assert_eq!(
        refused,
        "unknown StatusFlags bit 'fualt'; expected one of: in-alarm, fault, overridden, \
         out-of-service (or the bits as 0/1 digits, or none)"
    );
    // A long enumeration offers the nearest few.
    let refused =
        parse_value_for(O::ANALOG_INPUT, P::UNITS, None, "degrees-farenheit").unwrap_err();
    assert!(
        refused.starts_with("unknown EngineeringUnits 'degrees-farenheit'; nearest: "),
        "{refused}"
    );
    assert!(refused.contains("degrees-fahrenheit"), "{refused}");
    assert_eq!(refused.matches(", ").count(), 4, "{refused}");
    let refused = parse_value_for(O::ANALOG_INPUT, P::UNITS, None, "fahrenheit").unwrap_err();
    assert!(refused.contains("nearest: degrees-fahrenheit"), "{refused}");
}
