//! What the MS/TP scenario reports in a good run. `tests/mstp.rs` compares
//! the native run's summary with it, and the emulation build of the `mstp`
//! image the emulated CPU's, so the two can't drift apart.

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::MasterState;

use super::Summary;

/// The peer station the scenario exchanges the token with.
const PEER: u8 = 2;

/// The summary [`run`](super::run) returns in a good run.
pub const EXPECTED: Summary = Summary {
    // The five frames of the codec part (a Who-Is among them) go through
    // the stream decoder; the encoder refuses a Token with data.
    frames_encoded: 5,
    encode_refusals: 1,
    frames_decoded: 5,
    // The Who-Is copy with a bad data CRC, and two octets of noise.
    frames_malformed: 1,
    noise_octets: 2,
    // Transmit and back to receive for each of the four frames sent.
    direction_changes: 8,
    // With the token: the queued Who-Is to the peer, then a poll for the
    // next master, which is the peer. The peer answers, so the token goes to
    // it. Then the answer to the peer's request.
    sent: [
        Some((FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, PEER)),
        Some((FrameType::POLL_FOR_MASTER, PEER)),
        Some((FrameType::TOKEN, PEER)),
        Some((FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, PEER)),
        None,
        None,
        None,
        None,
    ],
    // The peer's broadcast and its request, then its Test_Response.
    npdus_delivered: 2,
    frames_delivered: 1,
    requests: 1,
    state: MasterState::Idle,
    // The token, the peer's answer to the poll, its broadcast, its
    // Test_Response and its request.
    frames_received: 5,
    frames_transmitted: 4,
    token_rotations: 1,
    parser_errors: 0,
    // Every unicast slot fills, the next NPDU is refused, and so is one a
    // standard frame can't carry.
    queued_before_full: 4,
    drops_full: 1,
    drops_oversize: 1,
    errors: 0,
};
