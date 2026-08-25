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

use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetExtendedPropertyState, BACnetPropertyStates,
    BACnetProprietaryPropertyState,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::primitives;
use crate::tags::{self, TagClass};

pub mod cov_subscription;
pub mod event_parameter;
pub mod fault_parameter;
pub mod object_property_reference;
pub mod recipient;

pub use cov_subscription::{encode_cov_subscription, encode_cov_subscription_list};
pub use event_parameter::{decode_event_parameter, encode_event_parameter};
pub use fault_parameter::{decode_fault_parameters, encode_fault_parameters};
pub use object_property_reference::{
    decode_object_property_reference, decode_setpoint_reference, encode_object_property_reference,
    encode_setpoint_reference,
};
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

/// Encode a [`BACnetPropertyStates`] using the Standard 135-2020 Clause 21 tags.
///
/// Returns an error without modifying `buf` when a constructed proprietary
/// value does not contain a BACnet TLV sequence.
pub fn encode_property_state(
    buf: &mut BytesMut,
    state: &BACnetPropertyStates,
) -> Result<(), Error> {
    use BACnetPropertyStates as S;
    if let S::Other(value) = state {
        if value.is_constructed() {
            validate_tlv_sequence(value.data(), "proprietary property-state body")?;
            let mut framed = BytesMut::new();
            tags::encode_opening_tag(&mut framed, value.tag());
            let (_, body_start) = tags::decode_tag(&framed, 0)?;
            framed.extend_from_slice(value.data());
            tags::encode_closing_tag(&mut framed, value.tag());
            let (_, end) = tags::extract_context_value(&framed, body_start, value.tag())?;
            if end != framed.len() {
                return Err(Error::decoding(
                    end,
                    "proprietary property-state body has trailing data",
                ));
            }
        }
    }
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
        S::RestartReason(v) => primitives::encode_ctx_enumerated(buf, 14, *v),
        S::DoorAlarmState(v) => primitives::encode_ctx_enumerated(buf, 15, *v),
        S::Action(v) => primitives::encode_ctx_enumerated(buf, 16, *v),
        S::DoorSecuredStatus(v) => primitives::encode_ctx_enumerated(buf, 17, *v),
        S::DoorStatus(v) => primitives::encode_ctx_enumerated(buf, 18, *v),
        S::DoorValue(v) => primitives::encode_ctx_enumerated(buf, 19, *v),
        S::FileAccessMethod(v) => primitives::encode_ctx_enumerated(buf, 20, *v),
        S::LockStatus(v) => primitives::encode_ctx_enumerated(buf, 21, *v),
        S::LifeSafetyOperation(v) => primitives::encode_ctx_enumerated(buf, 22, *v),
        S::Maintenance(v) => primitives::encode_ctx_enumerated(buf, 23, *v),
        S::NodeType(v) => primitives::encode_ctx_enumerated(buf, 24, *v),
        S::NotifyType(v) => primitives::encode_ctx_enumerated(buf, 25, *v),
        S::ShedState(v) => primitives::encode_ctx_enumerated(buf, 27, *v),
        S::SilencedState(v) => primitives::encode_ctx_enumerated(buf, 28, *v),
        S::AccessEvent(v) => primitives::encode_ctx_enumerated(buf, 30, *v),
        S::ZoneOccupancyState(v) => primitives::encode_ctx_enumerated(buf, 31, *v),
        S::AccessCredentialDisableReason(v) => primitives::encode_ctx_enumerated(buf, 32, *v),
        S::AccessCredentialDisable(v) => primitives::encode_ctx_enumerated(buf, 33, *v),
        S::AuthenticationStatus(v) => primitives::encode_ctx_enumerated(buf, 34, *v),
        S::BackupState(v) => primitives::encode_ctx_enumerated(buf, 36, *v),
        S::WriteStatus(v) => primitives::encode_ctx_enumerated(buf, 37, *v),
        S::LightingInProgress(v) => primitives::encode_ctx_enumerated(buf, 38, *v),
        S::LightingOperation(v) => primitives::encode_ctx_enumerated(buf, 39, *v),
        S::LightingTransition(v) => primitives::encode_ctx_enumerated(buf, 40, *v),
        S::IntegerValue(v) => primitives::encode_ctx_signed(buf, 41, *v),
        S::BinaryLightingValue(v) => primitives::encode_ctx_enumerated(buf, 42, *v),
        S::TimerState(v) => primitives::encode_ctx_enumerated(buf, 43, *v),
        S::TimerTransition(v) => primitives::encode_ctx_enumerated(buf, 44, *v),
        S::BacnetIpMode(v) => primitives::encode_ctx_enumerated(buf, 45, *v),
        S::NetworkPortCommand(v) => primitives::encode_ctx_enumerated(buf, 46, *v),
        S::NetworkType(v) => primitives::encode_ctx_enumerated(buf, 47, *v),
        S::NetworkNumberQuality(v) => primitives::encode_ctx_enumerated(buf, 48, *v),
        S::EscalatorOperationDirection(v) => primitives::encode_ctx_enumerated(buf, 49, *v),
        S::EscalatorFault(v) => primitives::encode_ctx_enumerated(buf, 50, *v),
        S::EscalatorMode(v) => primitives::encode_ctx_enumerated(buf, 51, *v),
        S::LiftCarDirection(v) => primitives::encode_ctx_enumerated(buf, 52, *v),
        S::LiftCarDoorCommand(v) => primitives::encode_ctx_enumerated(buf, 53, *v),
        S::LiftCarDriveStatus(v) => primitives::encode_ctx_enumerated(buf, 54, *v),
        S::LiftCarMode(v) => primitives::encode_ctx_enumerated(buf, 55, *v),
        S::LiftGroupMode(v) => primitives::encode_ctx_enumerated(buf, 56, *v),
        S::LiftFault(v) => primitives::encode_ctx_enumerated(buf, 57, *v),
        S::ProtocolLevel(v) => primitives::encode_ctx_enumerated(buf, 58, *v),
        S::AuditLevel(v) => primitives::encode_ctx_enumerated(buf, 59, *v),
        S::AuditOperation(v) => primitives::encode_ctx_enumerated(buf, 60, *v),
        S::ExtendedValue(v) => primitives::encode_ctx_unsigned(buf, 63, v.encoded() as u64),
        S::Other(v) if v.is_constructed() => {
            tags::encode_opening_tag(buf, v.tag());
            buf.extend_from_slice(v.data());
            tags::encode_closing_tag(buf, v.tag());
        }
        S::Other(v) => primitives::encode_ctx_octet_string(buf, v.tag(), v.data()),
    }
    Ok(())
}

/// Decode one [`BACnetPropertyStates`] CHOICE element at `offset`.
///
/// Proprietary tags 64 through 254 retain their encoded contents in
/// [`BACnetPropertyStates::Other`]. Reserved standard tags are rejected.
pub fn decode_property_state(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetPropertyStates, usize), Error> {
    use BACnetPropertyStates as S;
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != TagClass::Context || tag.is_closing {
        return Err(Error::decoding(
            offset,
            "BACnetPropertyStates: expected a context tag",
        ));
    }
    if tag.is_opening {
        if !(64..=254).contains(&tag.number) {
            return Err(Error::decoding(
                offset,
                "BACnetPropertyStates: constructed form requires a proprietary tag",
            ));
        }
        let (content, end) = tags::extract_context_value(data, pos, tag.number)?;
        validate_tlv_sequence(content, "proprietary property-state body")?;
        return Ok((
            S::Other(BACnetProprietaryPropertyState::constructed(
                tag.number,
                content.to_vec(),
            )?),
            end,
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
            match content[0] {
                0 => S::BooleanValue(false),
                1 => S::BooleanValue(true),
                value => {
                    return Err(Error::decoding(
                        pos,
                        format!("BACnetPropertyStates boolean-value must be 0 or 1, got {value}"),
                    ));
                }
            }
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
        14 => S::RestartReason(unsigned()?),
        15 => S::DoorAlarmState(unsigned()?),
        16 => S::Action(unsigned()?),
        17 => S::DoorSecuredStatus(unsigned()?),
        18 => S::DoorStatus(unsigned()?),
        19 => S::DoorValue(unsigned()?),
        20 => S::FileAccessMethod(unsigned()?),
        21 => S::LockStatus(unsigned()?),
        22 => S::LifeSafetyOperation(unsigned()?),
        23 => S::Maintenance(unsigned()?),
        24 => S::NodeType(unsigned()?),
        25 => S::NotifyType(unsigned()?),
        27 => S::ShedState(unsigned()?),
        28 => S::SilencedState(unsigned()?),
        30 => S::AccessEvent(unsigned()?),
        31 => S::ZoneOccupancyState(unsigned()?),
        32 => S::AccessCredentialDisableReason(unsigned()?),
        33 => S::AccessCredentialDisable(unsigned()?),
        34 => S::AuthenticationStatus(unsigned()?),
        36 => S::BackupState(unsigned()?),
        37 => S::WriteStatus(unsigned()?),
        38 => S::LightingInProgress(unsigned()?),
        39 => S::LightingOperation(unsigned()?),
        40 => S::LightingTransition(unsigned()?),
        41 => S::IntegerValue(primitives::decode_signed_canonical(content)?),
        42 => S::BinaryLightingValue(unsigned()?),
        43 => S::TimerState(unsigned()?),
        44 => S::TimerTransition(unsigned()?),
        45 => S::BacnetIpMode(unsigned()?),
        46 => S::NetworkPortCommand(unsigned()?),
        47 => S::NetworkType(unsigned()?),
        48 => S::NetworkNumberQuality(unsigned()?),
        49 => S::EscalatorOperationDirection(unsigned()?),
        50 => S::EscalatorFault(unsigned()?),
        51 => S::EscalatorMode(unsigned()?),
        52 => S::LiftCarDirection(unsigned()?),
        53 => S::LiftCarDoorCommand(unsigned()?),
        54 => S::LiftCarDriveStatus(unsigned()?),
        55 => S::LiftCarMode(unsigned()?),
        56 => S::LiftGroupMode(unsigned()?),
        57 => S::LiftFault(unsigned()?),
        58 => S::ProtocolLevel(unsigned()?),
        59 => S::AuditLevel(unsigned()?),
        60 => S::AuditOperation(unsigned()?),
        63 => S::ExtendedValue(BACnetExtendedPropertyState::from_encoded(unsigned()?)?),
        other @ 64..=254 => S::Other(BACnetProprietaryPropertyState::primitive(
            other,
            content.to_vec(),
        )?),
        reserved => {
            return Err(Error::decoding(
                offset,
                format!("BACnetPropertyStates context tag {reserved} is reserved"),
            ));
        }
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

/// Validate a BACnet TLV sequence without normalizing its encoded values.
///
/// This checks matching context tags, the context nesting limit, and
/// application-value forms while preserving defined CharacterString encodings.
pub fn validate_tlv_sequence(data: &[u8], what: &str) -> Result<(), Error> {
    let mut offset = 0;
    let mut count = 0;
    while offset < data.len() {
        if count >= MAX_FRAMED_ITEMS {
            return Err(Error::decoding(
                offset,
                format!("{what}: sequence exceeds item limit"),
            ));
        }
        let (tag, content) = tags::decode_tag(data, offset)?;
        if tag.is_opening {
            let (inner, next) = tags::extract_context_value(data, content, tag.number)?;
            validate_tlv_sequence(inner, what)?;
            offset = next;
        } else if tag.class == TagClass::Application {
            offset = primitives::validate_application_value(data, offset)?;
        } else {
            let (_, next) = primitives::decode_application_value(data, offset)?;
            offset = next;
        }
        count += 1;
    }
    Ok(())
}

/// Validate the shared Extended Event/Fault `parameters` production.
/// Its only context-tagged CHOICE is `reference [0]` over
/// `BACnetDeviceObjectPropertyReference`; NotificationParameters uses a
/// different Extended production with `property-value [0]`.
pub(crate) fn validate_extended_parameters(data: &[u8], what: &str) -> Result<(), Error> {
    let mut offset = 0;
    let mut count = 0;
    while offset < data.len() {
        if count >= MAX_FRAMED_ITEMS {
            return Err(Error::decoding(
                offset,
                format!("{what}: parameters exceed item limit"),
            ));
        }
        let (tag, content) = tags::decode_tag(data, offset)?;
        if tag.class == TagClass::Context {
            if !tag.is_opening_tag(0) {
                return Err(Error::decoding(
                    offset,
                    format!("{what}: expected reference opening tag [0]"),
                ));
            }
            let (_, after_reference) = decode_dopr_body(data, content, what)?;
            offset = expect_closing(data, after_reference, 0, what)?;
        } else {
            offset = primitives::validate_application_value(data, offset)?;
        }
        count += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
