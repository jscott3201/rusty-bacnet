//! The probe with every provided `BACnetObject` method left at its default.

use bacnet_objects::property_metadata::PropertyMetadata;
use std::borrow::Cow;

use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::probe::Probe;

/// A probe's identity and readings, every provided method left at the trait
/// default: what an adapter that forwarded nothing more would answer.
pub struct Defaults(pub Probe);

impl BACnetObject for Defaults {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.0.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.0.object_name()
    }
    fn read_property(&self, property: P, index: Option<u32>) -> Result<PropertyValue, Error> {
        self.0.read_property(property, index)
    }
    fn write_property(
        &mut self,
        property: P,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.0.write_property(property, index, value, priority)
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.0.property_metadata()
    }
    fn property_list(&self) -> Cow<'static, [P]> {
        self.0.property_list()
    }
}
