//! The constructors that take `EngineeringUnits` (#915) serve that value
//! as the Units property (Output_Units for the Loop), as the same ENUMERATED
//! number: named units, proprietary ones, and a value above u16 that the
//! constructors pass through unchanged.

use crate::accumulator::{AccumulatorObject, PulseConverterObject};
use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::loop_obj::LoopObject;
use crate::present_value_access::PresentValueAccess;
use crate::traits::BACnetObject;
use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;

fn served(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

#[test]
fn constructors_serve_the_units_they_were_given() {
    let units = PropertyIdentifier::UNITS;
    for unit in [
        EngineeringUnits::DEGREES_CELSIUS,
        EngineeringUnits::PERCENT,
        EngineeringUnits::from_raw(256),
        EngineeringUnits::from_raw(65_535),
        // Above u16: the constructors store and serve the full u32.
        EngineeringUnits::from_raw(70_000),
    ] {
        let expected = PropertyValue::Enumerated(unit.to_raw());
        let objects: Vec<Box<dyn BACnetObject>> = vec![
            Box::new(AnalogInputObject::new(1, "ai", unit).unwrap()),
            Box::new(AnalogOutputObject::new(1, "ao", unit).unwrap()),
            Box::new(AnalogValueObject::new(1, "av", unit).unwrap()),
            Box::new(
                AnalogValueObject::with_access(1, "av-w", unit, PresentValueAccess::Writable)
                    .unwrap(),
            ),
            Box::new(AccumulatorObject::new(1, "acc", unit).unwrap()),
            Box::new(PulseConverterObject::new(1, "pc", unit).unwrap()),
        ];
        for object in &objects {
            assert_eq!(
                served(object.as_ref(), units),
                expected,
                "{} {unit:?}",
                object.object_name()
            );
        }
        let lp = LoopObject::new(1, "loop", unit).unwrap();
        assert_eq!(served(&lp, PropertyIdentifier::OUTPUT_UNITS), expected);
    }
}
