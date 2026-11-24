//! A record from an object's rows: which errors are problems, how a
//! value of the wrong type reads, and what makes an object commandable.

use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P,
};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::PropertyValue;

use super::record::record;
use super::state_tests::REQUIRES_EVENT_STATE;
use super::test_lab::{oid, refused, row, text};
use super::*;
use crate::device::DeviceAddress;
use crate::format::RawRow;

fn of(object_type: ObjectType, rows: Vec<RawRow>) -> PointRecord {
    let at = DeviceAddress::local(&[2]).unwrap();
    record(&at, 7, oid(object_type, 1), rows)
}

fn denied(property: P) -> RawRow {
    let error = RemoteError::Error {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::READ_ACCESS_DENIED,
    };
    (property, Err(PropertyError::Remote(error)))
}

/// A property every object of the type has is a problem whatever the
/// device's error; an optional one the object lacks is absent with no
/// problem, and one the device refuses for another reason is a problem.
#[test]
fn only_a_missing_optional_property_is_no_problem() {
    let rows = vec![
        refused(P::OBJECT_NAME),
        refused(P::DESCRIPTION),
        refused(P::UNITS),
    ];
    let read = of(ObjectType::ANALOG_INPUT, rows);
    let problems: Vec<P> = read.problems.iter().map(|p| p.property).collect();
    assert_eq!(problems, [P::OBJECT_NAME, P::UNITS]);
    assert!(read.problem(P::DESCRIPTION).is_none());
    assert_eq!(
        read.problem(P::UNITS).unwrap().to_string(),
        "PROPERTY:UNKNOWN_PROPERTY"
    );
    assert!(read.name.is_none() && read.description.is_none() && read.units.is_none());

    let rows = vec![row(P::OBJECT_NAME, text("Zone")), denied(P::DESCRIPTION)];
    let read = of(ObjectType::TREND_LOG, rows);
    assert_eq!(read.name.as_deref(), Some("Zone"));
    assert_eq!(read.problems.len(), 1, "{read:?}");
    assert_eq!(read.problems[0].property, P::DESCRIPTION);
    // Units are optional where the type doesn't always have them.
    let rows = vec![row(P::OBJECT_NAME, text("Zone")), refused(P::UNITS)];
    assert!(of(ObjectType::TREND_LOG, rows).problems.is_empty());
    // A multi-state object always has its Number_Of_States.
    let rows = vec![refused(P::NUMBER_OF_STATES), refused(P::STATE_TEXT)];
    let read = of(ObjectType::MULTI_STATE_INPUT, rows);
    assert_eq!(read.problems.len(), 1, "{read:?}");
    assert_eq!(read.problems[0].property, P::NUMBER_OF_STATES);
}

/// A value that isn't of the property's type is a problem that says so,
/// and the field stays empty; the first row of a property counts.
#[test]
fn a_value_of_the_wrong_type_is_a_problem() {
    let rows = vec![
        row(P::OBJECT_NAME, PropertyValue::Unsigned(5)),
        row(P::DESCRIPTION, PropertyValue::Real(1.0)),
        row(P::UNITS, PropertyValue::Real(1.0)),
    ];
    let read = of(ObjectType::ANALOG_INPUT, rows);
    let said = |property| read.problem(property).map(ToString::to_string);
    assert_eq!(
        said(P::OBJECT_NAME).as_deref(),
        Some("not a character string")
    );
    assert_eq!(
        said(P::DESCRIPTION).as_deref(),
        Some("not a character string")
    );
    assert_eq!(said(P::UNITS).as_deref(), Some("not an engineering unit"));
    assert!(read.name.is_none() && read.units.is_none());

    let rows = vec![row(P::NUMBER_OF_STATES, text("three"))];
    let read = of(ObjectType::MULTI_STATE_VALUE, rows);
    assert_eq!(
        read.problem(P::NUMBER_OF_STATES).unwrap().to_string(),
        "not an Unsigned"
    );

    let rows = vec![
        row(P::OBJECT_NAME, text("First")),
        row(P::OBJECT_NAME, text("Second")),
        row(P::UNITS, PropertyValue::Enumerated(62)),
    ];
    let read = of(ObjectType::ANALOG_INPUT, rows);
    assert_eq!(read.name.as_deref(), Some("First"));
    assert_eq!(read.units, Some(EngineeringUnits::DEGREES_CELSIUS));
    assert!(read.problems.is_empty());
}

/// An output is commandable with nothing read; an input never; a value,
/// and an object of a type this release doesn't know, when it has a
/// Priority_Array. A Priority_Array the device refuses for another reason
/// is a problem, and the object reads as not commandable.
#[test]
fn commandable_by_type_or_by_priority_array() {
    let name = || row(P::OBJECT_NAME, text("Zone"));
    let array = || row(P::PRIORITY_ARRAY, PropertyValue::Null);
    assert!(of(ObjectType::BINARY_OUTPUT, vec![name()]).commandable);
    assert!(!of(ObjectType::BINARY_INPUT, vec![name(), array()]).commandable);
    assert!(of(ObjectType::BINARY_VALUE, vec![name(), array()]).commandable);
    let lacking = of(
        ObjectType::BINARY_VALUE,
        vec![name(), refused(P::PRIORITY_ARRAY)],
    );
    assert!(!lacking.commandable && lacking.problems.is_empty());
    let vendor = ObjectType::from_raw(700);
    assert!(of(vendor, vec![name(), array()]).commandable);
    assert!(!of(vendor, vec![name()]).commandable);
    let hidden = of(
        ObjectType::ANALOG_VALUE,
        vec![name(), denied(P::PRIORITY_ARRAY)],
    );
    assert!(!hidden.commandable);
    assert_eq!(hidden.problems.len(), 1);
}

/// The texts are the device's exactly; the values' own text names units
/// and states.
#[test]
fn texts_are_exact_and_values_are_named() {
    let rows = vec![
        row(P::OBJECT_NAME, text("Fan\u{7}")),
        row(P::ACTIVE_TEXT, text("On\n")),
        row(P::INACTIVE_TEXT, text("Off")),
        row(P::PRESENT_VALUE, PropertyValue::Enumerated(1)),
    ];
    let read = of(ObjectType::BINARY_VALUE, rows);
    assert_eq!(read.name.as_deref(), Some("Fan\u{7}"));
    assert_eq!(read.state_texts.active.as_deref(), Some("On\n"));
    let value = read.present_value.unwrap();
    assert_eq!(value.as_bool(), Some(true));
    assert_eq!(value.text, "active (On )", "cleaned for display");
}

/// Polarity is a problem when a Binary Input or Output lacks it, whatever
/// the error, or gives a value that isn't an Enumerated, and the field
/// stays empty.
#[test]
fn a_missing_polarity_is_a_problem_where_the_type_has_one() {
    for object_type in [ObjectType::BINARY_INPUT, ObjectType::BINARY_OUTPUT] {
        let read = of(object_type, vec![refused(P::POLARITY)]);
        assert_eq!(read.problems.len(), 1, "{read:?}");
        assert_eq!(read.problems[0].property, P::POLARITY);
        assert!(read.polarity.is_none());
        let read = of(object_type, vec![denied(P::POLARITY)]);
        assert_eq!(read.problems[0].property, P::POLARITY);
        let read = of(
            object_type,
            vec![row(P::POLARITY, PropertyValue::Enumerated(1))],
        );
        assert!(read.problems.is_empty(), "{read:?}");
        let polarity = read.polarity.unwrap();
        assert_eq!(polarity.text, "reverse");
        let read = of(
            object_type,
            vec![row(P::POLARITY, PropertyValue::Real(1.0))],
        );
        assert!(read.polarity.is_none(), "{read:?}");
        let said = read.problem(P::POLARITY).map(ToString::to_string);
        assert_eq!(said.as_deref(), Some("not an Enumerated (got a Real, 1)"));
    }
    // A Binary Value has no Polarity: one it calls unknown is no problem.
    let read = of(ObjectType::BINARY_VALUE, vec![refused(P::POLARITY)]);
    assert!(read.problems.is_empty() && read.polarity.is_none());
}

/// Relinquish_Default is a problem when an output lacks it, and absent
/// without one when a value does; Present_Value, Status_Flags and
/// Event_State are problems when a type whose table requires them lacks
/// them, and absent without one on a type that needn't have them.
#[test]
fn required_settings_and_values_are_problems_when_lacking() {
    let lacking = || {
        vec![
            row(P::OBJECT_NAME, text("Zone")),
            refused(P::RELINQUISH_DEFAULT),
            refused(P::PRESENT_VALUE),
            refused(P::STATUS_FLAGS),
            refused(P::EVENT_STATE),
        ]
    };
    // `expected`, and Event_State after it where the type requires one.
    let and_event_state = |expected: &[P], object_type| -> Vec<P> {
        let mut expected = expected.to_vec();
        if REQUIRES_EVENT_STATE.contains(&object_type) {
            expected.push(P::EVENT_STATE);
        }
        expected
    };
    let problems = |object_type| -> Vec<P> {
        let read = of(object_type, lacking());
        read.problems
            .iter()
            .map(|problem| problem.property)
            .collect()
    };
    let all = [P::RELINQUISH_DEFAULT, P::PRESENT_VALUE, P::STATUS_FLAGS];
    for output in [
        ObjectType::ANALOG_OUTPUT,
        ObjectType::BINARY_OUTPUT,
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::ACCESS_DOOR,
        ObjectType::LIGHTING_OUTPUT,
        ObjectType::BINARY_LIGHTING_OUTPUT,
    ] {
        assert_eq!(problems(output), and_event_state(&all, output), "{output}");
    }
    // A value's Relinquish_Default comes with its Priority_Array; an
    // input has none.
    let values = [P::PRESENT_VALUE, P::STATUS_FLAGS];
    for other in [
        ObjectType::ANALOG_VALUE,
        ObjectType::BINARY_INPUT,
        ObjectType::MULTI_STATE_VALUE,
        ObjectType::CHARACTERSTRING_VALUE,
        ObjectType::DATEPATTERN_VALUE,
        ObjectType::ACCUMULATOR,
        ObjectType::LOOP,
        ObjectType::PULSE_CONVERTER,
        ObjectType::LIFE_SAFETY_POINT,
        ObjectType::LIFE_SAFETY_ZONE,
        ObjectType::CHANNEL,
        ObjectType::SCHEDULE,
        ObjectType::LOAD_CONTROL,
        ObjectType::TIMER,
        ObjectType::STAGING,
        ObjectType::GLOBAL_GROUP,
        ObjectType::CREDENTIAL_DATA_INPUT,
    ] {
        assert_eq!(problems(other), and_event_state(&values, other), "{other}");
    }
    // These have a Present_Value and no Status_Flags.
    for value_only in [
        ObjectType::GROUP,
        ObjectType::COLOR,
        ObjectType::COLOR_TEMPERATURE,
    ] {
        let expected = and_event_state(&[P::PRESENT_VALUE], value_only);
        assert_eq!(problems(value_only), expected, "{value_only}");
    }

    // A trend log needs an Event_State and none of the others; a Device
    // and a vendor's type needn't have any of them.
    assert_eq!(problems(ObjectType::TREND_LOG), [P::EVENT_STATE]);
    for none in [ObjectType::DEVICE, ObjectType::from_raw(700)] {
        assert!(problems(none).is_empty(), "{none}");
    }
}

/// Status_Flags read with values are the four flags by name, a bit
/// string too short for all four reading the rest as clear (#2620). One
/// that isn't a bit string is a problem saying what came, and the field
/// and the flags are empty.
#[test]
fn status_flags_are_typed_by_name() {
    use bacnet_types::bitstring::StatusFlags;
    let bits = |unused_bits, octet| PropertyValue::BitString {
        unused_bits,
        data: vec![octet],
    };
    let flags_of = |value| {
        let rows = vec![
            row(P::OBJECT_NAME, text("Zone")),
            row(P::STATUS_FLAGS, value),
        ];
        of(ObjectType::ANALOG_INPUT, rows)
    };

    let read = flags_of(bits(4, 0b1001_0000));
    assert_eq!(
        read.flags(),
        Some(StatusFlags::IN_ALARM | StatusFlags::OUT_OF_SERVICE)
    );
    assert_eq!(
        read.status_flags.as_ref().unwrap().text,
        "in-alarm | out-of-service"
    );
    assert!(read.problems.is_empty(), "{read:?}");
    // Two bits only: fault set, the other two absent and clear.
    let read = flags_of(bits(6, 0b0100_0000));
    assert_eq!(read.flags(), Some(StatusFlags::FAULT));
    assert!(read.problems.is_empty(), "{read:?}");

    let read = flags_of(PropertyValue::Unsigned(5));
    assert_eq!(read.flags(), None);
    assert!(read.status_flags.is_none(), "left out as its siblings are");
    assert_eq!(
        read.problem(P::STATUS_FLAGS).unwrap().to_string(),
        "not a bit string (got an Unsigned, 5)"
    );

    // Not read: no flags, and no problem.
    let read = of(
        ObjectType::ANALOG_INPUT,
        vec![row(P::OBJECT_NAME, text("Zone"))],
    );
    assert_eq!(read.flags(), None);
    assert!(read.problems.is_empty(), "{read:?}");
}
