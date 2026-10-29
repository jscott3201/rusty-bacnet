//! Property-aware input, continued: numbers sent as the property's
//! datatype, the life safety and DateTime Value special cases, range
//! errors, and the bit positions the named bit strings use.

use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, ResolvedBits};
use bacnet_types::primitives::{Date, PropertyValue, Time};
use bytes::BytesMut;

use super::parse_value_for;
use super::typed::bit_string;
use crate::format::{Floats, StateTexts, ValueFormat};
use crate::label::kebab;

type O = ObjectType;
type P = PropertyIdentifier;

fn parse(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    text: &str,
) -> Result<PropertyValue, String> {
    parse_value_for(object_type, property, index, text)
}

#[test]
fn a_number_takes_the_datatype_of_an_analog_or_integer_value() {
    for property in [
        P::PRESENT_VALUE,
        P::RELINQUISH_DEFAULT,
        P::COV_INCREMENT,
        P::HIGH_LIMIT,
        P::LOW_LIMIT,
        P::DEADBAND,
    ] {
        for object_type in [O::ANALOG_INPUT, O::ANALOG_OUTPUT, O::ANALOG_VALUE] {
            assert_eq!(
                parse(object_type, property, None, "72"),
                Ok(PropertyValue::Real(72.0)),
                "{object_type} {property}"
            );
        }
        assert_eq!(
            parse(O::LARGE_ANALOG_VALUE, property, None, "72"),
            Ok(PropertyValue::Double(72.0))
        );
    }
    assert_eq!(
        parse(O::ANALOG_OUTPUT, P::PRIORITY_ARRAY, Some(8), "-3"),
        Ok(PropertyValue::Real(-3.0))
    );
    // A Double holds what a REAL can't.
    assert_eq!(
        parse(O::LARGE_ANALOG_VALUE, P::PRESENT_VALUE, None, "1e39"),
        Ok(PropertyValue::Double(1e39))
    );
    // A Double is read as a Double, so no digit goes through a REAL.
    assert_eq!(
        parse(O::LARGE_ANALOG_VALUE, P::PRESENT_VALUE, None, "72.1"),
        Ok(PropertyValue::Double(72.1))
    );
    // An Integer Value's value and limits are INTEGER; its COV_Increment
    // and Deadband are Unsigned.
    assert_eq!(
        parse(O::INTEGER_VALUE, P::PRESENT_VALUE, None, "72"),
        Ok(PropertyValue::Signed(72))
    );
    assert_eq!(
        parse(O::INTEGER_VALUE, P::DEADBAND, None, "2"),
        Ok(PropertyValue::Unsigned(2))
    );
    assert_eq!(
        parse(O::POSITIVE_INTEGER_VALUE, P::HIGH_LIMIT, None, "9"),
        Ok(PropertyValue::Unsigned(9))
    );
    for (object_type, text, error) in [
        (O::ANALOG_VALUE, "1e39", "out of a REAL's range"),
        (O::LARGE_ANALOG_VALUE, "1e999", "out of a Double's range"),
        (O::INTEGER_VALUE, "72.5", "whole number"),
        (O::INTEGER_VALUE, "3000000000", "out of an INTEGER's range"),
        (O::POSITIVE_INTEGER_VALUE, "-1", "negative"),
        (O::POSITIVE_INTEGER_VALUE, "1.5", "whole number"),
    ] {
        let refused = parse(object_type, P::PRESENT_VALUE, None, text).unwrap_err();
        assert!(refused.contains(error), "{text}: {refused}");
    }
    // A relinquish, an explicit form and an array's size keep their meaning.
    assert_eq!(
        parse(O::ANALOG_VALUE, P::PRIORITY_ARRAY, Some(8), "null"),
        Ok(PropertyValue::Null)
    );
    assert_eq!(
        parse(O::ANALOG_VALUE, P::PRIORITY_ARRAY, Some(0), "16"),
        Ok(PropertyValue::Unsigned(16))
    );
    // Other objects and properties are untouched.
    assert_eq!(
        parse(O::MULTI_STATE_VALUE, P::NUMBER_OF_STATES, None, "4"),
        Ok(PropertyValue::Unsigned(4))
    );
}

#[test]
fn tracking_value_is_a_life_safety_state_only_on_life_safety_objects() {
    assert_eq!(
        parse(O::LIFE_SAFETY_POINT, P::TRACKING_VALUE, None, "alarm"),
        Ok(PropertyValue::Enumerated(2))
    );
    assert_eq!(
        parse(O::LIGHTING_OUTPUT, P::TRACKING_VALUE, None, "42.5"),
        Ok(PropertyValue::Real(42.5))
    );
}

#[test]
fn date_time_values_and_accumulators_take_date_times() {
    let date = Date {
        year: 126,
        month: 10,
        day: 25,
        day_of_week: Date::UNSPECIFIED,
    };
    let time = Time {
        hour: 14,
        minute: 30,
        second: 0,
        hundredths: 0,
    };
    let date_time = PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)]);
    assert_eq!(
        parse(
            O::ACCUMULATOR,
            P::VALUE_CHANGE_TIME,
            None,
            "2026-10-25 14:30"
        ),
        Ok(date_time.clone())
    );
    // A Priority_Array slot is the date-time choice, under context tag 1.
    let mut expected = BytesMut::new();
    expected.extend_from_slice(&[0x1E]);
    encode_property_value(&mut expected, &date_time).unwrap();
    expected.extend_from_slice(&[0x1F]);
    assert_eq!(
        parse(
            O::DATETIME_VALUE,
            P::PRIORITY_ARRAY,
            Some(5),
            "2026-10-25 14:30"
        ),
        Ok(PropertyValue::ApplicationData(expected.to_vec()))
    );
}

#[test]
fn range_and_state_errors_say_what_is_needed() {
    let refused = parse(O::ANALOG_INPUT, P::EVENT_STATE, None, "4294967296").unwrap_err();
    assert!(
        refused.contains("out of an enumeration's range"),
        "{refused}"
    );
    let refused = parse(O::MULTI_STATE_VALUE, P::PRESENT_VALUE, None, "Heat").unwrap_err();
    assert!(refused.contains("not a state number"), "{refused}");
    assert_eq!(
        parse(O::MULTI_STATE_VALUE, P::PRESENT_VALUE, None, "2 (Heat)"),
        Ok(PropertyValue::Unsigned(2))
    );
}

#[test]
fn each_named_bit_is_the_bit_the_formatter_names() {
    let texts = StateTexts::default();
    let format = ValueFormat::new(O::DEVICE, &texts, Floats::Exact);
    for property in [
        P::STATUS_FLAGS,
        P::EVENT_ENABLE,
        P::LIMIT_ENABLE,
        P::PROTOCOL_SERVICES_SUPPORTED,
        P::PROTOCOL_OBJECT_TYPES_SUPPORTED,
    ] {
        let width = match ResolvedBits::from_property(property, 0, &[]) {
            ResolvedBits::StatusFlags(_) => 4,
            ResolvedBits::EventTransitionBits(_) => 3,
            ResolvedBits::LimitEnable(_) => 2,
            _ => 64,
        };
        for bit in 0..width {
            let mut set = vec![false; width];
            set[bit] = true;
            let value = bit_string(&set);
            let mut buf = BytesMut::new();
            encode_property_value(&mut buf, &value).unwrap();
            let name = format.value(property, &buf);
            // No name at this position: a bit string without names, or a
            // service bit the enumeration doesn't name, printed as its number.
            if name.starts_with("bits(") || name.parse::<u32>().is_ok() {
                continue;
            }
            // Parsing the one name sets that one bit, at the same position.
            let parsed = parse(O::DEVICE, property, None, &name).unwrap();
            let PropertyValue::BitString { data, .. } = parsed else {
                panic!("{property} {name}: not a bit string");
            };
            let on: Vec<usize> = (0..data.len() * 8)
                .filter(|i| data[i / 8] & (0x80 >> (i % 8)) != 0)
                .collect();
            assert_eq!(on, [bit], "{property} {name} = {}", kebab(&name));
        }
    }
}

/// A state text may hold parentheses and commas: the first ` (` starts
/// it, and a list splits only at commas outside one, so what `read`
/// prints writes back (#2132).
#[test]
fn state_texts_with_parentheses_and_commas_write_back() {
    let texts = StateTexts {
        active: Some("On, Auto".into()),
        inactive: Some("Off (manual)".into()),
        states: vec!["Off".into(), "Low (eco)".into()],
    };
    let format = ValueFormat::new(O::MULTI_STATE_VALUE, &texts, Floats::Exact);
    let mut state = BytesMut::new();
    encode_property_value(&mut state, &PropertyValue::Unsigned(2)).unwrap();
    let printed = format.value(P::PRESENT_VALUE, &state);
    assert_eq!(printed, "2 (Low (eco))");
    assert_eq!(
        parse(O::MULTI_STATE_VALUE, P::PRESENT_VALUE, None, &printed),
        Ok(PropertyValue::Unsigned(2))
    );
    assert_eq!(
        parse(
            O::BINARY_VALUE,
            P::PRESENT_VALUE,
            None,
            "inactive (Off (manual))"
        ),
        Ok(PropertyValue::Enumerated(0))
    );
    assert_eq!(
        parse(
            O::BINARY_INPUT,
            P::ALARM_VALUES,
            None,
            "[active (On, Auto), inactive (Off (manual))]"
        ),
        Ok(PropertyValue::List(vec![
            PropertyValue::Enumerated(1),
            PropertyValue::Enumerated(0)
        ]))
    );
}
