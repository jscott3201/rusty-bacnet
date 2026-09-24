//! Shared selected-coordinate preparation; never substitutes Present_Value.
use super::CovSample;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::{
    enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier},
    error::Error,
    primitives::PropertyValue,
};
use bytes::BytesMut;

pub(crate) struct PreparedCovValue {
    pub sample: CovSample,
    pub encoded: Vec<u8>,
    numeric: bool,
    increment: Option<f32>,
}
impl PreparedCovValue {
    pub fn reports(&self, previous: Option<&CovSample>) -> bool {
        self.sample.reports(previous, self.increment, self.numeric)
    }
}
fn property_error(code: ErrorCode) -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY.to_raw() as u32,
        code: code.to_raw() as u32,
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
    let increment = if numeric && property == PropertyIdentifier::PRESENT_VALUE {
        increment.or_else(|| object.cov_increment())
    } else {
        increment
    };
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, sample.value())?;
    Ok(PreparedCovValue {
        sample,
        encoded: encoded.to_vec(),
        numeric,
        increment,
    })
}

/// Admission shares coordinate/retention checks without querying threshold policy.
pub(crate) fn validate_sample(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: &PropertyValue,
) -> Result<(CovSample, bool), Error> {
    let array = object.is_array_property(property);
    if index.is_some() && !array {
        return Err(property_error(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY));
    }
    if array
        && index.is_none()
        && !known_whole_array(object.object_identifier().object_type(), property)
    {
        return Err(property_error(ErrorCode::NOT_COV_PROPERTY));
    }
    let numeric = !(array && (index.is_none() || index == Some(0)))
        && matches!(
            value,
            PropertyValue::Real(_)
                | PropertyValue::Double(_)
                | PropertyValue::Signed(_)
                | PropertyValue::Unsigned(_)
        );
    Ok((CovSample::new(value)?, numeric))
}

fn known_whole_array(object: ObjectType, property: PropertyIdentifier) -> bool {
    use ObjectType as O;
    use PropertyIdentifier as P;
    let analog_binary_multi = matches!(
        object,
        O::ANALOG_INPUT
            | O::ANALOG_OUTPUT
            | O::ANALOG_VALUE
            | O::BINARY_INPUT
            | O::BINARY_OUTPUT
            | O::BINARY_VALUE
            | O::MULTI_STATE_INPUT
            | O::MULTI_STATE_OUTPUT
            | O::MULTI_STATE_VALUE
    );
    let value = matches!(
        object,
        O::BITSTRING_VALUE
            | O::CHARACTERSTRING_VALUE
            | O::DATEPATTERN_VALUE
            | O::DATE_VALUE
            | O::DATETIMEPATTERN_VALUE
            | O::DATETIME_VALUE
            | O::INTEGER_VALUE
            | O::LARGE_ANALOG_VALUE
            | O::OCTETSTRING_VALUE
            | O::POSITIVE_INTEGER_VALUE
            | O::TIMEPATTERN_VALUE
            | O::TIME_VALUE
    );
    match property {
        P::PROPERTY_LIST => {
            analog_binary_multi
                || value
                || matches!(
                    object,
                    O::ACCUMULATOR
                        | O::PULSE_CONVERTER
                        | O::COLOR
                        | O::COLOR_TEMPERATURE
                        | O::LIGHTING_OUTPUT
                        | O::BINARY_LIGHTING_OUTPUT
                        | O::LOOP
                        | O::ACCESS_DOOR
                )
        }
        P::PRIORITY_ARRAY => {
            value
                || matches!(
                    object,
                    O::ANALOG_OUTPUT
                        | O::ANALOG_VALUE
                        | O::BINARY_OUTPUT
                        | O::BINARY_VALUE
                        | O::MULTI_STATE_OUTPUT
                        | O::MULTI_STATE_VALUE
                        | O::LIGHTING_OUTPUT
                        | O::BINARY_LIGHTING_OUTPUT
                        | O::ACCESS_DOOR
                )
        }
        P::STATE_TEXT => matches!(
            object,
            O::MULTI_STATE_INPUT | O::MULTI_STATE_OUTPUT | O::MULTI_STATE_VALUE
        ),
        P::EVENT_TIME_STAMPS | P::EVENT_MESSAGE_TEXTS => analog_binary_multi,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
