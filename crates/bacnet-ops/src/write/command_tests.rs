//! Commandability by object type and property, the slots of a
//! Priority_Array, and what a write at a priority does to Present_Value.

use bacnet_encoding::primitives::encode_priority_date_time;
use bacnet_types::primitives::{Date, Time};

use super::*;
use crate::format::{Floats, StateTexts};

fn encoded(values: &[PropertyValue]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for value in values {
        encode_property_value(&mut buf, value).unwrap();
    }
    buf.to_vec()
}

fn p(level: u8) -> Priority {
    Priority::new(level).unwrap()
}

/// Clause 19.2.1.1's list: six outputs and the Channel always, the value
/// objects when they have a Priority_Array, inputs and other properties
/// never; a type or property past the standard's may be.
#[test]
fn commandability_follows_the_standard_list() {
    type O = ObjectType;
    type P = PropertyIdentifier;
    let pv = P::PRESENT_VALUE;
    for always in [
        O::ACCESS_DOOR,
        O::ANALOG_OUTPUT,
        O::BINARY_OUTPUT,
        O::MULTI_STATE_OUTPUT,
        O::LIGHTING_OUTPUT,
        O::BINARY_LIGHTING_OUTPUT,
        O::CHANNEL,
    ] {
        assert_eq!(
            Commandability::of(always, pv),
            Commandability::Always,
            "{always}"
        );
    }
    for optional in [
        O::ANALOG_VALUE,
        O::BINARY_VALUE,
        O::MULTI_STATE_VALUE,
        O::INTEGER_VALUE,
        O::LARGE_ANALOG_VALUE,
        O::DATETIME_VALUE,
        O::TIMEPATTERN_VALUE,
        O::from_raw(130),
        O::from_raw(70),
    ] {
        assert_eq!(
            Commandability::of(optional, pv),
            Commandability::Optional,
            "{optional}"
        );
    }
    for never in [
        O::ANALOG_INPUT,
        O::BINARY_INPUT,
        O::DEVICE,
        O::SCHEDULE,
        O::STAGING,
        O::COLOR,
        O::COLOR_TEMPERATURE,
    ] {
        assert_eq!(
            Commandability::of(never, pv),
            Commandability::Never,
            "{never}"
        );
    }
    // Only Present_Value, of a standard object, by a standard property.
    let ao = O::ANALOG_OUTPUT;
    assert_eq!(
        Commandability::of(ao, P::DESCRIPTION),
        Commandability::Never
    );
    assert_eq!(
        Commandability::of(ao, P::RELINQUISH_DEFAULT),
        Commandability::Never
    );
    let vendor = P::from_raw(600);
    assert_eq!(
        Commandability::of(O::ANALOG_INPUT, vendor),
        Commandability::Optional
    );
    let later = P::from_raw(4_194_400);
    assert_eq!(Commandability::of(ao, later), Commandability::Never);
    assert_eq!(
        Commandability::of(O::from_raw(130), P::DESCRIPTION),
        Commandability::Optional
    );
    // A Channel has no Priority_Array of its own; the others that may
    // command do.
    assert!(!keeps_priorities(O::CHANNEL));
    assert!(keeps_priorities(O::ANALOG_VALUE) && keeps_priorities(O::ANALOG_OUTPUT));
    assert!(!keeps_priorities(O::ANALOG_INPUT));
}

/// Sixteen values or none; a DateTime Value's context-tagged date-time
/// slot reads as its Date and Time.
#[test]
fn a_priority_array_is_sixteen_commands() {
    let mut slots = vec![PropertyValue::Null; 16];
    slots[3] = PropertyValue::Real(4.0);
    assert_eq!(commands(&encoded(&slots)), Some(slots.clone()));
    assert_eq!(commands(&encoded(&slots[..15])), None);
    assert_eq!(commands(&[0xFF]), None);
    let date = Date {
        year: 126,
        month: 11,
        day: 14,
        day_of_week: 6,
    };
    let time = Time {
        hour: 7,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    let mut octets = BytesMut::new();
    encode_property_value(&mut octets, &PropertyValue::Null).unwrap();
    encode_priority_date_time(&mut octets, &date, &time);
    octets.extend_from_slice(&encoded(&vec![PropertyValue::Null; 14]));
    let read = commands(&octets).unwrap();
    assert_eq!(
        read[1],
        PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)])
    );
    assert_eq!(elements(&octets).unwrap().len(), 16);
}

/// Present_Value follows the lowest-numbered slot that holds a command.
#[test]
fn a_write_changes_present_value_only_from_the_slot_in_control() {
    let texts = StateTexts::default();
    let format = ValueFormat::new(ObjectType::ANALOG_OUTPUT, &texts, Floats::Exact);
    let mut slots = vec![PropertyValue::Null; 16];
    slots[9] = PropertyValue::Real(55.0);
    let forty_two = encoded(&[PropertyValue::Real(42.0)]);
    let default = Some("0");
    // Weaker than 10: 10 keeps control.
    assert_eq!(
        effect(&format, &slots, p(12), &forty_two, default),
        Effect::Unchanged {
            held_by: Some(p(10))
        }
    );
    // Stronger: it takes control.
    assert_eq!(
        effect(&format, &slots, p(8), &forty_two, default),
        Effect::Changes {
            from: "55".into(),
            to: "42".into()
        }
    );
    // The same value at 10 itself changes nothing, and no stronger slot
    // holds it.
    let fifty_five = encoded(&[PropertyValue::Real(55.0)]);
    assert_eq!(
        effect(&format, &slots, p(10), &fifty_five, default),
        Effect::Unchanged { held_by: None }
    );
    // Emptying 10, the only command, falls back to Relinquish_Default.
    assert_eq!(
        effect(&format, &slots, p(10), &[0x00], default),
        Effect::Changes {
            from: "55".into(),
            to: "0".into()
        }
    );
    // With every slot empty, a write takes control from it.
    let empty = vec![PropertyValue::Null; 16];
    assert_eq!(
        effect(&format, &empty, p(16), &forty_two, None),
        Effect::Changes {
            from: "?".into(),
            to: "42".into()
        }
    );
}
