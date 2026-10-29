//! State_Text written whole sets Number_Of_States (#1443): Clause 12.20.11
//! and its Multi-state Input and Output counterparts tie the two sizes
//! together both ways. A new count is checked as one given at creation is,
//! and a refusal changes nothing.

use super::super::*;
use crate::command_source::test_origin;
use PropertyIdentifier as P;
use bacnet_types::error::RemoteError;

fn code(result: Result<(), Error>) -> ErrorCode {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            code
        }
        other => panic!("expected a PROPERTY refusal, got {other:?}"),
    }
}

fn labels(count: usize) -> PropertyValue {
    PropertyValue::List(
        (1..=count)
            .map(|state| PropertyValue::CharacterString(format!("Label {state}")))
            .collect(),
    )
}

fn count(object: &dyn BACnetObject) -> PropertyValue {
    object.read_property(P::NUMBER_OF_STATES, None).unwrap()
}

fn snapshot(object: &dyn BACnetObject) -> (PropertyValue, PropertyValue) {
    (
        count(object),
        object.read_property(P::STATE_TEXT, None).unwrap(),
    )
}

fn all_three(number_of_states: u32) -> [Box<dyn BACnetObject>; 3] {
    [
        Box::new(MultiStateInputObject::new(1, "MSI-1", number_of_states).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "MSO-1", number_of_states).unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV-1", number_of_states).unwrap()),
    ]
}

#[test]
fn a_whole_state_text_sets_the_count_both_ways() {
    for mut object in all_three(3) {
        let kind = object.object_identifier().object_type();
        // The same size changes the labels only.
        object
            .write_property(P::STATE_TEXT, None, labels(3), None)
            .unwrap();
        assert_eq!(snapshot(&*object), (PropertyValue::Unsigned(3), labels(3)));
        // Growth and shrink both carry Number_Of_States along.
        for states in [5, 2, 4] {
            object
                .write_property(P::STATE_TEXT, None, labels(states), None)
                .unwrap();
            assert_eq!(
                snapshot(&*object),
                (PropertyValue::Unsigned(states as u64), labels(states)),
                "{kind:?} {states}"
            );
        }
        // A single label arrives alone, as a one-element array decodes.
        object
            .write_property(
                P::STATE_TEXT,
                None,
                PropertyValue::CharacterString("Only".into()),
                None,
            )
            .unwrap();
        assert_eq!(count(&*object), PropertyValue::Unsigned(1), "{kind:?}");
        // The cap is a valid count.
        let cap = MAX_NUMBER_OF_STATES as usize;
        object
            .write_property(P::STATE_TEXT, None, labels(cap), None)
            .unwrap();
        assert_eq!(count(&*object), PropertyValue::Unsigned(cap as u64));
    }
}

#[test]
fn a_bad_whole_state_text_is_refused_and_changes_nothing() {
    for mut object in all_three(3) {
        let kind = object.object_identifier().object_type();
        let before = snapshot(&*object);
        for (value, expected) in [
            (labels(0), ErrorCode::VALUE_OUT_OF_RANGE),
            (
                labels(MAX_NUMBER_OF_STATES as usize + 1),
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            (
                PropertyValue::List(vec![
                    PropertyValue::CharacterString("a".into()),
                    PropertyValue::Unsigned(2),
                ]),
                ErrorCode::INVALID_DATA_TYPE,
            ),
            (PropertyValue::Unsigned(2), ErrorCode::INVALID_DATA_TYPE),
            (PropertyValue::Null, ErrorCode::INVALID_DATA_TYPE),
        ] {
            assert_eq!(
                code(object.write_property(P::STATE_TEXT, None, value.clone(), None)),
                expected,
                "{kind:?} {value:?}"
            );
            assert_eq!(snapshot(&*object), before, "{kind:?}");
        }
    }
}

/// Each object holding state 3 one way, at 4 states.
fn holders() -> Vec<(&'static str, Box<dyn BACnetObject>)> {
    let three = PropertyValue::Unsigned(3);
    let commanded = |mut object: Box<dyn BACnetObject>| {
        object
            .write_property_from(
                P::PRESENT_VALUE,
                None,
                three.clone(),
                Some(8),
                &test_origin(),
            )
            .unwrap();
        object
    };
    let mut simulated = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
    simulated.set_present_value(3);
    let mut input_alarms = MultiStateInputObject::new(2, "MSI-2", 4).unwrap();
    input_alarms.set_alarm_values(vec![1, 3]);
    let mut value_alarms = MultiStateValueObject::new(2, "MSV-2", 4).unwrap();
    value_alarms.set_alarm_values(vec![3]);
    let mut output_default = MultiStateOutputObject::new(2, "MSO-2", 4).unwrap();
    output_default.set_relinquish_default(3).unwrap();
    let mut value_default = MultiStateValueObject::new(3, "MSV-3", 4).unwrap();
    value_default.set_relinquish_default(3).unwrap();
    vec![
        ("input Present_Value", Box::new(simulated)),
        ("input Alarm_Values", Box::new(input_alarms)),
        (
            "output command",
            commanded(Box::new(
                MultiStateOutputObject::new(1, "MSO-1", 4).unwrap(),
            )),
        ),
        ("output Relinquish_Default", Box::new(output_default)),
        (
            "value command",
            commanded(Box::new(MultiStateValueObject::new(1, "MSV-1", 4).unwrap())),
        ),
        ("value Relinquish_Default", Box::new(value_default)),
        ("value Alarm_Values", Box::new(value_alarms)),
    ]
}

#[test]
fn a_shrink_that_would_strand_a_held_state_is_refused() {
    for (what, mut object) in holders() {
        let before = snapshot(&*object);
        assert_eq!(
            code(object.write_property(P::STATE_TEXT, None, labels(2), None)),
            ErrorCode::VALUE_OUT_OF_RANGE,
            "{what}"
        );
        assert_eq!(snapshot(&*object), before, "{what}");
        // A count that keeps every held state is taken.
        object
            .write_property(P::STATE_TEXT, None, labels(3), None)
            .unwrap();
        assert_eq!(count(&*object), PropertyValue::Unsigned(3), "{what}");
    }
}

#[test]
fn feedback_and_already_stranded_states_do_not_block_a_count() {
    // A Multi-state Output's Feedback_Value is sensed, not held: the shrink
    // goes ahead and the object reports the mismatch as a fault.
    let mut output = MultiStateOutputObject::new(1, "MSO-1", 4).unwrap();
    output
        .write_property(P::FEEDBACK_VALUE, None, PropertyValue::Unsigned(4), None)
        .unwrap();
    output
        .write_property(P::STATE_TEXT, None, labels(2), None)
        .unwrap();
    assert_eq!(
        output.read_property(P::RELIABILITY, None).unwrap(),
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );

    // A local shrink can leave a default past the count. A whole write that
    // doesn't strand anything more is taken, and the fault clears once the
    // count holds the default again.
    let mut value = MultiStateValueObject::new(1, "MSV-1", 4).unwrap();
    value.set_relinquish_default(4).unwrap();
    value.set_number_of_states(2).unwrap();
    assert_eq!(
        value.read_property(P::RELIABILITY, None).unwrap(),
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    value
        .write_property(P::STATE_TEXT, None, labels(1), None)
        .unwrap();
    value
        .write_property(P::STATE_TEXT, None, labels(4), None)
        .unwrap();
    assert_eq!(
        value.read_property(P::RELIABILITY, None).unwrap(),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
}

fn resize(object: &mut dyn BACnetObject, count: u64) -> Result<(), Error> {
    object.write_property(P::STATE_TEXT, Some(0), PropertyValue::Unsigned(count), None)
}

#[test]
fn a_count_at_index_0_resizes_state_text_and_the_count() {
    for mut object in all_three(3) {
        let kind = object.object_identifier().object_type();
        object
            .write_property(P::STATE_TEXT, None, labels(3), None)
            .unwrap();
        // A grow keeps the labels and appends a new object's defaults.
        resize(&mut *object, 5).unwrap();
        let mut grown = match labels(3) {
            PropertyValue::List(labels) => labels,
            _ => unreachable!(),
        };
        grown.extend(
            ["State 4", "State 5"].map(|label| PropertyValue::CharacterString(label.into())),
        );
        assert_eq!(
            snapshot(&*object),
            (PropertyValue::Unsigned(5), PropertyValue::List(grown)),
            "{kind:?}"
        );
        // A shrink drops the last labels.
        resize(&mut *object, 2).unwrap();
        assert_eq!(
            snapshot(&*object),
            (PropertyValue::Unsigned(2), labels(2)),
            "{kind:?}"
        );
        // Zero states, past the maximum, or another datatype change nothing.
        let before = snapshot(&*object);
        for (count, expected) in [
            (0, ErrorCode::VALUE_OUT_OF_RANGE),
            (
                u64::from(MAX_NUMBER_OF_STATES) + 1,
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
        ] {
            assert_eq!(
                code(resize(&mut *object, count)),
                expected,
                "{kind:?} {count}"
            );
            assert_eq!(snapshot(&*object), before, "{kind:?}");
        }
        assert_eq!(
            code(object.write_property(
                P::STATE_TEXT,
                Some(0),
                PropertyValue::CharacterString("3".into()),
                None
            )),
            ErrorCode::INVALID_DATA_TYPE
        );
        resize(&mut *object, u64::from(MAX_NUMBER_OF_STATES)).unwrap();
    }
}

#[test]
fn a_count_at_index_0_that_would_strand_a_held_state_is_refused() {
    for (what, mut object) in holders() {
        let before = snapshot(&*object);
        assert_eq!(
            code(resize(&mut *object, 2)),
            ErrorCode::VALUE_OUT_OF_RANGE,
            "{what}"
        );
        assert_eq!(snapshot(&*object), before, "{what}");
        resize(&mut *object, 3).unwrap();
        assert_eq!(count(&*object), PropertyValue::Unsigned(3), "{what}");
    }
}
