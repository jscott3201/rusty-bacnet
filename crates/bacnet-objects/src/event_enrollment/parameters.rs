//! Event_Parameters and Fault_Parameters as a write gives them: the
//! decoders a write runs, which a load of the stored octets runs too
//! (#2331), and the octets storage keeps.

use bacnet_types::constructed::{BACnetEventParameter, FaultParameters};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{DecodingKind, Error};
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use crate::common;

/// The most octets of Event_Parameters, or of Fault_Parameters, storage
/// keeps, as the value's BACnet encoding (#2331): 64 KiB. A list of
/// standard property states as long as the decoder takes fits with room to
/// spare: 10,000 entries, each at most six octets (a proprietary state
/// holds octets of its own and may be longer). So does every parameter set
/// a standard algorithm needs. An enrollment with storage refuses a longer
/// write; one without takes what the decoder takes.
pub(crate) const MAX_KEPT_OCTETS: usize = 64 * 1024;

/// Event_Parameters in the framed encoding a write carries and storage
/// keeps: one whole value, nothing after it.
pub(crate) fn framed_event_parameters(bytes: &[u8]) -> Result<BACnetEventParameter, Error> {
    match bacnet_encoding::constructed::decode_event_parameter(bytes, 0) {
        Ok((parameters, consumed)) if consumed == bytes.len() => Ok(parameters),
        // The one value the decoder can't convert is a string in a
        // character set it doesn't support (a change-of-characterstring
        // alarm value in DBCS, JIS or UCS-4).
        Err(error) if matches!(error.decode_fault(), Some((_, DecodingKind::Unsupported))) => Err(
            common::protocol_error(ErrorClass::PROPERTY, ErrorCode::CHARACTER_SET_NOT_SUPPORTED),
        ),
        _ => Err(common::invalid_data_type_error()),
    }
}

/// Fault_Parameters in the framed encoding, as [`framed_event_parameters`].
pub(crate) fn framed_fault_parameters(bytes: &[u8]) -> Result<FaultParameters, Error> {
    match bacnet_encoding::constructed::decode_fault_parameters(bytes, 0) {
        Ok((parameters, consumed)) if consumed == bytes.len() => Ok(parameters),
        _ => Err(common::invalid_data_type_error()),
    }
}

/// The framed encoding of `parameters`, which an enrollment serves and
/// storage keeps; an error for a value with none (an opaque body under a
/// tag the standard assigns, say), which no write leaves.
pub(crate) fn event_parameter_octets(parameters: &BACnetEventParameter) -> Result<BytesMut, Error> {
    let mut octets = BytesMut::new();
    bacnet_encoding::constructed::encode_event_parameter(&mut octets, parameters)?;
    Ok(octets)
}

/// The framed encoding of `parameters`, as [`event_parameter_octets`].
pub(crate) fn fault_parameter_octets(parameters: &FaultParameters) -> Result<BytesMut, Error> {
    let mut octets = BytesMut::new();
    bacnet_encoding::constructed::encode_fault_parameters(&mut octets, parameters)?;
    Ok(octets)
}

/// Why stored octets aren't a value storage keeps.
pub(crate) enum NotKept {
    /// They are no value of the property: the decoder's refusal, which an
    /// enrollment with storage answers a write with. Only that check,
    /// compiled with `event-reporting`, reads it; a file's reader calls
    /// such octets damage whatever the refusal was.
    Undecodable(#[cfg_attr(not(feature = "event-reporting"), allow(dead_code))] Error),
    /// They are a value, in another encoding than a save writes for it.
    Recoded,
}

/// The value `octets` hold, by the one rule for what storage keeps of
/// Event_Parameters and Fault_Parameters (#2331): the octets read back,
/// through the framed decoder a write of the property runs (`framed`), as
/// one whole value whose encoding (`octets_of`) is those octets again. A
/// load holds a file's row to it, and an enrollment with storage holds a
/// write to it before it saves, so what is saved is what a load takes
/// back, and a load followed by a save leaves the row as it was.
///
/// The decoder alone refuses octets after a value; the comparison would
/// refuse them too, a value's encoding being shorter than they are.
pub(crate) fn reloaded<T>(
    octets: &[u8],
    framed: fn(&[u8]) -> Result<T, Error>,
    octets_of: fn(&T) -> Result<BytesMut, Error>,
) -> Result<T, NotKept> {
    let value = framed(octets).map_err(NotKept::Undecodable)?;
    if octets_of(&value).ok().as_deref() != Some(octets) {
        return Err(NotKept::Recoded);
    }
    Ok(value)
}

pub(super) fn decode_event_parameters(value: PropertyValue) -> Result<BACnetEventParameter, Error> {
    let parameters = match value {
        // Sentinel 255 keeps legacy raw octets distinct from framed choices.
        PropertyValue::OctetString(data) => BACnetEventParameter::Opaque { tag: 0xFF, data },
        PropertyValue::ApplicationData(bytes) => framed_event_parameters(&bytes)?,
        // Older internal clients still use the flat application-tagged form.
        other => {
            BACnetEventParameter::decode(&other).map_err(|_| common::invalid_data_type_error())?
        }
    };

    event_parameter_octets(&parameters).map_err(|_| common::invalid_data_type_error())?;
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
        PropertyValue::ApplicationData(bytes) => framed_fault_parameters(&bytes)?,
        other => FaultParameters::decode_property_value(&other)
            .map_err(|_| common::invalid_data_type_error())?,
    };

    fault_parameter_octets(&parameters).map_err(|_| common::invalid_data_type_error())?;
    Ok(parameters)
}

/// A written Time_Delay_Normal: an Unsigned of at most 32 bits.
pub(super) fn written_time_delay_normal(value: &PropertyValue) -> Result<u32, Error> {
    match value {
        PropertyValue::Unsigned(delay) => common::u64_to_u32(*delay),
        _ => Err(common::invalid_data_type_error()),
    }
}
