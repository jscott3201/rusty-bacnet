//! `BACnetActionList` and `BACnetActionCommand` (ASHRAE 135-2020 Clause 21),
//! the element of a Command object's Action array (Clause 12.10.8).
//!
//! An action list frames its commands in an opening/closing context tag `[0]`
//! pair and puts them back to back inside it. A command is nine context-tagged
//! members numbered in order: device `[0]` (optional), object `[1]`, property
//! `[2]`, array index `[3]` (optional), the value framed in `[4]`, priority
//! `[5]` and post delay `[6]` (both optional), then the quit-on-failure `[7]`
//! and write-successful `[8]` BOOLEANs. An Action array read whole
//! concatenates its lists with no further frame.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::constructed::{BACnetActionCommand, BACnetActionList};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::{DecodeError, DecodingKind, Error, RangeError};
use bytes::BytesMut;

use super::tagged::{
    decode_ctx_boolean, decode_ctx_object_id, decode_ctx_unsigned, decode_framed_value,
    decode_optional_ctx, expect_opening,
};
use super::MAX_FRAMED_ITEMS;
use crate::{primitives, tags};

const WHAT: &str = "BACnetActionCommand";

/// Encode one bare `BACnetActionCommand`, appending to `buf`.
///
/// Fails, leaving `buf` as it was, when the priority is outside 1..=16 or the
/// value can't be encoded (see [`primitives::encode_property_value`]).
pub fn encode_action_command(
    buf: &mut BytesMut,
    command: &BACnetActionCommand,
) -> Result<(), Error> {
    if let Some(priority) = command.priority
        && !(1..=16).contains(&priority) {
            return Err(
                RangeError::new("BACnetActionCommand: priority {} is outside 1..=16")
                    .with_value(priority.into())
                    .into(),
            );
        }
    let start = buf.len();
    if let Some(device) = &command.device_identifier {
        primitives::encode_ctx_object_id(buf, 0, device);
    }
    primitives::encode_ctx_object_id(buf, 1, &command.object_identifier);
    primitives::encode_ctx_enumerated(buf, 2, command.property_identifier.to_raw());
    if let Some(index) = command.property_array_index {
        primitives::encode_ctx_unsigned(buf, 3, u64::from(index));
    }
    tags::encode_opening_tag(buf, 4);
    if let Err(error) = primitives::encode_property_value(buf, &command.property_value) {
        buf.truncate(start);
        return Err(error);
    }
    tags::encode_closing_tag(buf, 4);
    if let Some(priority) = command.priority {
        primitives::encode_ctx_unsigned(buf, 5, u64::from(priority));
    }
    if let Some(delay) = command.post_delay {
        primitives::encode_ctx_unsigned(buf, 6, u64::from(delay));
    }
    primitives::encode_ctx_boolean(buf, 7, command.quit_on_failure);
    primitives::encode_ctx_boolean(buf, 8, command.write_successful);
    Ok(())
}

/// Decode one bare `BACnetActionCommand` at `offset`; returns it and the
/// offset past its write-successful flag.
///
/// The value inside `[4]` decodes as one application value when it holds one
/// element and as a [`PropertyValue::List`] otherwise (empty included). A
/// context-tagged element in it decodes to [`PropertyValue::ApplicationData`],
/// so the value re-encodes to the same octets.
///
/// [`PropertyValue::List`]: bacnet_types::primitives::PropertyValue::List
/// [`PropertyValue::ApplicationData`]: bacnet_types::primitives::PropertyValue::ApplicationData
pub fn decode_action_command(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetActionCommand, usize), Error> {
    let (device_identifier, offset) =
        decode_optional_ctx(data, offset, 0, WHAT, decode_ctx_object_id)?;
    let (object_identifier, offset) = decode_ctx_object_id(data, offset, 1, WHAT)?;
    let (property, offset) = decode_ctx_unsigned::<u32>(data, offset, 2, WHAT)?;
    let property_identifier = PropertyIdentifier::from_raw(property);
    let (property_array_index, offset) =
        decode_optional_ctx(data, offset, 3, WHAT, decode_ctx_unsigned::<u32>)?;

    let content = expect_opening(data, offset, 4, WHAT)?;
    let (property_value, offset) = decode_framed_value(data, content, 4, WHAT)?;

    let (priority, next) = decode_optional_ctx(data, offset, 5, WHAT, decode_ctx_unsigned::<u32>)?;
    let priority = priority
        .map(|value| {
            u8::try_from(value)
                .ok()
                .filter(|p| (1..=16).contains(p))
                .ok_or_else(|| {
                    DecodeError::new(
                        DecodingKind::InvalidEncoding,
                        offset,
                        "priority {} is outside 1..=16",
                    )
                    .within(WHAT)
                    .with_value(value.into())
                })
        })
        .transpose()?;
    let (post_delay, offset) =
        decode_optional_ctx(data, next, 6, WHAT, decode_ctx_unsigned::<u32>)?;
    let (quit_on_failure, offset) = decode_ctx_boolean(data, offset, 7, WHAT)?;
    let (write_successful, offset) = decode_ctx_boolean(data, offset, 8, WHAT)?;
    Ok((
        BACnetActionCommand {
            device_identifier,
            object_identifier,
            property_identifier,
            property_array_index,
            property_value,
            priority,
            post_delay,
            quit_on_failure,
            write_successful,
        },
        offset,
    ))
}

/// Encode one `BACnetActionList`, its `[0]` frame included, appending to
/// `buf`. Fails, leaving `buf` as it was, when a command fails to encode.
pub fn encode_action_list(buf: &mut BytesMut, list: &BACnetActionList) -> Result<(), Error> {
    let start = buf.len();
    tags::encode_opening_tag(buf, 0);
    for command in &list.commands {
        if let Err(error) = encode_action_command(buf, command) {
            buf.truncate(start);
            return Err(error);
        }
    }
    tags::encode_closing_tag(buf, 0);
    Ok(())
}

/// Decode one `BACnetActionList` at `offset`; returns it and the offset past
/// its closing `[0]` tag, so an Action array is walked by calling this at
/// each element's start.
pub fn decode_action_list(data: &[u8], offset: usize) -> Result<(BACnetActionList, usize), Error> {
    let content = expect_opening(data, offset, 0, "BACnetActionList")?;
    let (inner, end) = tags::extract_context_value(data, content, 0)?;
    let body = &data[..content + inner.len()];
    let mut commands = Vec::new();
    let mut offset = content;
    while offset < body.len() {
        if commands.len() >= MAX_FRAMED_ITEMS {
            return Err(Error::overflow(
                offset,
                "BACnetActionList: commands exceed item limit",
            ));
        }
        let (command, next) = decode_action_command(body, offset)?;
        commands.push(command);
        offset = next;
    }
    Ok((BACnetActionList { commands }, end))
}
