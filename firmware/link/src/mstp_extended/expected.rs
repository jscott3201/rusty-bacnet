//! What the mstp-extended scenario reports in a good run.
//! `tests/mstp_extended.rs` compares the native run's summary with it, and
//! the emulation build of the `mstp-extended` image the emulated CPU's, so
//! the two can't drift apart.

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::MasterState;

use super::{REPLY_LENGTH, Summary};

/// The peer station.
const PEER: u8 = 2;
/// This station.
const STATION: u8 = 1;

/// The summary [`run`](super::run) returns in a good run.
pub const EXPECTED: Summary = Summary {
    // With extended frames on.
    max_npdu: 1497,
    // With the token: the request, too long for a standard frame, in a Type
    // 32 frame of 526 octets (the preamble and header, 512 octets of data
    // and one of COBS overhead, and the encoded CRC-32K); then a poll for
    // the next master, which is the peer, and the token to it.
    sent: [
        Some((FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY, PEER, 526)),
        Some((FrameType::POLL_FOR_MASTER, PEER, 8)),
        Some((FrameType::TOKEN, PEER, 8)),
        None,
    ],
    request_intact: true,
    // The peer's reply, from its Type 33 frame.
    delivered: Some((PEER, STATION, REPLY_LENGTH as u16)),
    reply_intact: true,
    requests: 0,
    // Transmit and back to receive for each of the three frames sent.
    direction_changes: 6,
    // Watching the peer take up the token it was passed.
    state: MasterState::PassToken,
    // The token, the reply and the answer to the poll.
    frames_received: 3,
    frames_transmitted: 3,
    token_rotations: 1,
    parser_errors: 0,
    errors: 0,
};
