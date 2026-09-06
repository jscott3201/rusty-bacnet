//! `BACnetObjectPropertyReference` and its `BACnetSetpointReference` wrapper
//! (ASHRAE 135-2020 Clause 21) — wire codecs for the object-reference
//! properties of the Loop (Clause 12.17) and Pulse Converter (Clause 12.23)
//! objects.
//!
//! ```text
//! BACnetObjectPropertyReference ::= SEQUENCE {
//!     object-identifier    [0] BACnetObjectIdentifier,
//!     property-identifier  [1] BACnetPropertyIdentifier,
//!     property-array-index [2] Unsigned OPTIONAL  -- used only with array datatype
//! }
//!
//! BACnetSetpointReference ::= SEQUENCE {
//!     setpoint-reference [0] BACnetObjectPropertyReference OPTIONAL
//! }
//! ```
//!
//! Every member is context-tagged, so a property *write* carries a reference
//! as primitive context tags [0]/[1] (plus optional [2]) concatenated on the
//! wire; the `Setpoint_Reference` property nests those members in the
//! opening/closing tag 0 frame of `BACnetSetpointReference`. Unlike
//! `BACnetDeviceObjectPropertyReference` there is NO device member in this
//! production — these references name objects in the local device only — so
//! a device-qualifying [3] element is rejected on decode rather than being
//! silently absorbed (the tranche-J [`super::decode_dopr_body`] codec accepts
//! [3]; this codec narrows it).

use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::error::Error;
use bytes::BytesMut;

use crate::primitives;
use crate::tags;

use super::decode_dopr_body;

const WHAT: &str = "BACnetObjectPropertyReference";

/// Encode the bare `BACnetObjectPropertyReference` member sequence:
/// context-tagged [0]/[1] plus [2] when the reference is indexed.
pub fn encode_object_property_reference(buf: &mut BytesMut, r: &BACnetObjectPropertyReference) {
    primitives::encode_ctx_object_id(buf, 0, &r.object_identifier);
    primitives::encode_ctx_unsigned(buf, 1, r.property_identifier as u64);
    if let Some(index) = r.property_array_index {
        primitives::encode_ctx_unsigned(buf, 2, index as u64);
    }
}

/// Encode the `BACnetSetpointReference` form: the reference inside an
/// opening/closing context tag 0 frame (the production's
/// `setpoint-reference [0]` member).
pub fn encode_setpoint_reference(buf: &mut BytesMut, r: &BACnetObjectPropertyReference) {
    tags::encode_opening_tag(buf, 0);
    encode_object_property_reference(buf, r);
    tags::encode_closing_tag(buf, 0);
}

/// Decode a whole property-value payload as the bare
/// `BACnetObjectPropertyReference` member sequence.
///
/// Full consumption is required and the production is narrowed relative to
/// the shared DOPR body codec: a device-qualifying member `[3]` is not part
/// of `BACnetObjectPropertyReference` (the Loop/Pulse Converter references
/// are local-device only), so it is rejected — as is any other content
/// trailing the [0]/[1]/[2] members.
pub fn decode_object_property_reference(
    data: &[u8],
) -> Result<BACnetObjectPropertyReference, Error> {
    let (dopr, end) = decode_dopr_body(data, 0, WHAT)?;
    if end != data.len() {
        return Err(Error::decoding(
            end,
            format!("{WHAT}: trailing content after the reference members"),
        ));
    }
    if dopr.device_identifier.is_some() {
        return Err(Error::decoding(
            0,
            format!("{WHAT}: [3] device-identifier is not part of this production"),
        ));
    }
    Ok(BACnetObjectPropertyReference {
        object_identifier: dopr.object_identifier,
        property_identifier: dopr.property_identifier,
        property_array_index: dopr.property_array_index,
    })
}

/// Decode a whole property-value payload as the `BACnetSetpointReference`
/// form: an opening/closing context tag 0 frame whose content is the bare
/// reference, decoded with [`decode_object_property_reference`]'s strictness.
///
/// The production's member is `OPTIONAL`: an empty frame (`0x0E 0x0F`) is a
/// syntactically valid encoding of the ABSENT alternative (Clause 12.17,
/// `Setpoint_Reference`: "The absence of a reference indicates that the
/// setpoint for this control loop is fixed and is contained in the Setpoint
/// property") and yields `None` — not an encoding error.
pub fn decode_setpoint_reference(
    data: &[u8],
) -> Result<Option<BACnetObjectPropertyReference>, Error> {
    let what = "BACnetSetpointReference";
    let (tag, pos) = tags::decode_tag(data, 0)?;
    if !tag.is_opening_tag(0) {
        return Err(Error::decoding(
            0,
            format!("{what}: expected opening tag [0]"),
        ));
    }
    let (inner, after) = tags::extract_context_value(data, pos, 0)?;
    if after != data.len() {
        return Err(Error::decoding(
            after,
            format!("{what}: trailing content after the closing tag"),
        ));
    }
    if inner.is_empty() {
        return Ok(None); // setpoint-reference [0] absent (OPTIONAL member)
    }
    decode_object_property_reference(inner).map(Some)
}

#[cfg(test)]
mod tests;
