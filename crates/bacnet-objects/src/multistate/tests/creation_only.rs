//! Number_Of_States as a CreateObject initial value (#1429):
//! `initialize_property` checks it and changes nothing on a refusal, and
//! WriteProperty keeps refusing it. State_Text written whole goes the write
//! route now, which sets the count from it (#1443).

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

fn labels(labels: &[&str]) -> PropertyValue {
    PropertyValue::List(
        labels
            .iter()
            .map(|label| PropertyValue::CharacterString((*label).into()))
            .collect(),
    )
}

fn states(object: &dyn BACnetObject) -> (PropertyValue, PropertyValue) {
    (
        object.read_property(P::NUMBER_OF_STATES, None).unwrap(),
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
fn a_new_count_resizes_state_text() {
    for mut object in all_three(2) {
        let kind = object.object_identifier().object_type();
        assert_eq!(
            object.creation_only_properties(),
            &[P::NUMBER_OF_STATES],
            "{kind:?}"
        );
        object
            .write_property(P::STATE_TEXT, None, labels(&["Off", "On"]), None)
            .unwrap();
        object
            .initialize_property(P::NUMBER_OF_STATES, PropertyValue::Unsigned(3))
            .unwrap();
        // Growth keeps the labels and adds the default one.
        assert_eq!(
            states(&*object),
            (
                PropertyValue::Unsigned(3),
                labels(&["Off", "On", "State 3"])
            ),
            "{kind:?}"
        );
        let before = states(&*object);
        for (value, expected) in [
            (PropertyValue::Unsigned(0), ErrorCode::VALUE_OUT_OF_RANGE),
            (
                PropertyValue::Unsigned(u64::from(MAX_NUMBER_OF_STATES) + 1),
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            (
                PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            (PropertyValue::Enumerated(3), ErrorCode::INVALID_DATA_TYPE),
        ] {
            assert_eq!(
                code(object.initialize_property(P::NUMBER_OF_STATES, value.clone())),
                expected,
                "{kind:?} {value:?}"
            );
            assert_eq!(states(&*object), before, "{kind:?}");
        }
        // The cap itself is a valid count, and one state takes one label.
        object
            .initialize_property(
                P::NUMBER_OF_STATES,
                PropertyValue::Unsigned(MAX_NUMBER_OF_STATES.into()),
            )
            .unwrap();
        object
            .initialize_property(P::NUMBER_OF_STATES, PropertyValue::Unsigned(1))
            .unwrap();
        assert_eq!(
            states(&*object),
            (PropertyValue::Unsigned(1), labels(&["Off"]))
        );
        // Nothing else is set this way, State_Text written whole included;
        // WriteProperty refuses a whole Number_Of_States.
        for property in [P::DESCRIPTION, P::STATE_TEXT] {
            assert_eq!(
                code(object.initialize_property(property, labels(&["Only"]))),
                ErrorCode::WRITE_ACCESS_DENIED,
                "{kind:?} {property:?}"
            );
        }
        assert_eq!(
            code(object.write_property(
                P::NUMBER_OF_STATES,
                None,
                PropertyValue::Unsigned(1),
                None
            )),
            ErrorCode::WRITE_ACCESS_DENIED,
            "{kind:?}"
        );
    }
}

#[test]
fn a_count_that_would_strand_a_held_state_is_refused() {
    let five = PropertyValue::Unsigned(5);
    let three = PropertyValue::Unsigned(3);

    // Present_Value, commanded or simulated.
    let mut input = MultiStateInputObject::new(1, "MSI-1", 5).unwrap();
    input.set_present_value(5);
    let mut output = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
    output
        .write_property_from(
            P::PRESENT_VALUE,
            None,
            five.clone(),
            Some(8),
            &test_origin(),
        )
        .unwrap();
    let mut value = MultiStateValueObject::new(1, "MSV-1", 5).unwrap();
    value
        .write_property_from(
            P::PRESENT_VALUE,
            None,
            five.clone(),
            Some(8),
            &test_origin(),
        )
        .unwrap();
    // Relinquish_Default, and Alarm_Values.
    let mut default = MultiStateValueObject::new(2, "MSV-2", 5).unwrap();
    default.set_relinquish_default(5).unwrap();
    let mut alarms = MultiStateInputObject::new(2, "MSI-2", 5).unwrap();
    alarms.set_alarm_values(vec![1, 5]);
    let objects: [&mut dyn BACnetObject; 5] = [
        &mut input,
        &mut output,
        &mut value,
        &mut default,
        &mut alarms,
    ];
    for object in objects {
        let name = object.object_name().to_owned();
        assert_eq!(
            code(object.initialize_property(P::NUMBER_OF_STATES, three.clone())),
            ErrorCode::VALUE_OUT_OF_RANGE,
            "{name}"
        );
        assert_eq!(
            object.read_property(P::NUMBER_OF_STATES, None).unwrap(),
            five
        );
        // A count that still holds every state is taken.
        object
            .initialize_property(P::NUMBER_OF_STATES, PropertyValue::Unsigned(6))
            .unwrap();
    }

    // A Multi-state Output's Feedback_Value is sensed, not held: it doesn't
    // stop the count, and the object reports the mismatch as a fault.
    let mut output = MultiStateOutputObject::new(3, "MSO-3", 5).unwrap();
    output
        .write_property(P::FEEDBACK_VALUE, None, five, None)
        .unwrap();
    output
        .initialize_property(P::NUMBER_OF_STATES, three)
        .unwrap();
    assert_eq!(
        output.read_property(P::RELIABILITY, None).unwrap(),
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
}
