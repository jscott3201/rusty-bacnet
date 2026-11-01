//! Allocations a Property_List read makes (#2197). The served list is one
//! pass over the object's metadata rows: a whole-array read allocates only
//! the list it returns, and a count or one element allocates nothing beyond
//! what the object's rows themselves cost.

#![cfg(feature = "std")]

#[path = "../../bacnet-services/tests/support/counting_alloc.rs"]
mod counting_alloc;

use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier as P};
use counting_alloc::counted;

/// Allocations for a whole-array read, the count and the first element.
fn property_list_reads(object: &dyn BACnetObject) -> [usize; 3] {
    [None, Some(0), Some(1)].map(|index| {
        let (value, allocations) = counted(|| object.read_property(P::PROPERTY_LIST, index));
        value.expect("Property_List reads");
        allocations
    })
}

#[test]
fn property_list_read_allocations() {
    let input = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let device = DeviceObject::new(DeviceConfig::default()).unwrap();
    // Both serve borrowed rows, so the rows cost nothing. Before #2197 each
    // read collected the list twice: 9, 8 and 8 allocations.
    for (name, object) in [
        ("Analog Input", &input as &dyn BACnetObject),
        ("Device", &device),
    ] {
        let counts = property_list_reads(object);
        eprintln!("{name} [whole, count, element]: {counts:?}");
        assert_eq!(counts, [1, 0, 0], "{name}");
    }
}
