//! Units on Integer, Positive Integer and Large Analog Value (#1092).
//!
//! Tables 12-50, 12-51 and 12-46 code Units R. It reads as a
//! BACnetEngineeringUnits enumeration, starts at NO_UNITS, is set locally
//! through `set_units`, and has no network write route.

use super::super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

fn assert_error<T: std::fmt::Debug>(result: Result<T, Error>, expected: ErrorCode) {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(code, expected, "expected {expected:?}");
        }
        other => panic!("expected PROPERTY/{expected:?}, got {other:?}"),
    }
}

fn units(object: &dyn BACnetObject) -> PropertyValue {
    object
        .read_property(PropertyIdentifier::UNITS, None)
        .unwrap()
}

/// Run the Units checks on one freshly built numeric value object.
macro_rules! assert_units_row {
    ($object:expr_2021) => {{
        let mut object = $object;
        let kind = object.object_identifier().object_type();
        assert_eq!(
            units(&object),
            PropertyValue::Enumerated(EngineeringUnits::NO_UNITS.to_raw()),
            "{kind:?}"
        );
        assert!(object.property_list().contains(&PropertyIdentifier::UNITS));
        assert!(
            object
                .required_properties()
                .contains(&PropertyIdentifier::UNITS)
        );
        assert!(!object.is_array_property(PropertyIdentifier::UNITS));

        object.set_units(EngineeringUnits::DEGREES_CELSIUS).unwrap();
        assert_eq!(object.units(), EngineeringUnits::DEGREES_CELSIUS);
        assert_eq!(
            units(&object),
            PropertyValue::Enumerated(EngineeringUnits::DEGREES_CELSIUS.to_raw())
        );
        // A vendor unit is kept; past 65535 is outside the enumeration.
        object
            .set_units(EngineeringUnits::from_raw(65_535))
            .unwrap();
        for raw in [65_536, u32::MAX] {
            assert_error(
                object.set_units(EngineeringUnits::from_raw(raw)),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            assert_eq!(object.units().to_raw(), 65_535, "{kind:?}");
        }

        // No network write route, whatever the value's datatype.
        assert!(!object.is_writable_property(PropertyIdentifier::UNITS));
        for value in [
            PropertyValue::Enumerated(EngineeringUnits::PERCENT.to_raw()),
            PropertyValue::Unsigned(98),
            PropertyValue::Null,
        ] {
            assert_error(
                object.write_property(PropertyIdentifier::UNITS, None, value, None),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
        assert_eq!(object.units().to_raw(), 65_535, "{kind:?}");
    }};
}

#[test]
fn numeric_value_units_start_at_no_units_and_are_set_locally() {
    assert_units_row!(IntegerValueObject::new(1, "IV-1").unwrap());
    assert_units_row!(PositiveIntegerValueObject::new(1, "PIV-1").unwrap());
    assert_units_row!(LargeAnalogValueObject::new(1, "LAV-1").unwrap());
}

#[test]
fn non_numeric_value_types_have_no_units() {
    let objects: [Box<dyn BACnetObject>; 4] = [
        Box::new(CharacterStringValueObject::new(1, "CSV-1").unwrap()),
        Box::new(DateValueObject::new(1, "DV-1").unwrap()),
        Box::new(TimeValueObject::new(1, "TV-1").unwrap()),
        Box::new(BitStringValueObject::new(1, "BSV-1").unwrap()),
    ];
    for object in objects {
        assert!(!object.property_list().contains(&PropertyIdentifier::UNITS));
        assert_error(
            object.read_property(PropertyIdentifier::UNITS, None),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
}
