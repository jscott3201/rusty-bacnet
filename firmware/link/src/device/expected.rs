//! What the device scenario reports in a good run. `tests/device.rs`
//! compares the native run's summary with it, and the emulation build of
//! the `device` image the emulated CPU's, so the two can't drift apart.
//!
//! Each answer's NPDU is written out here by hand from the Clause 6.2 and
//! 20.1 layouts and the application tags of Clause 20.2, not by the
//! encoders the device runs; its hash is taken over these octets.

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::MasterState;
use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};

use super::{CLIENT, LINK_FRAMES, Pdu, REPLIES, Reply, Summary};
use crate::tally::{FNV_OFFSET, fnv1a};

/// The I-Am, to every network (control 0x20: DNET 0xFFFF, DLEN 0, hop
/// count 255): Unconfirmed-Request 0x10, I-Am 0x00, then Device 1234
/// (0xC4 and 0x020004D2), Unsigned 480 (0x22 0x01E0), Enumerated 3, no
/// segmentation (0x91 0x03), and Unsigned 0, the vendor (0x21 0x00).
const I_AM: &[u8] = &[
    0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x00, 0xC4, 0x02, 0x00, 0x04, 0xD2, 0x22, 0x01, 0xE0,
    0x91, 0x03, 0x21, 0x00,
];

/// The answers to the thirteen requests, in order. A local NPDU is 0x01
/// 0x00. A ComplexACK starts 0x30, the invoke ID, ReadProperty 0x0C, then
/// the object identifier (context tag 0), the property (context tag 1),
/// any array index (context tag 2), and the value between the opening and
/// closing context tag 3 (0x3E, 0x3F). An Error is 0x50, the invoke ID,
/// 0x0C and the class and code as Enumerated values (0x91); a Reject 0x60,
/// the invoke ID and the reason; a server's Abort 0x71, the invoke ID and
/// the reason.
const ANSWERS: [&[u8]; 13] = [
    // 1: Object_Name, a CharacterString of 13 octets (0x75 0x0D): UTF-8
    // (0x00) and "rusty-device".
    &[
        0x01, 0x00, 0x30, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x4D, 0x3E, 0x75, 0x0D,
        0x00, b'r', b'u', b's', b't', b'y', b'-', b'd', b'e', b'v', b'i', b'c', b'e', 0x3F,
    ],
    // 2: Object_Identifier of Device 1234, for the wildcard instance asked.
    &[
        0x01, 0x00, 0x30, 0x02, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x4B, 0x3E, 0xC4, 0x02,
        0x00, 0x04, 0xD2, 0x3F,
    ],
    // 3: Object_List's length at index 0, Unsigned 2.
    &[
        0x01, 0x00, 0x30, 0x03, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x4C, 0x29, 0x00, 0x3E,
        0x21, 0x02, 0x3F,
    ],
    // 4: Object_List's second element, Analog Input 1.
    &[
        0x01, 0x00, 0x30, 0x04, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x4C, 0x29, 0x02, 0x3E,
        0xC4, 0x00, 0x00, 0x00, 0x01, 0x3F,
    ],
    // 5: PROPERTY (2), INVALID_ARRAY_INDEX (42).
    &[0x01, 0x00, 0x50, 0x05, 0x0C, 0x91, 0x02, 0x91, 0x2A],
    // 6: Present_Value, the Real 72.5 (0x44, 0x42910000).
    &[
        0x01, 0x00, 0x30, 0x06, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x3E, 0x44, 0x42,
        0x91, 0x00, 0x00, 0x3F,
    ],
    // 7: PROPERTY (2), PROPERTY_IS_NOT_AN_ARRAY (50).
    &[0x01, 0x00, 0x50, 0x07, 0x0C, 0x91, 0x02, 0x91, 0x32],
    // 8: OBJECT (1), UNKNOWN_OBJECT (31).
    &[0x01, 0x00, 0x50, 0x08, 0x0C, 0x91, 0x01, 0x91, 0x1F],
    // 9: PROPERTY (2), UNKNOWN_PROPERTY (32).
    &[0x01, 0x00, 0x50, 0x09, 0x0C, 0x91, 0x02, 0x91, 0x20],
    // 10: Reject, MISSING_REQUIRED_PARAMETER (5).
    &[0x01, 0x00, 0x60, 0x0A, 0x05],
    // 11: Reject, UNRECOGNIZED_SERVICE (9).
    &[0x01, 0x00, 0x60, 0x0B, 0x09],
    // 12: Abort, SEGMENTATION_NOT_SUPPORTED (4).
    &[0x01, 0x00, 0x71, 0x0C, 0x04],
    // 13: back through the router to station 7 on network 5 (control 0x20:
    // DNET 5, DLEN 1, DADR 7, hop count 255): Protocol_Services_Supported,
    // a BIT STRING of 8 octets (0x85 0x08), 7 bits unused, with bits 12
    // (readProperty) and 34 (who-Is) set.
    &[
        0x01, 0x20, 0x00, 0x05, 0x01, 0x07, 0xFF, 0x30, 0x0D, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2,
        0x19, 0x61, 0x3E, 0x85, 0x08, 0x07, 0x00, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x3F,
    ],
];

/// What the client reads in each answer.
const PDUS: [Pdu; 13] = [
    Pdu::ComplexAck(1),
    Pdu::ComplexAck(2),
    Pdu::ComplexAck(3),
    Pdu::ComplexAck(4),
    Pdu::Error(5, ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX),
    Pdu::ComplexAck(6),
    Pdu::Error(7, ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
    Pdu::Error(8, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
    Pdu::Error(9, ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
    Pdu::Reject(10, RejectReason::MISSING_REQUIRED_PARAMETER),
    Pdu::Reject(11, RejectReason::UNRECOGNIZED_SERVICE),
    Pdu::Abort(12, AbortReason::SEGMENTATION_NOT_SUPPORTED),
    Pdu::ComplexAck(13),
];

/// One answer as the summary records it.
fn reply(frame_type: FrameType, destination: u8, pdu: Pdu, npdu: &[u8]) -> Option<Reply> {
    Some(Reply {
        frame_type,
        destination,
        pdu,
        length: u8::try_from(npdu.len()).unwrap_or(u8::MAX),
        fnv: fnv1a(FNV_OFFSET, npdu),
    })
}

/// The summary [`run`](super::run) returns in a good run.
pub fn expected() -> Summary {
    let mut replies = [None; REPLIES];
    // The I-Am waits for the token, then goes out as a broadcast frame.
    replies[0] = reply(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        0xFF,
        Pdu::IAm,
        I_AM,
    );
    // Each answer goes back to the client in the request's reply window.
    for ((slot, npdu), pdu) in replies[1..].iter_mut().zip(ANSWERS).zip(PDUS) {
        *slot = reply(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            CLIENT,
            pdu,
            npdu,
        );
    }
    let mut link_frames = [None; LINK_FRAMES];
    // After the I-Am, a poll for the next master, which is the client, then
    // the token to it.
    link_frames[0] = Some((FrameType::POLL_FOR_MASTER, CLIENT));
    link_frames[1] = Some((FrameType::TOKEN, CLIENT));
    Summary {
        replies,
        link_frames,
        // Two Who-Is, thirteen requests and the two confirmed requests by
        // broadcast; the Who-Is for devices 5000 to 6000 and the two
        // broadcasts are left unanswered.
        delivered: 17,
        unanswered: 3,
        state: MasterState::Idle,
        // Two Who-Is, the token, the reply to the poll, the requests and the
        // two broadcasts.
        frames_received: 19,
        // The I-Am, the poll, the token and the thirteen answers.
        frames_transmitted: 16,
        token_rotations: 1,
        parser_errors: 0,
        errors: 0,
    }
}
