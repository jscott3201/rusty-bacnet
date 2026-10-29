//! #1062: the Table 12-20 rows the Loop used to leave out. The three required
//! ones (Controlled_Variable_Units, Action, Priority_For_Writing) and the
//! units rows that footnotes 1 to 3 pair with each served gain constant.

use super::*;
use bacnet_types::enums::{Action, EngineeringUnits};
use bacnet_types::error::RemoteError;

const UNITS_ROWS: [PropertyIdentifier; 4] = [
    PropertyIdentifier::CONTROLLED_VARIABLE_UNITS,
    PropertyIdentifier::PROPORTIONAL_CONSTANT_UNITS,
    PropertyIdentifier::INTEGRAL_CONSTANT_UNITS,
    PropertyIdentifier::DERIVATIVE_CONSTANT_UNITS,
];

fn new_loop() -> LoopObject {
    LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap()
}

fn read(lo: &LoopObject, property: PropertyIdentifier) -> PropertyValue {
    lo.read_property(property, None).unwrap()
}

fn assert_property_error(result: Result<(), Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY && code == expected),
        "expected PROPERTY/{expected:?}, got {result:?}"
    );
}

fn set_out_of_service(lo: &mut LoopObject, value: bool) {
    lo.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(value),
        None,
    )
    .unwrap();
}

#[test]
fn loop_serves_the_missing_table_12_20_rows_with_their_datatypes() {
    let lo = new_loop();
    for property in UNITS_ROWS {
        assert_eq!(
            read(&lo, property),
            PropertyValue::Enumerated(EngineeringUnits::NO_UNITS.to_raw()),
            "{property:?}"
        );
    }
    assert_eq!(
        read(&lo, PropertyIdentifier::ACTION),
        PropertyValue::Enumerated(Action::DIRECT.to_raw())
    );
    assert_eq!(
        read(&lo, PropertyIdentifier::PRIORITY_FOR_WRITING),
        PropertyValue::Unsigned(16)
    );
    let list = lo.property_list();
    let required = lo.required_properties();
    for property in [
        PropertyIdentifier::CONTROLLED_VARIABLE_UNITS,
        PropertyIdentifier::ACTION,
        PropertyIdentifier::PRIORITY_FOR_WRITING,
    ] {
        assert!(list.contains(&property), "{property:?} in Property_List");
        assert!(required.contains(&property), "{property:?} is required");
    }
    // Each served gain constant comes with its units row (footnotes 1-3).
    for (constant, units) in [
        (
            PropertyIdentifier::PROPORTIONAL_CONSTANT,
            PropertyIdentifier::PROPORTIONAL_CONSTANT_UNITS,
        ),
        (
            PropertyIdentifier::INTEGRAL_CONSTANT,
            PropertyIdentifier::INTEGRAL_CONSTANT_UNITS,
        ),
        (
            PropertyIdentifier::DERIVATIVE_CONSTANT,
            PropertyIdentifier::DERIVATIVE_CONSTANT_UNITS,
        ),
    ] {
        assert!(list.contains(&constant) && list.contains(&units));
        assert!(!required.contains(&units), "{units:?} is optional");
    }
}

#[test]
fn loop_action_takes_direct_or_reverse_in_and_out_of_service() {
    let mut lo = new_loop();
    assert!(lo.is_writable_property(PropertyIdentifier::ACTION));
    for out_of_service in [false, true] {
        set_out_of_service(&mut lo, out_of_service);
        for action in [Action::REVERSE, Action::DIRECT] {
            lo.write_property(
                PropertyIdentifier::ACTION,
                None,
                PropertyValue::Enumerated(action.to_raw()),
                None,
            )
            .unwrap();
            assert_eq!(
                read(&lo, PropertyIdentifier::ACTION),
                PropertyValue::Enumerated(action.to_raw())
            );
        }
    }
}

#[test]
fn loop_action_refuses_values_outside_bacnet_action_atomically() {
    let mut lo = new_loop();
    lo.write_property(
        PropertyIdentifier::ACTION,
        None,
        PropertyValue::Enumerated(Action::REVERSE.to_raw()),
        None,
    )
    .unwrap();
    for (value, code) in [
        (PropertyValue::Enumerated(2), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            PropertyValue::Enumerated(u32::MAX),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (PropertyValue::Unsigned(0), ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Boolean(true), ErrorCode::INVALID_DATA_TYPE),
    ] {
        assert_property_error(
            lo.write_property(PropertyIdentifier::ACTION, None, value, None),
            code,
        );
        assert_eq!(
            read(&lo, PropertyIdentifier::ACTION),
            PropertyValue::Enumerated(Action::REVERSE.to_raw()),
            "a refused write keeps the action"
        );
    }
}

#[test]
fn loop_units_rows_are_application_set_and_read_only() {
    let mut lo = new_loop();
    lo.set_controlled_variable_units(EngineeringUnits::DEGREES_CELSIUS)
        .unwrap();
    lo.set_proportional_constant_units(EngineeringUnits::PERCENT)
        .unwrap();
    lo.set_integral_constant_units(EngineeringUnits::SECONDS)
        .unwrap();
    lo.set_derivative_constant_units(EngineeringUnits::from_raw(65_535))
        .unwrap();
    let expected = [
        EngineeringUnits::DEGREES_CELSIUS,
        EngineeringUnits::PERCENT,
        EngineeringUnits::SECONDS,
        EngineeringUnits::from_raw(65_535),
    ];
    for (property, units) in UNITS_ROWS.into_iter().zip(expected) {
        assert_eq!(
            read(&lo, property),
            PropertyValue::Enumerated(units.to_raw())
        );
        assert!(!lo.is_writable_property(property), "{property:?}");
        for out_of_service in [false, true] {
            set_out_of_service(&mut lo, out_of_service);
            assert_property_error(
                lo.write_property(
                    property,
                    None,
                    PropertyValue::Enumerated(EngineeringUnits::NO_UNITS.to_raw()),
                    None,
                ),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
        assert_eq!(
            read(&lo, property),
            PropertyValue::Enumerated(units.to_raw())
        );
    }
}

#[test]
fn loop_units_setters_refuse_values_beyond_engineering_units() {
    let mut lo = new_loop();
    let beyond = EngineeringUnits::from_raw(65_536);
    type UnitsSetter = fn(&mut LoopObject, EngineeringUnits) -> Result<(), Error>;
    let setters: [UnitsSetter; 4] = [
        LoopObject::set_controlled_variable_units,
        LoopObject::set_proportional_constant_units,
        LoopObject::set_integral_constant_units,
        LoopObject::set_derivative_constant_units,
    ];
    for (property, setter) in UNITS_ROWS.into_iter().zip(setters) {
        assert_property_error(setter(&mut lo, beyond), ErrorCode::VALUE_OUT_OF_RANGE);
        assert_eq!(
            read(&lo, property),
            PropertyValue::Enumerated(EngineeringUnits::NO_UNITS.to_raw()),
            "{property:?} unchanged"
        );
    }
}

#[test]
fn loop_priority_for_writing_is_application_set_in_one_to_sixteen() {
    let mut lo = new_loop();
    for priority in [1, 8, 16] {
        lo.set_priority_for_writing(priority).unwrap();
        assert_eq!(
            read(&lo, PropertyIdentifier::PRIORITY_FOR_WRITING),
            PropertyValue::Unsigned(priority.into())
        );
    }
    lo.set_priority_for_writing(9).unwrap();
    for priority in [0, 17, u8::MAX] {
        assert_property_error(
            lo.set_priority_for_writing(priority),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert!(!lo.is_writable_property(PropertyIdentifier::PRIORITY_FOR_WRITING));
    assert_property_error(
        lo.write_property(
            PropertyIdentifier::PRIORITY_FOR_WRITING,
            None,
            PropertyValue::Unsigned(1),
            None,
        ),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(
        read(&lo, PropertyIdentifier::PRIORITY_FOR_WRITING),
        PropertyValue::Unsigned(9)
    );
}

#[test]
fn loop_action_is_a_single_value_not_an_array() {
    // ACTION is BACnetARRAY[N] only on Command (Table 12-12).
    assert!(!new_loop().is_array_property(PropertyIdentifier::ACTION));
    assert!(
        crate::command::CommandObject::new(1, "CMD-1")
            .unwrap()
            .is_array_property(PropertyIdentifier::ACTION)
    );
}
