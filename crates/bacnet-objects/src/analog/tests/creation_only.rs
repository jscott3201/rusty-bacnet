//! Units as a CreateObject initial value on the createable analog types
//! (#1429): `initialize_property` sets it, checked as BACnetEngineeringUnits,
//! while WriteProperty keeps refusing it.

use super::super::*;
use bacnet_types::enums::EngineeringUnits;

fn code(result: Result<(), Error>) -> ErrorCode {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            code
        }
        other => panic!("expected a PROPERTY refusal, got {other:?}"),
    }
}

#[test]
fn analog_input_and_output_take_units_only_at_creation() {
    let objects: [Box<dyn BACnetObject>; 2] = [
        Box::new(AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap()),
        Box::new(AnalogOutputObject::new(1, "AO-1", EngineeringUnits::NO_UNITS).unwrap()),
    ];
    for mut object in objects {
        let kind = object.object_identifier().object_type();
        assert_eq!(
            object.creation_only_properties(),
            &[PropertyIdentifier::UNITS],
            "{kind:?}"
        );
        let units = |object: &dyn BACnetObject| {
            object
                .read_property(PropertyIdentifier::UNITS, None)
                .unwrap()
        };
        let celsius = EngineeringUnits::DEGREES_CELSIUS.to_raw();
        object
            .initialize_property(
                PropertyIdentifier::UNITS,
                PropertyValue::Enumerated(celsius),
            )
            .unwrap();
        assert_eq!(units(&*object), PropertyValue::Enumerated(celsius));
        // A vendor unit is kept; past 65535 is outside the enumeration.
        object
            .initialize_property(PropertyIdentifier::UNITS, PropertyValue::Enumerated(65_535))
            .unwrap();
        for (value, expected) in [
            (
                PropertyValue::Enumerated(65_536),
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            (PropertyValue::Unsigned(62), ErrorCode::INVALID_DATA_TYPE),
            (PropertyValue::Null, ErrorCode::INVALID_DATA_TYPE),
        ] {
            assert_eq!(
                code(object.initialize_property(PropertyIdentifier::UNITS, value)),
                expected,
                "{kind:?}"
            );
            assert_eq!(units(&*object), PropertyValue::Enumerated(65_535));
        }
        // Nothing else is set this way, and the write route still refuses
        // Units.
        assert_eq!(
            code(object.initialize_property(
                PropertyIdentifier::DESCRIPTION,
                PropertyValue::CharacterString("d".into())
            )),
            ErrorCode::WRITE_ACCESS_DENIED
        );
        assert!(!object.is_writable_property(PropertyIdentifier::UNITS));
        assert_eq!(
            code(object.write_property(
                PropertyIdentifier::UNITS,
                None,
                PropertyValue::Enumerated(celsius),
                None
            )),
            ErrorCode::WRITE_ACCESS_DENIED
        );
    }
}

#[test]
fn analog_value_is_not_createable_and_takes_nothing_at_creation() {
    let mut value = AnalogValueObject::new(1, "AV-1", EngineeringUnits::NO_UNITS).unwrap();
    assert!(!value.is_createable());
    assert!(value.creation_only_properties().is_empty());
    assert_eq!(
        code(value.initialize_property(PropertyIdentifier::UNITS, PropertyValue::Enumerated(62))),
        ErrorCode::WRITE_ACCESS_DENIED
    );
}
