//! Integration tests for the incremental MS/TP stream decoder.

use bacnet_mstp_codec::crc::{crc16, crc8};
use bacnet_mstp_codec::{
    DecodeEvent, Frame, FrameError, FrameType, StreamDecoder, StreamPhase,
    MAX_EXTENDED_FRAME_LENGTH, MAX_STANDARD_FRAME_LENGTH, PREAMBLE,
};
use proptest::prelude::*;

fn encode(frame: Frame<'_>) -> Vec<u8> {
    let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
    let used = frame.encode_into(&mut wire).unwrap();
    wire[..used].to_vec()
}

/// Push empty slices until the decoder has nothing left to report, and
/// collect the decoded frames' (type, destination, source).
fn drain(decoder: &mut StreamDecoder<'_>) -> Vec<(FrameType, u8, u8)> {
    let mut frames = Vec::new();
    for _ in 0..MAX_STANDARD_FRAME_LENGTH {
        match decoder.push_slice(&[]) {
            DecodeEvent::NeedMore => return frames,
            DecodeEvent::Frame { frame, .. } => frames.push((
                frame.header.frame_type,
                frame.header.destination,
                frame.header.source,
            )),
            DecodeEvent::Error(error) => panic!("unexpected stream error {error:?}"),
            DecodeEvent::Invalid { .. } | DecodeEvent::Malformed { .. } => {}
        }
    }
    panic!("decoder never drained");
}

#[test]
fn stream_resync_and_bad_data_crc_discard() {
    let frame = Frame::new(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 0, 7, &[1, 2]);
    let mut wire = encode(frame);
    let used = wire.len();
    wire[used - 1] ^= 0x80;
    let mut storage = [0u8; 32];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert_eq!(
        decoder.push_slice(&wire),
        DecodeEvent::Malformed {
            discard: used,
            error: FrameError::DataCrcMismatch { length: 2 },
            destination: Some(0),
        }
    );
    let valid = encode(frame);
    assert_eq!(
        decoder.push_slice(&[9]),
        DecodeEvent::Invalid { discard: 1 }
    );
    assert!(matches!(
        decoder.push_slice(&valid),
        DecodeEvent::Frame { .. }
    ));
    assert_eq!(decoder.buffered_len(), valid.len());
}

#[test]
fn stream_phase_excludes_retained_resynchronization_noise() {
    let mut storage = [0; 64];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert_eq!(decoder.phase(), StreamPhase::Sync);
    assert_eq!(decoder.push(PREAMBLE[0]), DecodeEvent::NeedMore);
    assert_eq!(decoder.phase(), StreamPhase::Preamble);
    assert_eq!(decoder.push(PREAMBLE[1]), DecodeEvent::NeedMore);
    assert_eq!(decoder.phase(), StreamPhase::Header);

    let mut encoded = encode(Frame::new(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        1,
        2,
        &[9],
    ));
    decoder.reset();
    assert_eq!(decoder.push_slice(&encoded[..8]), DecodeEvent::NeedMore);
    assert_eq!(decoder.phase(), StreamPhase::Data);

    decoder.reset();
    encoded[7] ^= 0x01;
    assert!(matches!(
        decoder.push_slice(&encoded),
        DecodeEvent::Malformed { .. }
    ));
    assert_eq!(decoder.phase(), StreamPhase::Sync);
}

#[test]
fn stream_retains_suffix_after_two_frames_in_one_slice() {
    let mut wire = encode(Frame::new(FrameType::TOKEN, 0, 7, &[]));
    wire.extend(encode(Frame::new(FrameType::POLL_FOR_MASTER, 7, 0, &[])));
    let mut storage = [0u8; 16];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert!(matches!(
        decoder.push_slice(&wire),
        DecodeEvent::Frame { consumed: 8, .. }
    ));
    assert_eq!(decoder.buffered_len(), 16);
    assert!(matches!(
        decoder.push_slice(&[]),
        DecodeEvent::Frame { consumed: 8, .. }
    ));
    // The next push commits the second frame.
    assert_eq!(decoder.push_slice(&[]), DecodeEvent::NeedMore);
    assert_eq!(decoder.buffered_len(), 0);
}

#[test]
fn stream_exhaustion_preserves_state_and_recovers() {
    let wire = encode(Frame::new(FrameType::TOKEN, 0, 7, &[]));
    let mut storage = [0u8; 8];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert_eq!(
        decoder.push_slice(&[0u8; 9]),
        DecodeEvent::Error(FrameError::BufferExhausted {
            needed: 9,
            available: 8
        })
    );
    assert_eq!(decoder.buffered_len(), 0);
    assert!(matches!(
        decoder.push_slice(&wire),
        DecodeEvent::Frame { consumed: 8, .. }
    ));
}

/// `BufferExhausted` reports the refused slice and the room that was free,
/// not totals, and leaves what was buffered alone.
#[test]
fn refused_slice_reports_its_length_and_the_free_room() {
    let wire = encode(Frame::new(FrameType::TEST_REQUEST, 1, 2, &[0xAB; 6]));
    assert_eq!(wire.len(), 16);
    let mut storage = [0u8; 20];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert_eq!(decoder.push_slice(&wire[..10]), DecodeEvent::NeedMore);
    assert_eq!(
        decoder.push_slice(&[0u8; 11]),
        DecodeEvent::Error(FrameError::BufferExhausted {
            needed: 11,
            available: 10
        })
    );
    assert_eq!(decoder.buffered_len(), 10);
    assert!(matches!(
        decoder.push_slice(&wire[10..]),
        DecodeEvent::Frame { consumed: 16, .. }
    ));
}

/// A push commits the frame delivered before it ahead of measuring the free
/// room, so a refused slice reports the room the frame left behind, and the
/// frame is not delivered again.
#[test]
fn refused_slice_after_a_delivered_frame_counts_the_frame_as_committed() {
    let token = encode(Frame::new(FrameType::TOKEN, 1, 2, &[]));
    let mut wire = token.clone();
    wire.extend_from_slice(&token[..4]);
    let mut storage = [0u8; 16];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert!(matches!(
        decoder.push_slice(&wire),
        DecodeEvent::Frame { consumed: 8, .. }
    ));
    assert_eq!(decoder.buffered_len(), 12);
    assert_eq!(
        decoder.push_slice(&[0u8; 13]),
        DecodeEvent::Error(FrameError::BufferExhausted {
            needed: 13,
            available: 12
        })
    );
    // The delivered frame is gone; the half token behind it stays.
    assert_eq!(decoder.buffered_len(), 4);
    assert!(matches!(
        decoder.push_slice(&token[4..]),
        DecodeEvent::Frame { consumed: 8, .. }
    ));
    assert_eq!(decoder.push_slice(&[]), DecodeEvent::NeedMore);
    assert_eq!(decoder.buffered_len(), 0);
}

/// Storage of exactly a frame's length decodes it, octet by octet or whole.
#[test]
fn storage_of_exactly_one_frame_decodes_it() {
    let wire = encode(Frame::new(
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        9,
        8,
        &[0x01, 0x04, 0x00, 0x05],
    ));
    let mut storage = vec![0u8; wire.len()];
    let mut decoder = StreamDecoder::new(&mut storage);
    for (index, &byte) in wire.iter().enumerate() {
        let event = decoder.push(byte);
        if index + 1 < wire.len() {
            assert_eq!(event, DecodeEvent::NeedMore, "octet {index}");
        } else {
            assert!(
                matches!(event, DecodeEvent::Frame { consumed, .. } if consumed == wire.len()),
                "{event:?}"
            );
        }
    }
    assert_eq!(decoder.push_slice(&[]), DecodeEvent::NeedMore);
    assert!(matches!(
        decoder.push_slice(&wire),
        DecodeEvent::Frame { consumed, .. } if consumed == wire.len()
    ));
}

/// Frames behind others in one push are checked and decoded where they
/// lie, past the octets already dropped: a standard and an extended data
/// frame, then one with a bad data CRC, whose destination is still read
/// from its own header.
#[test]
fn frames_behind_others_in_one_push_decode_where_they_lie() {
    let standard = [7u8; 10];
    // Zeros among the data, so COBS has blocks to undo.
    let extended: Vec<u8> = (0..600u16).map(|index| (index % 7) as u8).collect();
    let mut stream = vec![0x00];
    for frame in [
        Frame::new(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 1, 2, &standard),
        Frame::new(
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            3,
            2,
            &extended,
        ),
    ] {
        let mut wire = vec![0u8; MAX_EXTENDED_FRAME_LENGTH];
        let used = frame.encode_into(&mut wire).unwrap();
        stream.extend_from_slice(&wire[..used]);
    }
    let mut bad = encode(Frame::new(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        5,
        2,
        &[1, 2, 3],
    ));
    let last = bad.len() - 1;
    bad[last] ^= 0x80;
    stream.extend_from_slice(&bad);

    let mut storage = vec![0u8; stream.len()];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert_eq!(
        decoder.push_slice(&stream),
        DecodeEvent::Invalid { discard: 1 }
    );
    match decoder.push_slice(&[]) {
        DecodeEvent::Frame { frame, .. } => {
            assert_eq!((frame.header.destination, frame.data), (1, &standard[..]));
        }
        other => panic!("expected the standard frame, got {other:?}"),
    }
    match decoder.push_slice(&[]) {
        DecodeEvent::Frame { frame, .. } => {
            assert_eq!(frame.header.destination, 3);
            assert_eq!(frame.data, &extended[..]);
        }
        other => panic!("expected the extended frame, got {other:?}"),
    }
    assert_eq!(
        decoder.push_slice(&[]),
        DecodeEvent::Malformed {
            discard: bad.len(),
            error: FrameError::DataCrcMismatch { length: 3 },
            destination: Some(5),
        }
    );
    assert_eq!(decoder.push_slice(&[]), DecodeEvent::NeedMore);
    assert_eq!(decoder.buffered_len(), 0);
}

/// The phase reads the octets still buffered, wherever dropped noise left
/// them in the storage.
#[test]
fn the_phase_reads_the_octets_behind_dropped_noise() {
    let mut storage = [0u8; 16];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert_eq!(
        decoder.push_slice(&[0x00, PREAMBLE[0]]),
        DecodeEvent::Invalid { discard: 1 }
    );
    assert_eq!(decoder.push_slice(&[]), DecodeEvent::NeedMore);
    assert_eq!(decoder.phase(), StreamPhase::Preamble);
    assert_eq!(decoder.push(PREAMBLE[1]), DecodeEvent::NeedMore);
    assert_eq!(decoder.phase(), StreamPhase::Header);
    assert_eq!(decoder.buffered_len(), 2);
}

/// Octets dropped from the front leave the rest where they are, so a frame
/// that then needs the whole storage is moved to its front, here past a
/// one-octet gap, and still decodes whole: every octet of it in place, at
/// any data length.
#[test]
fn a_frame_behind_dropped_octets_moves_to_the_front_intact() {
    for length in [1, 2, 7, 40, 200, 501] {
        let data: Vec<u8> = (0..length).map(|index| (index * 13 + 5) as u8).collect();
        let wire = encode(Frame::new(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            4,
            3,
            &data,
        ));
        let mut storage = vec![0u8; wire.len()];
        let mut decoder = StreamDecoder::new(&mut storage);
        // A lone 0x55 might start a preamble, so it is kept, then dropped as
        // noise once the next octet, the frame's own 0x55, follows it.
        assert_eq!(decoder.push(PREAMBLE[0]), DecodeEvent::NeedMore);
        let (head, last) = wire.split_at(wire.len() - 1);
        assert_eq!(
            decoder.push_slice(head),
            DecodeEvent::Invalid { discard: 1 }
        );
        assert_eq!(decoder.push_slice(&[]), DecodeEvent::NeedMore);
        assert_eq!(decoder.buffered_len(), head.len());
        match decoder.push_slice(last) {
            DecodeEvent::Frame { frame, consumed } => {
                assert_eq!(consumed, wire.len());
                assert_eq!((frame.header.destination, frame.header.source), (4, 3));
                assert_eq!(frame.data, &data[..], "data length {length}");
            }
            other => panic!("data length {length}: {other:?}"),
        }
        assert_eq!(decoder.push_slice(&[]), DecodeEvent::NeedMore);
        assert_eq!(decoder.buffered_len(), 0);
    }
}

/// A dropped candidate carries its destination once the header CRC passed,
/// so a receiver can ignore trouble on frames meant for other stations.
#[test]
fn malformed_candidates_carry_a_trusted_destination() {
    let mut storage = [0u8; 64];
    let mut decoder = StreamDecoder::new(&mut storage);

    // Reserved COBS type 34 to station 5 with the smallest COBS Length: a
    // good header, so the destination is known, and the frame's other seven
    // octets are skipped as they arrive.
    let fields = [0x22, 0x05, 0x01, 0x00, 0x05];
    let mut cobs = PREAMBLE.to_vec();
    cobs.extend_from_slice(&fields);
    cobs.push(crc8(&fields));
    assert_eq!(
        decoder.push_slice(&cobs),
        DecodeEvent::Malformed {
            discard: 15,
            error: FrameError::UnsupportedCobsFrame { raw_type: 0x22 },
            destination: Some(5),
        }
    );
    assert_eq!(drain(&mut decoder), []);
    assert_eq!(decoder.phase(), StreamPhase::Data);
    assert_eq!(decoder.push_slice(&[0x55; 7]), DecodeEvent::NeedMore);
    assert_eq!(decoder.phase(), StreamPhase::Sync);

    // A COBS header whose Length is out of range is a bad header: its
    // destination is known, but only its first octet goes.
    let fields = [0x20, 0x05, 0x01, 0x00, 0x04];
    let mut short = PREAMBLE.to_vec();
    short.extend_from_slice(&fields);
    short.push(crc8(&fields));
    assert_eq!(
        decoder.push_slice(&short),
        DecodeEvent::Malformed {
            discard: 1,
            error: FrameError::InvalidCobsLength { length: 4 },
            destination: Some(5),
        }
    );
    assert_eq!(drain(&mut decoder), []);

    // A bad header CRC: the destination octet can't be trusted.
    let mut bad = encode(Frame::new(FrameType::TOKEN, 5, 1, &[]));
    bad[7] ^= 0xFF;
    assert_eq!(
        decoder.push_slice(&bad),
        DecodeEvent::Malformed {
            discard: 1,
            error: FrameError::HeaderCrcMismatch,
            destination: None,
        }
    );
}

/// A frame longer than the whole storage used to wedge the decoder: every
/// later push failed with `BufferExhausted` until `reset()`. Now the decoder
/// reports it once, as soon as the header shows its size, skips the rest of
/// it by count, and keeps going.
#[test]
fn frame_longer_than_storage_is_reported_once_then_decoding_resumes() {
    let long = encode(Frame::new(
        FrameType::TEST_REQUEST,
        1,
        2,
        &[0x11, 0x22, 0x33],
    ));
    assert_eq!(long.len(), 13);
    let token = encode(Frame::new(FrameType::TOKEN, 4, 3, &[]));
    let mut storage = [0u8; 12];
    let mut decoder = StreamDecoder::new(&mut storage);

    let mut too_long = 0;
    let mut frames = Vec::new();
    for (index, &byte) in long.iter().chain(&token).enumerate() {
        match decoder.push(byte) {
            DecodeEvent::Malformed {
                discard: 13,
                error: FrameError::FrameTooLong { needed, capacity },
                destination,
            } => {
                // Reported when the header completes, not when storage fills.
                assert_eq!((index, needed, capacity), (7, 13, 12));
                assert_eq!(destination, Some(1));
                too_long += 1;
            }
            DecodeEvent::Frame { frame, .. } => frames.push((
                frame.header.frame_type,
                frame.header.destination,
                frame.header.source,
            )),
            DecodeEvent::Error(error) => panic!("decoder wedged at octet {index}: {error:?}"),
            _ => {}
        }
    }
    frames.extend(drain(&mut decoder));
    assert_eq!(too_long, 1);
    assert_eq!(frames, [(FrameType::TOKEN, 4, 3)]);

    // The same storage still takes a frame that fits.
    decoder.reset();
    assert!(matches!(
        decoder.push_slice(&token),
        DecodeEvent::Frame { consumed: 8, .. }
    ));
}

/// The review's reproduction: the first 12 octets of a 13-octet frame fill
/// the storage exactly.
#[test]
fn full_storage_with_an_unfinishable_frame_recovers() {
    let long = encode(Frame::new(
        FrameType::TEST_REQUEST,
        1,
        2,
        &[0x11, 0x22, 0x33],
    ));
    let mut storage = [0u8; 12];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert_eq!(
        decoder.push_slice(&long[..12]),
        DecodeEvent::Malformed {
            discard: 13,
            error: FrameError::FrameTooLong {
                needed: 13,
                capacity: 12
            },
            destination: Some(1),
        }
    );
    let last = decoder.push(long[12]);
    assert!(!matches!(last, DecodeEvent::Error(_)), "{last:?}");
    assert_eq!(drain(&mut decoder), []);
    assert_eq!(decoder.buffered_len(), 0);
}

/// Storage smaller than a header can never decode anything, but it must not
/// wedge either.
#[test]
fn storage_smaller_than_a_header_reports_and_resynchronizes() {
    let token = encode(Frame::new(FrameType::TOKEN, 4, 3, &[]));
    let mut storage = [0u8; 4];
    let mut decoder = StreamDecoder::new(&mut storage);
    let mut reports = 0;
    for &byte in &token {
        match decoder.push(byte) {
            DecodeEvent::Malformed {
                error:
                    FrameError::FrameTooLong {
                        needed: 8,
                        capacity: 4,
                    },
                destination: None,
                ..
            } => reports += 1,
            DecodeEvent::Error(error) => panic!("decoder wedged: {error:?}"),
            _ => {}
        }
    }
    assert_eq!(reports, 1);
    assert_eq!(drain(&mut decoder), []);
}

#[test]
fn broadcast_source_is_malformed_and_slave_source_decodes() {
    let fields = [0x06, 0x01, 0xFF, 0x00, 0x00];
    let mut bad = PREAMBLE.to_vec();
    bad.extend_from_slice(&fields);
    bad.push(crc8(&fields));
    let mut storage = [0u8; 64];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert_eq!(
        decoder.push_slice(&bad),
        DecodeEvent::Malformed {
            discard: 1,
            error: FrameError::BroadcastSource,
            destination: Some(1),
        }
    );
    assert_eq!(drain(&mut decoder), []);

    let data = [0x01, 0x00];
    let mut slave = vec![0x55, 0xFF, 0x06, 0x01, 0xC8, 0x00, 0x02, 0xEF];
    slave.extend_from_slice(&data);
    slave.extend_from_slice(&crc16(&data).to_le_bytes());
    match decoder.push_slice(&slave) {
        DecodeEvent::Frame { frame, consumed } => {
            assert_eq!(consumed, slave.len());
            assert_eq!(frame.header.source, 200);
            assert_eq!(frame.data, data);
        }
        other => panic!("expected a frame, got {other:?}"),
    }
}

proptest! {
    #[test]
    fn byte_stream_resynchronizes_then_decodes(
        garbage in prop::collection::vec(0u8..=0x54, 0..64),
        payload in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let wire = encode(Frame::new(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 0, 7, &payload));
        let used = wire.len();
        let mut storage = [0u8; MAX_STANDARD_FRAME_LENGTH];
        let mut decoder = StreamDecoder::new(&mut storage);
        for byte in garbage {
            prop_assert_eq!(decoder.push(byte), DecodeEvent::Invalid { discard: 1 });
        }
        for (index, byte) in wire.iter().copied().enumerate() {
            let event = decoder.push(byte);
            if index + 1 == used {
                let completed =
                    matches!(event, DecodeEvent::Frame { consumed, .. } if consumed == used);
                prop_assert!(completed);
            } else {
                prop_assert_eq!(event, DecodeEvent::NeedMore);
            }
        }
    }

    /// Whatever the storage size and input, octet-at-a-time pushes are never
    /// refused, and draining terminates.
    #[test]
    fn decoder_never_wedges(
        capacity in 1usize..64,
        noise in prop::collection::vec(any::<u8>(), 0..128),
    ) {
        let token = encode(Frame::new(FrameType::TOKEN, 4, 3, &[]));
        let mut storage = vec![0u8; capacity];
        let mut decoder = StreamDecoder::new(&mut storage);
        for &byte in noise.iter().chain(&token) {
            let wedged = matches!(decoder.push(byte), DecodeEvent::Error(_));
            prop_assert!(!wedged);
        }
        drain(&mut decoder);
        // Noise can leave a partial candidate waiting for octets that never
        // come; a receiver's frame-abort timer resets it.
        decoder.reset();
        let decoded = token
            .iter()
            .any(|&byte| matches!(decoder.push(byte), DecodeEvent::Frame { .. }));
        prop_assert_eq!(decoded, capacity >= token.len());
    }
}
