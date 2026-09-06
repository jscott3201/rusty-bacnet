use bacnet_types::constructed::{BACnetEventParameter, FaultParameters};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use crate::common;

pub(super) fn decode_event_parameters(value: PropertyValue) -> Result<BACnetEventParameter, Error> {
    let parameters = match value {
        // Sentinel 255 keeps legacy raw octets distinct from framed choices.
        PropertyValue::OctetString(data) => BACnetEventParameter::Opaque { tag: 0xFF, data },
        PropertyValue::ApplicationData(bytes) => {
            match bacnet_encoding::constructed::decode_event_parameter(&bytes, 0) {
                Ok((parameters, consumed)) if consumed == bytes.len() => parameters,
                _ => return Err(common::invalid_data_type_error()),
            }
        }
        // Older internal clients still use the flat application-tagged form.
        other => {
            BACnetEventParameter::decode(&other).map_err(|_| common::invalid_data_type_error())?
        }
    };

    bacnet_encoding::constructed::encode_event_parameter(&mut BytesMut::new(), &parameters)
        .map_err(|_| common::invalid_data_type_error())?;
    Ok(parameters)
}

pub(super) fn decode_fault_parameters(
    value: PropertyValue,
) -> Result<Option<FaultParameters>, Error> {
    let parameters = match value {
        PropertyValue::Null => None,
        PropertyValue::ApplicationData(bytes) => {
            match bacnet_encoding::constructed::decode_fault_parameters(&bytes, 0) {
                Ok((parameters, consumed)) if consumed == bytes.len() => Some(parameters),
                _ => return Err(common::invalid_data_type_error()),
            }
        }
        other => Some(
            FaultParameters::decode_property_value(&other)
                .map_err(|_| common::invalid_data_type_error())?,
        ),
    };

    if let Some(parameters) = &parameters {
        bacnet_encoding::constructed::encode_fault_parameters(&mut BytesMut::new(), parameters)
            .map_err(|_| common::invalid_data_type_error())?;
    }
    Ok(parameters)
}
