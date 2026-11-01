//! BACnetTimerStateChangeValue (Clause 21): one element of a Timer object's
//! State_Change_Values (Clause 12.57.20).
//!
//! An untagged CHOICE: the primitives carry their own application tags, and
//! the four other alternatives are told apart by context tag. `[0]` is a
//! primitive NULL with no contents; `[1]`, `[2]` and `[3]` each open and
//! close around their value.

use bacnet_types::constructed::BACnetTimerStateChangeValue;
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bytes::BytesMut;

use super::lighting_command;
use super::tagged::expect_closing;
use crate::primitives;
use crate::tags::{self, TagClass};

const WHAT: &str = "BACnetTimerStateChangeValue";

/// Append one BACnetTimerStateChangeValue to `buf`.
///
/// A [`BACnetTimerStateChangeValue::Primitive`] holding a value that isn't
/// primitive, or a CharacterString with no encoding, is [`Error::Encode`],
/// and so is an empty [`BACnetTimerStateChangeValue::Constructed`]: the
/// frame holds one value at least. Either way `buf` is left as it was.
pub fn encode_timer_state_change_value(
    buf: &mut BytesMut,
    value: &BACnetTimerStateChangeValue,
) -> Result<(), Error> {
    match value {
        BACnetTimerStateChangeValue::NoValue => tags::encode_tag(buf, 0, TagClass::Context, 0),
        BACnetTimerStateChangeValue::Primitive(value) => {
            if !value.is_primitive() {
                return Err(Error::encode("timer state change value is not primitive"));
            }
            let mut encoded = BytesMut::new();
            primitives::encode_property_value(&mut encoded, value)?;
            buf.extend_from_slice(&encoded);
        }
        BACnetTimerStateChangeValue::Constructed(octets) => {
            if octets.is_empty() {
                return Err(Error::encode("empty constructed timer state change value"));
            }
            tags::encode_opening_tag(buf, 1);
            buf.extend_from_slice(octets);
            tags::encode_closing_tag(buf, 1);
        }
        BACnetTimerStateChangeValue::DateTime(date, time) => {
            tags::encode_opening_tag(buf, 2);
            primitives::encode_app_date(buf, date);
            primitives::encode_app_time(buf, time);
            tags::encode_closing_tag(buf, 2);
        }
        BACnetTimerStateChangeValue::LightingCommand(command) => {
            tags::encode_opening_tag(buf, 3);
            lighting_command::encode_lighting_command(buf, command);
            tags::encode_closing_tag(buf, 3);
        }
    }
    Ok(())
}

/// Decode one BACnetTimerStateChangeValue at `offset`: the value and the
/// offset past it. Octets after it are the caller's, so an array's elements
/// can be read back to back.
///
/// Refused with [`Error::Decoding`]: a context tag `[0]` with contents, an
/// empty `[1]` frame, a `[2]` frame that isn't one Date and one Time, a `[3]`
/// frame that isn't one lighting command, a frame left open, or a tag that
/// opens no alternative, a context tag past `[3]` included. A lighting
/// command field too wide for its type is [`Error::Range`]. Contents that run
/// past the end of `data` are [`Error::BufferTooShort`].
pub fn decode_timer_state_change_value(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetTimerStateChangeValue, usize), Error> {
    if offset >= data.len() {
        return Err(Error::missing(offset, "missing timer state change value"));
    }
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class == TagClass::Application {
        let (value, end) = primitives::decode_application_value(data, offset)?;
        return Ok((BACnetTimerStateChangeValue::Primitive(value), end));
    }
    if tag.is_context(0) && !tag.is_opening && !tag.is_closing {
        if tag.length != 0 {
            return Err(Error::decoding(offset, "no-value [0] carries contents"));
        }
        return Ok((BACnetTimerStateChangeValue::NoValue, pos));
    }
    match (tag.is_opening, tag.number) {
        (true, 1) => {
            let (inner, end) = tags::extract_context_value(data, pos, 1)?;
            if inner.is_empty() {
                return Err(Error::decoding(offset, "empty constructed-value [1]"));
            }
            Ok((BACnetTimerStateChangeValue::Constructed(inner.to_vec()), end))
        }
        (true, 2) => {
            let ((date, time), end) = primitives::decode_framed_date_time(data, offset, 2, WHAT)?;
            Ok((BACnetTimerStateChangeValue::DateTime(date, time), end))
        }
        (true, 3) => {
            let (command, end) = lighting_command::decode_lighting_command(data, pos)?;
            let end = expect_closing(data, end, 3, WHAT)?;
            Ok((BACnetTimerStateChangeValue::LightingCommand(command), end))
        }
        _ => Err(DecodeError::new(
            DecodingKind::InvalidTag,
            offset,
            "expected an application-tagged primitive or context tag 0 to 3",
        )
        .within(WHAT)
        .into()),
    }
}
