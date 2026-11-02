//! Units as a CreateObject initial value on the createable analog and
//! numeric value types
//! (#1429): `initialize_property` sets it, checked as BACnetEngineeringUnits,
//! while WriteProperty keeps refusing it.

use super::super::*;
use crate::creation::CreationTestExt;
use crate::property_metadata::is_writable_property;
use bacnet_types::enums::EngineeringUnits;
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

#[test]
fn the_analog_and_numeric_value_types_take_units_only_at_creation() {
    // Analog Value joined the other two when it became createable (#2172),
    // and the three numeric value types, whose tables also make Units
    // read-only, take it the same way.
    use crate::value_types::{
        IntegerValueObject, LargeAnalogValueObject, PositiveIntegerValueObject,
    };
    let objects: [Box<dyn BACnetObject>; 6] = [
        Box::new(AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap()),
        Box::new(AnalogOutputObject::new(1, "AO-1", EngineeringUnits::NO_UNITS).unwrap()),
        Box::new(AnalogValueObject::new(1, "AV-1", EngineeringUnits::NO_UNITS).unwrap()),
        Box::new(IntegerValueObject::new(1, "IV-1").unwrap()),
        Box::new(PositiveIntegerValueObject::new(1, "PIV-1").unwrap()),
        Box::new(LargeAnalogValueObject::new(1, "LAV-1").unwrap()),
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
        assert!(!is_writable_property(&*object, PropertyIdentifier::UNITS));
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
