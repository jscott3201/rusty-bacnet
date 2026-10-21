//! Decoding a write of a command property whose datatype is an unframed
//! SEQUENCE that opens with its operation under context tag 0: Lighting
//! Output's Lighting_Command (Clause 12.54) and the Color_Command of Color
//! and Color Temperature (Addendum 135-2020ca).
//!
//! The write reaches the object as the SEQUENCE's octets in one
//! `ApplicationData`. Error pairings follow Clause 15.9.1.3 and the Loop
//! references (#1312): a value that doesn't open with the operation field,
//! such as any application-tagged value, is INVALID_DATA_TYPE; octets that
//! then fail to decode as exactly one command are INVALID_DATA_ENCODING; and
//! an operation or field too wide for its type is VALUE_OUT_OF_RANGE, as is
//! any command the object then refuses.

use bacnet_encoding::tags;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

/// Decode a command write with `decode`, the codec for a value that fills
/// its input, mapping its failures to the Clause 15.9.1.3 errors.
pub(crate) fn decode_command_write<T>(
    value: PropertyValue,
    decode: fn(&[u8]) -> Result<T, Error>,
) -> Result<T, Error> {
    let PropertyValue::ApplicationData(bytes) = value else {
        return Err(super::invalid_data_type_error());
    };
    match tags::decode_tag(&bytes, 0) {
        Ok((tag, _)) if tag.is_context(0) => {}
        Ok(_) => return Err(super::invalid_data_type_error()),
        Err(_) => return Err(super::invalid_data_encoding_error()),
    }
    // The codecs report a field too wide for its type as a static range
    // error, and only once the octets are otherwise exactly one command; any
    // other failure is a broken encoding.
    decode(&bytes).map_err(|error| match error {
        Error::Range(_) => super::value_out_of_range_error(),
        _ => super::invalid_data_encoding_error(),
    })
}
