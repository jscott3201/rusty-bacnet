//! The codec part: encode a short stream of frames and decode it again.

use bacnet_mstp_codec::{DecodeEvent, Frame, FrameType, StreamDecoder};

use super::{Summary, WHO_IS};

/// Test data for a Test_Request.
const TEST_DATA: [u8; 3] = [0xA5, 0x5A, 0x00];

/// The frames on the line, in order.
const FRAMES: [Frame<'static>; 5] = [
    Frame::new(FrameType::TOKEN, 0x02, 0x01, &[]),
    Frame::new(FrameType::POLL_FOR_MASTER, 0x03, 0x01, &[]),
    Frame::new(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        0xFF,
        0x02,
        &WHO_IS,
    ),
    Frame::new(FrameType::TEST_REQUEST, 0x01, 0x02, &TEST_DATA),
    Frame::new(FrameType::REPLY_TO_POLL_FOR_MASTER, 0x01, 0x03, &[]),
];

/// Index of the frame sent a second time with a corrupted data octet.
const CORRUPTED: usize = 2;
/// Line noise ahead of the first frame.
const NOISE: [u8; 2] = [0x00, 0xFF];
/// Octets pushed into the decoder at a time, as a UART FIFO might hand them
/// over.
const CHUNK: usize = 5;

/// Encode the stream into `wire`, then decode it through `storage`.
///
/// The stream is: noise, then each frame of [`FRAMES`], with a copy of the
/// [`CORRUPTED`] one whose data CRC fails just before the original.
pub(crate) fn run(storage: &mut [u8], wire: &mut [u8], summary: &mut Summary) {
    // A Token never carries data, so the encoder refuses this one.
    if Frame::new(FrameType::TOKEN, 0x02, 0x01, &WHO_IS)
        .encode_into(wire)
        .is_err()
    {
        summary.encode_refusals += 1;
    }
    let Some(length) = encode_stream(wire, summary) else {
        summary.error();
        return;
    };
    let Some(stream) = wire.get(..length) else {
        summary.error();
        return;
    };
    decode_stream(storage, stream, summary);
}

/// Write the stream to `wire` and return its length.
fn encode_stream(wire: &mut [u8], summary: &mut Summary) -> Option<usize> {
    let mut length = NOISE.len();
    wire.get_mut(..length)?.copy_from_slice(&NOISE);
    for (index, frame) in FRAMES.iter().enumerate() {
        if index == CORRUPTED {
            let start = length;
            length += frame.encode_into(wire.get_mut(start..)?).ok()?;
            // The first data octet, just after the preamble and header.
            let octet = wire.get_mut(start + 8)?;
            *octet ^= 0x01;
        }
        length += frame.encode_into(wire.get_mut(length..)?).ok()?;
        summary.frames_encoded += 1;
    }
    Some(length)
}

/// Push `stream` through a decoder over `storage`, a few octets at a time,
/// and tally what comes out.
fn decode_stream(storage: &mut [u8], stream: &[u8], summary: &mut Summary) {
    let mut decoder = StreamDecoder::new(storage);
    let mut expected = FRAMES.iter();
    for chunk in stream.chunks(CHUNK) {
        let mut input = chunk;
        // Each push reports one event; push nothing until the decoder needs
        // more, so every frame and discard in the chunk is seen.
        loop {
            match decoder.push_slice(input) {
                DecodeEvent::NeedMore => break,
                DecodeEvent::Frame { frame, .. } => {
                    if expected.next() == Some(&frame) {
                        summary.frames_decoded += 1;
                    } else {
                        summary.error();
                    }
                }
                DecodeEvent::Invalid { discard } => {
                    summary.noise_octets = summary
                        .noise_octets
                        .saturating_add(u8::try_from(discard).unwrap_or(u8::MAX));
                }
                DecodeEvent::Malformed { .. } => summary.frames_malformed += 1,
                DecodeEvent::Error(_) => summary.error(),
            }
            input = &[];
        }
    }
}
