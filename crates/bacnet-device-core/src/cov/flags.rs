//! Flags preparation shared by ordinary, Single and Multiple notification contexts.
use alloc::vec::Vec;

use super::{CovObservation, CovSample, validate_flags};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::property_metadata::has_property;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::{enums::PropertyIdentifier, error::Error, primitives::PropertyValue};
use bytes::BytesMut;

/// Captured once under the same object borrow as selected values. No callbacks
/// or mutable object state are retained through notification admission/send.
pub struct PreparedFlags {
    /// The object's Status_Flags, or `None` when it has none.
    pub value: Option<PropertyValue>,
    /// The same, encoded as a notification carries it.
    pub encoded: Option<Vec<u8>>,
}
impl PreparedFlags {
    /// Read and check `object`'s Status_Flags, if it declares them.
    pub fn read(object: &dyn BACnetObject) -> Result<Self, Error> {
        if !has_property(
            &object.property_metadata(),
            PropertyIdentifier::STATUS_FLAGS,
        ) {
            return Ok(Self {
                value: None,
                encoded: None,
            });
        }
        let value = object.read_property(PropertyIdentifier::STATUS_FLAGS, None)?;
        validate_flags(&value)?;
        let mut encoded = BytesMut::new();
        encode_property_value(&mut encoded, &value)?;
        Ok(Self {
            value: Some(value),
            encoded: Some(encoded.to_vec()),
        })
    }
    /// An observation of `sample` with these flags.
    pub fn observation(&self, sample: CovSample) -> CovObservation {
        CovObservation::new(sample, self.value.as_ref()).expect("prepared flags validated")
    }
    /// Status_Flags as the selected coordinate of a property subscription.
    pub fn selected(
        &self,
        object: &dyn BACnetObject,
        index: Option<u32>,
    ) -> Result<super::prepare::PreparedCovValue, Error> {
        let value = self
            .value
            .as_ref()
            .ok_or_else(|| Error::Encoding("Selected Status_Flags is not declared".into()))?;
        let (sample, numeric) =
            super::validate_sample(object, PropertyIdentifier::STATUS_FLAGS, index, value)?;
        Ok(super::prepare::PreparedCovValue {
            sample,
            encoded: self
                .encoded
                .as_ref()
                .expect("present flags encoded")
                .clone(),
            numeric,
            increment: None,
        })
    }
}
