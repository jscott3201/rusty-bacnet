//! Flags preparation shared by ordinary, Single and Multiple notification contexts.
use super::{observation::validate_flags, CovObservation, CovSample};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::{enums::PropertyIdentifier, error::Error, primitives::PropertyValue};
use bytes::BytesMut;

/// Captured once under the same object borrow as selected values. No callbacks
/// or mutable object state are retained through notification admission/send.
pub(crate) struct PreparedFlags {
    pub value: Option<PropertyValue>,
    pub encoded: Option<Vec<u8>>,
}
impl PreparedFlags {
    pub fn read(object: &dyn BACnetObject) -> Result<Self, Error> {
        if !object
            .property_list()
            .contains(&PropertyIdentifier::STATUS_FLAGS)
        {
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
    pub fn observation(&self, sample: CovSample) -> CovObservation {
        CovObservation::new(sample, self.value.as_ref()).expect("prepared flags validated")
    }
    pub fn selected(
        &self,
        object: &dyn BACnetObject,
        index: Option<u32>,
    ) -> Result<super::prepare::PreparedCovValue, Error> {
        let value = self
            .value
            .as_ref()
            .ok_or_else(|| Error::Encoding("Selected Status_Flags is not declared".into()))?;
        let (sample, numeric) = super::prepare::validate_sample(
            object,
            PropertyIdentifier::STATUS_FLAGS,
            index,
            value,
        )?;
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
