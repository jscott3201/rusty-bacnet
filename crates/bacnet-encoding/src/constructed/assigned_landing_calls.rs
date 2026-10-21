//! Clause 21 codec for `BACnetAssignedLandingCalls`, the value of each element
//! of the Lift object's Assigned_Landing_Calls array (Clause 12.59).
//!
//! The value is one framed member, landing-calls `[0]`, listing the calls
//! assigned to a car door. Inside the frame each call is the floor number
//! `[0]` (Unsigned8) followed by its direction `[1]` (BACnetLiftCarDirection),
//! both primitive context tags. A car door with no assigned calls encodes as
//! the empty frame. The shape matches `BACnetLandingDoorStatus`, so both use
//! the floor-pair helpers.

use bacnet_types::constructed::{AssignedLandingCall, BACnetAssignedLandingCalls};
use bacnet_types::enums::LiftCarDirection;
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::floor_pairs::{decode_floor_pairs, encode_floor_pairs, PairNames};

const NAMES: PairNames = PairNames {
    value: "assigned landing calls",
    frame: "assigned landing calls landing-calls",
    floor: "landing call floor-number",
    second: "landing call direction",
    floor_oversized: "assigned landing calls floor-number [0] exceeds an Unsigned8",
    second_oversized: "assigned landing calls direction [1] exceeds 32 bits",
};

/// Encode one `BACnetAssignedLandingCalls` SEQUENCE.
pub fn encode_assigned_landing_calls(buf: &mut BytesMut, value: &BACnetAssignedLandingCalls) {
    encode_floor_pairs(
        buf,
        value
            .landing_calls
            .iter()
            .map(|call| (call.floor_number, call.direction.to_raw())),
    );
}

/// Decode one `BACnetAssignedLandingCalls` SEQUENCE at `offset`.
///
/// Returns the value and the offset just past its closing tag, so a caller
/// that expects exactly one value must check the returned offset reaches the
/// end of its data.
///
/// Failures split as in
/// [`decode_landing_door_status`](super::decode_landing_door_status): a
/// malformed value (a missing frame or member, an empty or truncated member,
/// or more than 10,000 calls) fails with [`Error::Decoding`] or
/// [`Error::BufferTooShort`], and a well-formed value whose floor-number
/// exceeds an Unsigned8 or whose direction exceeds 32 bits fails with
/// [`Error::Range`] naming the first oversized member. Within 32 bits a
/// direction is kept as received, reserved and proprietary values included,
/// for the receiver to judge.
pub fn decode_assigned_landing_calls(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetAssignedLandingCalls, usize), Error> {
    let (pairs, end) = decode_floor_pairs(data, offset, &NAMES)?;
    let landing_calls = pairs
        .into_iter()
        .map(|(floor_number, direction)| AssignedLandingCall {
            floor_number,
            direction: LiftCarDirection::from_raw(direction),
        })
        .collect();
    Ok((BACnetAssignedLandingCalls { landing_calls }, end))
}
