//! Shared selected-coordinate preparation; never substitutes Present_Value.
use bacnet_device_core::cov::{CovSample, validate_sample};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::{enums::PropertyIdentifier, error::Error, primitives::PropertyValue};
use bytes::BytesMut;

pub(crate) struct PreparedCovValue {
    pub sample: CovSample,
    pub encoded: Vec<u8>,
    pub(super) numeric: bool,
    pub(super) increment: Option<f64>,
}
impl PreparedCovValue {
    pub fn reports(&self, previous: Option<&CovSample>) -> bool {
        self.sample.reports(previous, self.increment, self.numeric)
    }
}
/// Caller has already performed the selected read, preserving its error precedence.
pub(crate) fn prepare_value(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    index: Option<u32>,
    increment: Option<f32>,
    value: &PropertyValue,
) -> Result<PreparedCovValue, Error> {
    let (sample, numeric) = validate_sample(object, property, index, value)?;
    let increment = effective_increment(object, property, numeric, increment);
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, sample.value())?;
    Ok(PreparedCovValue {
        sample,
        encoded: encoded.to_vec(),
        numeric,
        increment,
    })
}

/// Sole increment rule, shared with Trend Log COV acquisition (#1480): see
/// [`bacnet_objects::cov_change::effective_increment`].
pub(crate) use bacnet_objects::cov_change::effective_increment;

#[cfg(test)]
mod tests;
