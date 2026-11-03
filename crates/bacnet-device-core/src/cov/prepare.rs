//! The checks a COV coordinate passes before it is admitted: whether its
//! property and index can be monitored, and its value retained.
use bacnet_objects::property_metadata::is_array_property;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::{
    enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier},
    error::Error,
    primitives::PropertyValue,
};

use super::CovSample;

fn property_error(code: ErrorCode) -> Error {
    Error::protocol(ErrorClass::PROPERTY, code)
}

/// Admission shares coordinate/retention checks without querying threshold policy.
/// Returns the retained sample and whether the value is numeric.
pub fn validate_sample(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: &PropertyValue,
) -> Result<(CovSample, bool), Error> {
    let array = is_array_property(object, property);
    if index.is_some() && !array {
        return Err(property_error(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY));
    }
    if array
        && index.is_none()
        && !known_whole_array(object.object_identifier().object_type(), property)
    {
        return Err(property_error(ErrorCode::NOT_COV_PROPERTY));
    }
    let numeric = bacnet_objects::cov_change::is_numeric(object, property, index, value);
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
        // Event_Message_Texts_Config: #1329.
        P::EVENT_TIME_STAMPS | P::EVENT_MESSAGE_TEXTS | P::EVENT_MESSAGE_TEXTS_CONFIG => {
            analog_binary_multi
        }
        _ => false,
    }
}
