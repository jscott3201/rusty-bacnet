//! The framed list of floor pairs that `BACnetLandingDoorStatus` and
//! `BACnetAssignedLandingCalls` share (Clause 21).
//!
//! Each of the two productions holds one member, context tag 0, which frames
//! a list with opening and closing tag 0. Every entry of that list is two
//! primitive context tags in a fixed order: the universal floor number `[0]`
//! (Unsigned8), then a BACnet enumeration `[1]` (a door status, or a car
//! direction). An empty list encodes as the bare frame.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::error::{DecodeError, DecodingKind, Error, RangeError};
use bytes::BytesMut;

use super::members::{narrow, unsigned_member};
use super::tagged::{decode_ctx_primitive, expect_closing, expect_opening, next_is_closing};
use super::MAX_FRAMED_ITEMS;
use crate::primitives;
use crate::tags;

/// How one production names its parts in decode errors.
pub(super) struct PairNames {
    /// The production, e.g. "landing door status".
    pub(super) value: &'static str,
    /// The framed list member, e.g. "landing door status landing-doors".
    pub(super) frame: &'static str,
    /// An entry's floor-number member, e.g. "landing door floor-number".
    pub(super) floor: &'static str,
    /// An entry's enumerated member, e.g. "landing door door-status".
    pub(super) second: &'static str,
    /// The range error for an oversized floor number, e.g. "landing door
    /// status floor-number \[0\] exceeds an Unsigned8".
    pub(super) floor_oversized: &'static str,
    /// The range error for an oversized enumerated member, e.g. "landing
    /// door status door-status \[1\] exceeds 32 bits".
    pub(super) second_oversized: &'static str,
}

/// Encode the frame around `pairs`, each a floor number and the raw value of
/// its enumerated member.
pub(super) fn encode_floor_pairs(buf: &mut BytesMut, pairs: impl IntoIterator<Item = (u8, u32)>) {
    tags::encode_opening_tag(buf, 0);
    for (floor, value) in pairs {
        primitives::encode_ctx_unsigned(buf, 0, floor.into());
        primitives::encode_ctx_enumerated(buf, 1, value);
    }
    tags::encode_closing_tag(buf, 0);
}

/// Decode the frame at `offset` into its floor number and enumerated value
/// pairs, and return them with the offset just past the closing tag.
///
/// Two kinds of failure are kept apart, so a property writer can answer with
/// the matching Clause 15.9.1.3 error. A malformed value (a missing frame or
/// member, an empty or truncated member, or more than 10,000 entries) fails
/// with [`Error::Decoding`] or [`Error::BufferTooShort`]. A value that is
/// well formed throughout, but where a floor number exceeds an Unsigned8 or
/// an enumerated member exceeds 32 bits, fails with [`Error::Range`]
/// naming the first oversized member. A malformed member anywhere in the
/// value takes precedence over an oversized one.
pub(super) fn decode_floor_pairs(
    data: &[u8],
    offset: usize,
    names: &PairNames,
) -> Result<(Vec<(u8, u32)>, usize), Error> {
    let mut oversized = None;
    let mut offset = expect_opening(data, offset, 0, names.frame)?;
    let mut pairs = Vec::new();
    loop {
        if next_is_closing(data, offset, 0)? {
            if let Some(member) = oversized {
                return Err(RangeError::new(member).into());
            }
            return Ok((pairs, expect_closing(data, offset, 0, names.frame)?));
        }
        if pairs.len() >= MAX_FRAMED_ITEMS {
            return Err(DecodeError::new(
                DecodingKind::Overflow,
                offset,
                "exceeds the decoded item limit",
            )
            .within(names.value)
            .into());
        }
        let (floor, next) = decode_ctx_primitive(data, offset, 0, names.floor)?;
        let floor_number = narrow(
            unsigned_member(floor, next - floor.len())?,
            names.floor_oversized,
            &mut oversized,
        );

        let (raw, next) = decode_ctx_primitive(data, next, 1, names.second)?;
        let value: u32 = narrow(
            unsigned_member(raw, next - raw.len())?,
            names.second_oversized,
            &mut oversized,
        );

        pairs.push((floor_number, value));
        offset = next;
    }
}
