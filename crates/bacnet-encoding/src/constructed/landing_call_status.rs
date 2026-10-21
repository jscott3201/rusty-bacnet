//! Clause 21 codec for `BACnetLandingCallStatus`, the value of the Elevator
//! Group object's Landing_Call_Control and of each Landing_Calls element
//! (Clause 12.58).
//!
//! The SEQUENCE has three primitive context-tagged members: floor-number
//! `[0]`, then the untagged command CHOICE, whose alternatives carry their
//! own tags (direction `[1]` or destination `[2]`), then an optional
//! floor-text `[3]`.
//! There is no opening/closing frame, and a BACnetLIST of these values is the
//! plain concatenation of its elements.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::constructed::{BACnetLandingCallStatus, LandingCallCommand};
use bacnet_types::enums::LiftCarDirection;
use bacnet_types::error::{Error, RangeError};
use bytes::BytesMut;

use super::members::{narrow, unsigned_member};
use super::tagged::{
    contents, decode_ctx_character_string, decode_ctx_primitive, decode_optional_ctx,
};
use crate::primitives;
use crate::tags;

/// Encode one unframed `BACnetLandingCallStatus` SEQUENCE.
///
/// Fails, leaving `buf` untouched, only when `floor_text` is too long to
/// encode.
pub fn encode_landing_call_status(
    buf: &mut BytesMut,
    value: &BACnetLandingCallStatus,
) -> Result<(), Error> {
    let mut encoded = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut encoded, 0, value.floor_number.into());
    match value.command {
        LandingCallCommand::Direction(direction) => {
            primitives::encode_ctx_enumerated(&mut encoded, 1, direction.to_raw())
        }
        LandingCallCommand::Destination(floor) => {
            primitives::encode_ctx_unsigned(&mut encoded, 2, floor.into())
        }
    }
    if let Some(text) = &value.floor_text {
        primitives::encode_ctx_character_string(&mut encoded, 3, text)?;
    }
    buf.extend_from_slice(&encoded);
    Ok(())
}

/// Encode a BACnetLIST of `BACnetLandingCallStatus` as concatenated elements.
///
/// Fails, leaving `buf` untouched, when any element fails to encode.
pub fn encode_landing_call_status_list(
    buf: &mut BytesMut,
    values: &[BACnetLandingCallStatus],
) -> Result<(), Error> {
    let mut encoded = BytesMut::new();
    for value in values {
        encode_landing_call_status(&mut encoded, value)?;
    }
    buf.extend_from_slice(&encoded);
    Ok(())
}

/// Decode one unframed `BACnetLandingCallStatus` SEQUENCE at `offset`.
///
/// Returns the value and the offset just past it. Decoding stops after the
/// command member, or after floor-text `[3]` when that follows, so a caller
/// that expects exactly one value must check the returned offset reaches the
/// end of its data.
///
/// Two kinds of failure are kept apart, so a property writer can answer with
/// the matching Clause 15.9.1.3 error. A malformed value (a missing, empty,
/// truncated or misplaced member, or an undecodable floor-text) fails with
/// [`Error::Decoding`] or [`Error::BufferTooShort`]. A value whose members are
/// all well formed, but where floor-number or destination exceeds an
/// Unsigned8 or the direction exceeds 32 bits, fails with
/// [`Error::Range`] naming the first oversized member. A malformed member
/// anywhere in the value takes precedence over an oversized one. Within 32
/// bits the direction is kept as received, reserved and proprietary values
/// included, for the receiver to judge.
pub fn decode_landing_call_status(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetLandingCallStatus, usize), Error> {
    let mut oversized = None;

    let (floor, offset) =
        decode_ctx_primitive(data, offset, 0, "landing call status floor-number")?;
    let floor_number = narrow(
        unsigned_member(floor, offset - floor.len())?,
        "landing call status floor-number [0] exceeds an Unsigned8",
        &mut oversized,
    );

    let (tag, content) = tags::decode_tag(data, offset)?;
    let (command, offset) = if tag.is_context(1) {
        let (raw, end) = contents(data, content, tag.length)?;
        let direction: u32 = narrow(
            unsigned_member(raw, content)?,
            "landing call status direction [1] exceeds 32 bits",
            &mut oversized,
        );
        (
            LandingCallCommand::Direction(LiftCarDirection::from_raw(direction)),
            end,
        )
    } else if tag.is_context(2) {
        let (raw, end) = contents(data, content, tag.length)?;
        let destination = narrow(
            unsigned_member(raw, content)?,
            "landing call status destination [2] exceeds an Unsigned8",
            &mut oversized,
        );
        (LandingCallCommand::Destination(destination), end)
    } else {
        return Err(Error::decoding(
            offset,
            "landing call status requires direction [1] or destination [2]",
        ));
    };

    let (floor_text, offset) = decode_optional_ctx(
        data,
        offset,
        3,
        "landing call status floor-text",
        decode_ctx_character_string,
    )?;

    if let Some(member) = oversized {
        return Err(RangeError::new(member).into());
    }
    Ok((
        BACnetLandingCallStatus {
            floor_number,
            command,
            floor_text,
        },
        offset,
    ))
}

/// Decode a complete BACnetLIST of `BACnetLandingCallStatus`.
///
/// Every byte must belong to an element; an empty input is an empty list.
pub fn decode_landing_call_status_list(data: &[u8]) -> Result<Vec<BACnetLandingCallStatus>, Error> {
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let (value, end) = decode_landing_call_status(data, offset)?;
        values.push(value);
        offset = end;
    }
    Ok(values)
}
