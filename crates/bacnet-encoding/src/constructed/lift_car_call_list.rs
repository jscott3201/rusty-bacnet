//! Clause 21 codec for `BACnetLiftCarCallList`, the value of each element of
//! the Lift object's Registered_Car_Call array (Clause 12.59).
//!
//! The value is one framed member, floor-numbers `[0]`, between opening and
//! closing tag 0. The frame holds the registered floors as plain
//! application-tagged Unsigned values, each within an Unsigned8, with no
//! context tag of their own. A car door with no registered calls encodes as
//! the empty frame.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::constructed::BACnetLiftCarCallList;
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::members::{narrow, unsigned_member};
use super::tagged::{contents, expect_opening};
use super::MAX_FRAMED_ITEMS;
use crate::primitives;
use crate::tags::{self, app_tag, TagClass};

/// Encode one `BACnetLiftCarCallList` SEQUENCE.
pub fn encode_lift_car_call_list(buf: &mut BytesMut, value: &BACnetLiftCarCallList) {
    tags::encode_opening_tag(buf, 0);
    for &floor in &value.floor_numbers {
        primitives::encode_app_unsigned(buf, floor.into());
    }
    tags::encode_closing_tag(buf, 0);
}

/// Decode one `BACnetLiftCarCallList` SEQUENCE at `offset`.
///
/// Returns the value and the offset just past its closing tag, so a caller
/// that expects exactly one value must check the returned offset reaches the
/// end of its data.
///
/// Failures split as in
/// [`decode_landing_door_status`](super::decode_landing_door_status): a
/// malformed value (a missing frame, an entry that isn't an application
/// Unsigned, an empty or truncated entry, or more than 10,000 floors) fails
/// with [`Error::Decoding`] or [`Error::BufferTooShort`], and a well-formed
/// value with a floor number above an Unsigned8 fails with
/// [`Error::OutOfRange`]. A malformed entry anywhere in the value takes
/// precedence over an oversized one.
pub fn decode_lift_car_call_list(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetLiftCarCallList, usize), Error> {
    let mut oversized = None;
    let mut offset = expect_opening(data, offset, 0, "lift car call list floor-numbers")?;
    let mut floor_numbers = Vec::new();
    loop {
        let (tag, content) = tags::decode_tag(data, offset)?;
        if tag.is_closing_tag(0) {
            if let Some(member) = oversized {
                return Err(Error::OutOfRange(format!("lift car call list {member}")));
            }
            return Ok((BACnetLiftCarCallList { floor_numbers }, content));
        }
        if floor_numbers.len() >= MAX_FRAMED_ITEMS {
            return Err(Error::overflow(
                offset,
                "lift car call list exceeds the decoded item limit",
            ));
        }
        if tag.class != TagClass::Application || tag.number != app_tag::UNSIGNED {
            return Err(Error::decoding(
                offset,
                "lift car call list floor number must be an application Unsigned",
            ));
        }
        let (floor, next) = contents(data, content, tag.length)?;
        floor_numbers.push(narrow(
            unsigned_member(floor, content)?,
            "floor number exceeds an Unsigned8",
            &mut oversized,
        ));
        offset = next;
    }
}
