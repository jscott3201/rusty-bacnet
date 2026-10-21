//! `BACnetObjectPropertyReference` and its `BACnetSetpointReference` wrapper
//! (ASHRAE 135-2020 Clause 21) — wire codecs for the object-reference
//! properties of the Loop (Clause 12.17) and Pulse Converter (Clause 12.23)
//! objects.
//!
//! `BACnetObjectPropertyReference` names one property of an object, and can
//! narrow it to a single element when the property is an array:
//!
//! | Field | Tag | Type | Optional |
//! |---|---|---|---|
//! | `object-identifier` | `[0]` | `BACnetObjectIdentifier` | no |
//! | `property-identifier` | `[1]` | `BACnetPropertyIdentifier` | no |
//! | `property-array-index` | `[2]` | Unsigned | yes; only meaningful for array properties |
//!
//! `BACnetSetpointReference` wraps at most one such reference:
//!
//! | Field | Tag | Type | Optional |
//! |---|---|---|---|
//! | `setpoint-reference` | `[0]` | `BACnetObjectPropertyReference` | yes |
//!
//! Every member is context-tagged, so a property value carries a reference
//! as primitive context tags \[0\]/\[1\] (plus optional \[2\]) concatenated on
//! the wire; the `Setpoint_Reference` property nests those members in the
//! opening/closing tag 0 frame of `BACnetSetpointReference`, and leaves the
//! frame out entirely when it holds no reference (an absent optional member
//! has no encoding, Clause 20.2.16). Unlike
//! `BACnetDeviceObjectPropertyReference` there is NO device member in this
//! production — these references name objects in the local device only — so
//! a device-qualifying \[3\] element is rejected on decode rather than being
//! silently absorbed (the tranche-J `decode_dopr_body` codec accepts
//! \[3\]; this codec narrows it).

use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bytes::BytesMut;

pub use super::reference_sinks::{
    encode_object_property_reference_to_sink, encode_setpoint_reference_to_sink,
};

use super::tagged::{
    decode_ctx_constructed, decode_ctx_object_id, decode_ctx_unsigned, decode_optional_ctx,
    expect_end, next_is_context,
};

const WHAT: &str = "BACnetObjectPropertyReference";
const SETPOINT: &str = "BACnetSetpointReference";

/// Encode the bare `BACnetObjectPropertyReference` member sequence:
/// context-tagged \[0\]/\[1\] plus \[2\] when the reference is indexed.
pub fn encode_object_property_reference(buf: &mut BytesMut, r: &BACnetObjectPropertyReference) {
    encode_object_property_reference_to_sink(buf, r).expect("BytesMut accepts each reference");
}

/// Encode the `BACnetSetpointReference` form: the reference inside an
/// opening/closing context tag 0 frame (the production's
/// `setpoint-reference [0]` member).
pub fn encode_setpoint_reference(buf: &mut BytesMut, r: &BACnetObjectPropertyReference) {
    encode_setpoint_reference_to_sink(buf, r).expect("BytesMut accepts each reference");
}

/// Decode the bare `BACnetObjectPropertyReference` members starting at
/// `offset`, \[0\], \[1\] and the optional \[2\]; returns the reference
/// and the offset just past its last member.
///
/// The production is narrowed relative to the shared DOPR body codec: a
/// device-qualifying \[3\] right after the members is not part of it (the
/// Loop/Pulse Converter references are local-device only), so it is refused.
/// Anything else that follows is the caller's to judge.
pub fn decode_object_property_reference_at(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetObjectPropertyReference, usize), Error> {
    let (object_identifier, offset) = decode_ctx_object_id(data, offset, 0, WHAT)?;
    let (property_identifier, offset) = decode_ctx_unsigned::<u32>(data, offset, 1, WHAT)?;
    let (property_array_index, offset) =
        decode_optional_ctx(data, offset, 2, WHAT, decode_ctx_unsigned::<u32>)?;
    if next_is_context(data, offset, 3)? {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "[3] device-identifier is not part of this production",
        )
        .within(WHAT)
        .into());
    }
    Ok((
        BACnetObjectPropertyReference {
            object_identifier,
            property_identifier,
            property_array_index,
        },
        offset,
    ))
}

/// Decode a whole property-value payload as the bare
/// `BACnetObjectPropertyReference` member sequence: the members, read by
/// [`decode_object_property_reference_at`], must use all of `data`.
pub fn decode_object_property_reference(
    data: &[u8],
) -> Result<BACnetObjectPropertyReference, Error> {
    let (reference, end) = decode_object_property_reference_at(data, 0)?;
    expect_end(data, end, end, WHAT)?;
    Ok(reference)
}

/// Decode the `BACnetSetpointReference` frame at `offset` holding a
/// reference: opening context tag 0, the bare members, which must fill the
/// frame, and the closing tag. Returns the reference and the offset just
/// past the closing tag; what follows is the caller's to judge.
///
/// A frame with nothing inside (`0x0E 0x0F`) is the member present but
/// holding no object or property, which this refuses like any other
/// incomplete reference. The value without a reference has no frame at all;
/// see [`decode_setpoint_reference`].
pub fn decode_setpoint_reference_at(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetObjectPropertyReference, usize), Error> {
    let (inner, after) = decode_ctx_constructed(data, offset, 0, SETPOINT)?;
    let (reference, end) = decode_object_property_reference_at(inner, 0)?;
    expect_end(inner, end, offset, SETPOINT)?;
    Ok((reference, after))
}

/// Decode a whole property-value payload as the `BACnetSetpointReference`
/// form, the frame read by [`decode_setpoint_reference_at`], which must use
/// all of `data`.
///
/// The production's one member is optional, and an absent member has no
/// encoding (Clause 20.2.16), so an empty payload is the value without a
/// reference and yields `None`: Clause 12.17.16 then takes the setpoint from
/// the Loop's Setpoint property.
pub fn decode_setpoint_reference(
    data: &[u8],
) -> Result<Option<BACnetObjectPropertyReference>, Error> {
    if data.is_empty() {
        return Ok(None);
    }
    let (reference, after) = decode_setpoint_reference_at(data, 0)?;
    expect_end(data, after, after, SETPOINT)?;
    Ok(Some(reference))
}

#[cfg(test)]
mod tests;
