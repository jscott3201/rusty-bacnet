//! Clause 21 codec for `BACnetLandingDoorStatus`, the value of each element of
//! the Lift object's Landing_Door_Status array (Clause 12.59).
//!
//! The SEQUENCE has one member, landing-doors `[0]`: a SEQUENCE OF framed by
//! opening and closing tag 0. Each landing door inside the frame is a pair of
//! primitive context tags, floor-number `[0]` (Unsigned8) then door-status
//! `[1]` (BACnetDoorStatus). A car door with no landing doors encodes as the
//! empty frame. `BACnetAssignedLandingCalls` has the same shape, so both use
//! the floor-pair helpers.

use bacnet_types::constructed::{BACnetLandingDoorStatus, LandingDoor};
use bacnet_types::enums::DoorStatus;
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::floor_pairs::{decode_floor_pairs, encode_floor_pairs, PairNames};

const NAMES: PairNames = PairNames {
    value: "landing door status",
    frame: "landing door status landing-doors",
    floor: "landing door floor-number",
    second: "landing door door-status",
    floor_oversized: "landing door status floor-number [0] exceeds an Unsigned8",
    second_oversized: "landing door status door-status [1] exceeds 32 bits",
};

/// Encode one `BACnetLandingDoorStatus` SEQUENCE.
pub fn encode_landing_door_status(buf: &mut BytesMut, value: &BACnetLandingDoorStatus) {
    encode_floor_pairs(
        buf,
        value
            .landing_doors
            .iter()
            .map(|door| (door.floor_number, door.door_status.to_raw())),
    );
}

/// Decode one `BACnetLandingDoorStatus` SEQUENCE at `offset`.
///
/// Returns the value and the offset just past its closing tag, so a caller
/// that expects exactly one value must check the returned offset reaches the
/// end of its data.
///
/// Two kinds of failure are kept apart, as in
/// [`decode_landing_call_status`](super::decode_landing_call_status), so a
/// property writer can answer with the matching Clause 15.9.1.3 error. A
/// malformed value (a missing frame or member, an empty or truncated member,
/// or more than 10,000 landing doors) fails with [`Error::Decoding`] or
/// [`Error::BufferTooShort`]. A value that is well formed throughout, but
/// where a floor-number exceeds an Unsigned8 or a door-status exceeds 32
/// bits, fails with [`Error::Range`] naming the first oversized member.
/// A malformed member anywhere in the value takes precedence over an
/// oversized one. Within 32 bits a door-status is kept as received, reserved
/// and proprietary values included, for the receiver to judge.
pub fn decode_landing_door_status(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetLandingDoorStatus, usize), Error> {
    let (pairs, end) = decode_floor_pairs(data, offset, &NAMES)?;
    let landing_doors = pairs
        .into_iter()
        .map(|(floor_number, status)| LandingDoor {
            floor_number,
            door_status: DoorStatus::from_raw(status),
        })
        .collect();
    Ok((BACnetLandingDoorStatus { landing_doors }, end))
}
