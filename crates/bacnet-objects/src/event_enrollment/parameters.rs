use bacnet_types::constructed::{BACnetEventParameter, FaultParameters};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{DecodingKind, Error};
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
                // The one value the decoder can't convert is a string in a
                // character set it doesn't support (a change-of-characterstring
                // alarm value in DBCS, JIS or UCS-4).
                Err(error)
                    if matches!(error.decode_fault(), Some((_, DecodingKind::Unsupported))) =>
                {
                    return Err(common::protocol_error(
                        ErrorClass::PROPERTY,
                        ErrorCode::CHARACTER_SET_NOT_SUPPORTED,
                    ));
                }
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

/// A written Fault_Parameters value: the context-tagged BACnetFaultParameter
/// (the `none` choice included), or the flat form older internal clients
/// use. An application NULL is no member of the CHOICE, whose `none` is
/// context-tagged, so it is INVALID_DATA_TYPE like any other value of the
/// wrong datatype, which the server turns into the no-op Clause 15.9.2 makes
/// of it (#1417).
pub(super) fn decode_fault_parameters(value: PropertyValue) -> Result<FaultParameters, Error> {
    let parameters = match value {
        PropertyValue::ApplicationData(bytes) => {
            match bacnet_encoding::constructed::decode_fault_parameters(&bytes, 0) {
                Ok((parameters, consumed)) if consumed == bytes.len() => parameters,
                _ => return Err(common::invalid_data_type_error()),
            }
        }
        other => FaultParameters::decode_property_value(&other)
            .map_err(|_| common::invalid_data_type_error())?,
    };

    bacnet_encoding::constructed::encode_fault_parameters(&mut BytesMut::new(), &parameters)
        .map_err(|_| common::invalid_data_type_error())?;
    Ok(parameters)
}
