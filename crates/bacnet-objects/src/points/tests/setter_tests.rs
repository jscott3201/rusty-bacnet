//! The typed setters the builder uses (#2445): COV_Increment on the analog
//! objects, Active_Text and Inactive_Text on the binary ones and State_Text
//! on the multi-state ones, each checked as a write of it is; and the
//! default state labels, written without `core::fmt`.
use super::*;
use crate::common::numbered_name;

fn read(object: &dyn BACnetObject, property: P) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn labels(object: &dyn BACnetObject) -> Vec<String> {
    let PropertyValue::List(labels) = read(object, P::STATE_TEXT) else {
        panic!("State_Text is an array");
    };
    labels
        .into_iter()
        .map(|label| match label {
            PropertyValue::CharacterString(label) => label,
            other => panic!("a label {other:?}"),
        })
        .collect()
}

#[test]
fn set_cov_increment_takes_what_a_write_takes() {
    let units = EngineeringUnits::NO_UNITS;
    let mut input = AnalogInputObject::new(1, "ai", units).unwrap();
    let mut output = AnalogOutputObject::new(1, "ao", units).unwrap();
    let mut value = AnalogValueObject::new(1, "av", units).unwrap();
    macro_rules! check {
        ($object:ident) => {
            $object.set_cov_increment(0.5).unwrap();
            assert_eq!(read(&$object, P::COV_INCREMENT), PropertyValue::Real(0.5));
            for refused in [-0.25, f32::NAN, f32::INFINITY] {
                assert!($object.set_cov_increment(refused).is_err(), "{refused}");
                // A write of the same value is refused too.
                assert!(
                    $object
                        .write_property(P::COV_INCREMENT, None, PropertyValue::Real(refused), None)
                        .is_err()
                );
                assert_eq!(read(&$object, P::COV_INCREMENT), PropertyValue::Real(0.5));
            }
            $object.set_cov_increment(0.0).unwrap();
            assert_eq!(read(&$object, P::COV_INCREMENT), PropertyValue::Real(0.0));
        };
    }
    check!(input);
    check!(output);
    check!(value);
}

#[test]
fn the_binary_text_setters_take_any_text() {
    let mut input = BinaryInputObject::new(1, "bi").unwrap();
    let mut output = BinaryOutputObject::new(1, "bo").unwrap();
    let mut value = BinaryValueObject::new(1, "bv").unwrap();
    macro_rules! check {
        ($object:ident) => {
            $object.set_active_text("Open");
            $object.set_inactive_text("");
            assert_eq!(
                read(&$object, P::ACTIVE_TEXT),
                PropertyValue::CharacterString("Open".into())
            );
            assert_eq!(
                read(&$object, P::INACTIVE_TEXT),
                PropertyValue::CharacterString(String::new())
            );
        };
    }
    check!(input);
    check!(output);
    check!(value);
}

/// The labels replace the old ones and their count becomes
/// Number_Of_States; none, or more than the cap, is refused.
#[test]
fn set_state_texts_sets_the_labels_and_the_count() {
    let mut input = MultiStateInputObject::new(1, "msi", 2).unwrap();
    let mut output = MultiStateOutputObject::new(1, "mso", 2).unwrap();
    let mut value = MultiStateValueObject::new(1, "msv", 2).unwrap();
    let three = texts(&["Off", "Low", "High"]);
    let too_many = vec![String::from("x"); MAX_NUMBER_OF_STATES as usize + 1];
    macro_rules! check {
        ($object:ident) => {
            $object.set_state_texts(three.clone()).unwrap();
            assert_eq!(labels(&$object), three);
            assert_eq!(
                read(&$object, P::NUMBER_OF_STATES),
                PropertyValue::Unsigned(3)
            );
            for refused in [Vec::new(), too_many.clone()] {
                assert!($object.set_state_texts(refused).is_err());
                assert_eq!(labels(&$object), three);
                assert_eq!(
                    read(&$object, P::NUMBER_OF_STATES),
                    PropertyValue::Unsigned(3)
                );
            }
        };
    }
    check!(input);
    check!(output);
    check!(value);
}

/// A count that would strand a state the object holds is refused, as a
/// write of the whole array is, and nothing changes.
#[test]
fn set_state_texts_wont_strand_a_held_state() {
    let mut input = MultiStateInputObject::new(1, "msi", 3).unwrap();
    input.set_present_value(3);
    let mut output = MultiStateOutputObject::new(1, "mso", 3).unwrap();
    output.set_relinquish_default(3).unwrap();
    let mut value = MultiStateValueObject::new(1, "msv", 3).unwrap();
    value.set_alarm_values(vec![3]).unwrap();
    let two = texts(&["Off", "On"]);
    macro_rules! check {
        ($object:ident) => {
            let before = labels(&$object);
            assert!($object.set_state_texts(two.clone()).is_err());
            assert!(
                $object
                    .write_property(
                        P::STATE_TEXT,
                        None,
                        PropertyValue::List(
                            two.iter()
                                .cloned()
                                .map(PropertyValue::CharacterString)
                                .collect()
                        ),
                        None,
                    )
                    .is_err()
            );
            assert_eq!(labels(&$object), before);
            assert_eq!(
                read(&$object, P::NUMBER_OF_STATES),
                PropertyValue::Unsigned(3)
            );
        };
    }
    check!(input);
    check!(output);
    check!(value);
}

/// The labels a new object starts with, and those a grown count appends,
/// are the ones `format!` would write.
#[test]
fn the_default_labels_match_format() {
    let object = MultiStateValueObject::new(1, "msv", 12).unwrap();
    let expected: Vec<String> = (1..=12).map(|state| format!("State {state}")).collect();
    assert_eq!(labels(&object), expected);
    let mut grown = MultiStateInputObject::new(1, "msi", 1).unwrap();
    grown.set_number_of_states(11).unwrap();
    assert_eq!(labels(&grown), expected[..11]);
    for state in [0, 9, 10, 99, 100, MAX_NUMBER_OF_STATES, u32::MAX] {
        assert_eq!(numbered_name("State ", state), format!("State {state}"));
    }
}
