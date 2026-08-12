//! Full ASN.1 framing codecs for constructed Clause-21 types whose property
//! values are CHOICE/SEQUENCE productions — context-tagged framing per
//! ASHRAE 135-2020 Clause 20.2.1.5/20.2.1.6.
//!
//! These codecs encode to and decode from raw application-layer bytes, the
//! same bytes carried by [`bacnet_types::primitives::PropertyValue::ApplicationData`].
//! Objects serve framed properties (e.g. `Event_Parameters`,
//! `Fault_Parameters`, `Recipient_List`) by encoding with these functions;
//! the flat application-tagged model in [`crate::primitives::encode_property_value`]
//! cannot express their wire form.
//!
//! Tag-form rules applied here:
//!
//! - A CHOICE alternative over a `SEQUENCE` is an opening/closing context tag
//!   pair around the SEQUENCE's members.
//! - A CHOICE alternative over a primitive base type is a context-specific
//!   primitive tag holding the raw contents (e.g. `none [0] NULL`).
//! - A context-tagged inner `CHOICE` is always explicitly tagged: an
//!   opening/closing pair around the alternative's own encoding.
//! - Inner members declared `[n] T` over primitive `T` are context-tagged
//!   primitives; inner `SEQUENCE OF` / embedded `SEQUENCE` members are
//!   constructed opening/closing pairs holding application-tagged elements.

use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetPropertyStates};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::primitives;
use crate::tags::{self, TagClass};

pub mod event_parameter;
pub mod fault_parameter;
pub mod recipient;

pub use event_parameter::{decode_event_parameter, encode_event_parameter};
pub use fault_parameter::{decode_fault_parameters, encode_fault_parameters};
pub use recipient::{
    decode_destination, decode_destination_list, decode_recipient, encode_destination,
    encode_destination_list, encode_recipient,
};

/// Upper bound on decoded SEQUENCE OF / list lengths, mirroring the socket-
/// facing posture of `bacnet-services`' `MAX_DECODED_ITEMS`. Prevents memory
/// exhaustion from malformed framed payloads.
const MAX_FRAMED_ITEMS: usize = 10_000;

// ---------------------------------------------------------------------------
// Small tagged-field helpers
// ---------------------------------------------------------------------------

/// Require an opening context tag `tag` at `offset`; return the offset of its
/// content.
fn expect_opening(data: &[u8], offset: usize, tag: u8, what: &str) -> Result<usize, Error> {
    let (t, pos) = tags::decode_tag(data, offset)?;
    if !t.is_opening_tag(tag) {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected opening tag [{tag}]"),
        ));
    }
    Ok(pos)
}

/// Require a closing context tag `tag` at `offset`; return the offset past it.
fn expect_closing(data: &[u8], offset: usize, tag: u8, what: &str) -> Result<usize, Error> {
    let (t, pos) = tags::decode_tag(data, offset)?;
    if !t.is_closing_tag(tag) {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected closing tag [{tag}]"),
        ));
    }
    Ok(pos)
}

/// Decode a primitive context tag `tag` holding unsigned contents.
fn decode_ctx_unsigned(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &str,
) -> Result<(u64, usize), Error> {
    let (t, pos) = tags::decode_tag(data, offset)?;
    if !t.is_context(tag) {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected context tag [{tag}]"),
        ));
    }
    let end = pos
        .checked_add(t.length as usize)
        .ok_or_else(|| Error::decoding(pos, format!("{what}: length overflow")))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    Ok((primitives::decode_unsigned(&data[pos..end])?, end))
}

/// Decode a primitive context tag `tag` holding a 4-octet REAL.
fn decode_ctx_real(data: &[u8], offset: usize, tag: u8, what: &str) -> Result<(f32, usize), Error> {
    let (t, pos) = tags::decode_tag(data, offset)?;
    if !t.is_context(tag) || t.length != 4 {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected context tag [{tag}] REAL (4 octets)"),
        ));
    }
    let end = pos + 4;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    Ok((primitives::decode_real(&data[pos..end])?, end))
}

/// Decode a primitive context tag `tag` holding a BIT STRING
/// `(first octet = unused-bits count)`.
fn decode_ctx_bit_string(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &str,
) -> Result<((u8, Vec<u8>), usize), Error> {
    let (t, pos) = tags::decode_tag(data, offset)?;
    if !t.is_context(tag) {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected context tag [{tag}] BIT STRING"),
        ));
    }
    let end = pos
        .checked_add(t.length as usize)
        .ok_or_else(|| Error::decoding(pos, format!("{what}: length overflow")))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    let (unused_bits, bits) = primitives::decode_bit_string(&data[pos..end])?;
    Ok(((unused_bits, bits), end))
}

/// Decode one application-tagged BIT STRING (`SEQUENCE OF BIT STRING` item).
fn decode_app_bit_string(
    data: &[u8],
    offset: usize,
    what: &str,
) -> Result<((u8, Vec<u8>), usize), Error> {
    let (t, pos) = tags::decode_tag(data, offset)?;
    if t.class != TagClass::Application || t.number != tags::app_tag::BIT_STRING {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected application-tagged BIT STRING"),
        ));
    }
    let end = pos
        .checked_add(t.length as usize)
        .ok_or_else(|| Error::decoding(pos, format!("{what}: length overflow")))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    let (unused_bits, bits) = primitives::decode_bit_string(&data[pos..end])?;
    Ok(((unused_bits, bits), end))
}

/// Decode one application-tagged ENUMERATED (`SEQUENCE OF enumerated` item).
fn decode_app_enumerated(data: &[u8], offset: usize, what: &str) -> Result<(u32, usize), Error> {
    let (t, pos) = tags::decode_tag(data, offset)?;
    if t.class != TagClass::Application || t.number != tags::app_tag::ENUMERATED {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected application-tagged ENUMERATED"),
        ));
    }
    let end = pos
        .checked_add(t.length as usize)
        .ok_or_else(|| Error::decoding(pos, format!("{what}: length overflow")))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    let value = u32::try_from(primitives::decode_unsigned(&data[pos..end])?)
        .map_err(|_| Error::decoding(pos, format!("{what}: ENUMERATED exceeds u32")))?;
    Ok((value, end))
}

/// Decode one application-tagged CharacterString.
fn decode_app_character_string(
    data: &[u8],
    offset: usize,
    what: &str,
) -> Result<(String, usize), Error> {
    let (t, pos) = tags::decode_tag(data, offset)?;
    if t.class != TagClass::Application || t.number != tags::app_tag::CHARACTER_STRING {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected application-tagged CharacterString"),
        ));
    }
    let end = pos
        .checked_add(t.length as usize)
        .ok_or_else(|| Error::decoding(pos, format!("{what}: length overflow")))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    Ok((primitives::decode_character_string(&data[pos..end])?, end))
}

// ---------------------------------------------------------------------------
// BACnetPropertyStates (Clause 21 CHOICE) — spec-tagged framing
// ---------------------------------------------------------------------------

/// Encode a [`BACnetPropertyStates`] with the CHOICE tags of the Clause 21
/// production (135-2020).
///
/// The enum-value-typed alternatives travel as unsigned-encoded contents
/// under their CHOICE tag, `boolean-value [0]` as the context-tagged Boolean
/// form (one contents octet, Clause 20.2), and `Other` as raw contents under
/// its recorded tag.
///
/// NOTE: the 135-2020 production numbers `restart-reason [14]` and shifts
/// the door alternatives to 15–19; the timer/lift alternatives sit at
/// 43/44/52/53. This codec follows 135-2020 throughout — which corrects the
/// tag numbers the legacy flat property-value codec used for those variants.
pub fn encode_property_state(buf: &mut BytesMut, state: &BACnetPropertyStates) {
    use BACnetPropertyStates as S;
    match state {
        S::BooleanValue(v) => primitives::encode_ctx_boolean(buf, 0, *v),
        S::BinaryValue(v) => primitives::encode_ctx_enumerated(buf, 1, *v),
        S::EventType(v) => primitives::encode_ctx_enumerated(buf, 2, *v),
        S::Polarity(v) => primitives::encode_ctx_enumerated(buf, 3, *v),
        S::ProgramChange(v) => primitives::encode_ctx_enumerated(buf, 4, *v),
        S::ProgramState(v) => primitives::encode_ctx_enumerated(buf, 5, *v),
        S::ReasonForHalt(v) => primitives::encode_ctx_enumerated(buf, 6, *v),
        S::Reliability(v) => primitives::encode_ctx_enumerated(buf, 7, *v),
        S::State(v) => primitives::encode_ctx_enumerated(buf, 8, *v),
        S::SystemStatus(v) => primitives::encode_ctx_enumerated(buf, 9, *v),
        S::Units(v) => primitives::encode_ctx_enumerated(buf, 10, *v),
        S::UnsignedValue(v) => primitives::encode_ctx_unsigned(buf, 11, *v as u64),
        S::LifeSafetyMode(v) => primitives::encode_ctx_enumerated(buf, 12, *v),
        S::LifeSafetyState(v) => primitives::encode_ctx_enumerated(buf, 13, *v),
        // 135-2020: [14] is restart-reason (unmodeled); door alternatives
        // moved to 15..=19.
        S::DoorAlarmState(v) => primitives::encode_ctx_enumerated(buf, 15, *v),
        S::Action(v) => primitives::encode_ctx_enumerated(buf, 16, *v),
        S::DoorSecuredStatus(v) => primitives::encode_ctx_enumerated(buf, 17, *v),
        S::DoorStatus(v) => primitives::encode_ctx_enumerated(buf, 18, *v),
        S::DoorValue(v) => primitives::encode_ctx_enumerated(buf, 19, *v),
        S::TimerState(v) => primitives::encode_ctx_enumerated(buf, 43, *v),
        S::TimerTransition(v) => primitives::encode_ctx_enumerated(buf, 44, *v),
        S::LiftCarDirection(v) => primitives::encode_ctx_enumerated(buf, 52, *v),
        S::LiftCarDoorCommand(v) => primitives::encode_ctx_enumerated(buf, 53, *v),
        S::Other { tag, data } => primitives::encode_ctx_octet_string(buf, *tag, data),
    }
}

/// Decode one [`BACnetPropertyStates`] CHOICE element at `offset`.
///
/// Modeled alternatives decode by their Clause 21 tag; any other tag
/// (including the ones with no Rust variant, e.g. `restart-reason [14]` or
/// `integer-value [41]` INTEGER) is preserved verbatim as
/// [`BACnetPropertyStates::Other`].
pub fn decode_property_state(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetPropertyStates, usize), Error> {
    use BACnetPropertyStates as S;
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != TagClass::Context || tag.is_opening || tag.is_closing {
        return Err(Error::decoding(
            offset,
            "BACnetPropertyStates: expected a primitive context tag",
        ));
    }
    let end = pos
        .checked_add(tag.length as usize)
        .ok_or_else(|| Error::decoding(pos, "BACnetPropertyStates: length overflow"))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    let content = &data[pos..end];
    let unsigned = || -> Result<u32, Error> {
        u32::try_from(primitives::decode_unsigned(content)?)
            .map_err(|_| Error::decoding(pos, "BACnetPropertyStates: contents exceed u32"))
    };
    let state = match tag.number {
        0 => {
            if content.len() != 1 {
                return Err(Error::decoding(
                    offset,
                    format!(
                        "BACnetPropertyStates boolean-value: expected 1 contents octet, got {}",
                        content.len()
                    ),
                ));
            }
            S::BooleanValue(content[0] != 0)
        }
        1 => S::BinaryValue(unsigned()?),
        2 => S::EventType(unsigned()?),
        3 => S::Polarity(unsigned()?),
        4 => S::ProgramChange(unsigned()?),
        5 => S::ProgramState(unsigned()?),
        6 => S::ReasonForHalt(unsigned()?),
        7 => S::Reliability(unsigned()?),
        8 => S::State(unsigned()?),
        9 => S::SystemStatus(unsigned()?),
        10 => S::Units(unsigned()?),
        11 => S::UnsignedValue(unsigned()?),
        12 => S::LifeSafetyMode(unsigned()?),
        13 => S::LifeSafetyState(unsigned()?),
        15 => S::DoorAlarmState(unsigned()?),
        16 => S::Action(unsigned()?),
        17 => S::DoorSecuredStatus(unsigned()?),
        18 => S::DoorStatus(unsigned()?),
        19 => S::DoorValue(unsigned()?),
        43 => S::TimerState(unsigned()?),
        44 => S::TimerTransition(unsigned()?),
        52 => S::LiftCarDirection(unsigned()?),
        53 => S::LiftCarDoorCommand(unsigned()?),
        other => S::Other {
            tag: other,
            data: content.to_vec(),
        },
    };
    Ok((state, end))
}

// ---------------------------------------------------------------------------
// BACnetDeviceObjectPropertyReference (Clause 21 SEQUENCE)
// ---------------------------------------------------------------------------

/// Encode the context-tagged members of a `BACnetDeviceObjectPropertyReference`:
/// `object-identifier [0]`, `property-identifier [1]`, optional
/// `property-array-index [2]`, optional `device-identifier [3]` — the body
/// that sits between the enclosing field's opening/closing tags.
pub(crate) fn encode_dopr_body(buf: &mut BytesMut, r: &BACnetDeviceObjectPropertyReference) {
    primitives::encode_ctx_object_id(buf, 0, &r.object_identifier);
    primitives::encode_ctx_unsigned(buf, 1, r.property_identifier as u64);
    if let Some(index) = r.property_array_index {
        primitives::encode_ctx_unsigned(buf, 2, index as u64);
    }
    if let Some(ref device) = r.device_identifier {
        primitives::encode_ctx_object_id(buf, 3, device);
    }
}

/// Decode the members written by [`encode_dopr_body`], stopping before the
/// enclosing closing tag.
pub(crate) fn decode_dopr_body(
    data: &[u8],
    offset: usize,
    what: &str,
) -> Result<(BACnetDeviceObjectPropertyReference, usize), Error> {
    // [0] object-identifier
    let (t, pos) = tags::decode_tag(data, offset)?;
    if !t.is_context(0) || t.length != 4 {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected [0] object-identifier (4 octets)"),
        ));
    }
    let end = pos + 4;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
    let mut offset = end;

    // [1] property-identifier
    let (raw, new_offset) = decode_ctx_unsigned(data, offset, 1, what)?;
    let property_identifier = u32::try_from(raw)
        .map_err(|_| Error::decoding(offset, format!("{what}: property-identifier exceeds u32")))?;
    offset = new_offset;

    // [2] property-array-index OPTIONAL
    let mut property_array_index = None;
    if offset < data.len() {
        let (peek, peek_pos) = tags::decode_tag(data, offset)?;
        if peek.is_context(2) {
            let end = peek_pos
                .checked_add(peek.length as usize)
                .ok_or_else(|| Error::decoding(peek_pos, format!("{what}: length overflow")))?;
            if end > data.len() {
                return Err(Error::buffer_too_short(end, data.len()));
            }
            let index = primitives::decode_unsigned(&data[peek_pos..end])?;
            property_array_index = Some(u32::try_from(index).map_err(|_| {
                Error::decoding(offset, format!("{what}: property-array-index exceeds u32"))
            })?);
            offset = end;
        }
    }

    // [3] device-identifier OPTIONAL
    let mut device_identifier = None;
    if offset < data.len() {
        let (peek, peek_pos) = tags::decode_tag(data, offset)?;
        if peek.is_context(3) {
            if peek.length != 4 {
                return Err(Error::decoding(
                    offset,
                    format!("{what}: expected [3] device-identifier (4 octets)"),
                ));
            }
            let end = peek_pos + 4;
            if end > data.len() {
                return Err(Error::buffer_too_short(end, data.len()));
            }
            device_identifier = Some(ObjectIdentifier::decode(&data[peek_pos..end])?);
            offset = end;
        }
    }

    Ok((
        BACnetDeviceObjectPropertyReference {
            object_identifier,
            property_identifier,
            property_array_index,
            device_identifier,
        },
        offset,
    ))
}

#[cfg(test)]
mod tests;
