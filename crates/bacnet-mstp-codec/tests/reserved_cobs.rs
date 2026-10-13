//! The stream decoder's reserved-COBS mode (#1611): a receiver that supports
//! COBS-encoded frames checks the CRC-32K of the reserved COBS types, 34-127,
//! and takes a good one as a valid frame of a type it doesn't know (Clause
//! 9.5.4.9). Without the mode the decoder skips them by count, as before.

use bacnet_mstp_codec::crc::crc8;
use bacnet_mstp_codec::{
    DecodeEvent, Frame, FrameError, FrameType, StreamDecoder, MAX_COBS_FRAME_LENGTH,
    MAX_EXTENDED_FRAME_LENGTH, MAX_STANDARD_FRAME_LENGTH,
};
use proptest::prelude::*;

/// What a decoder reports, with frames reduced to their fields and data.
#[derive(Debug, PartialEq)]
enum Seen {
    Frame(FrameType, u8, u8, Vec<u8>),
    Invalid(usize),
    Malformed(usize, FrameError, Option<u8>),
}

fn encode(frame: Frame<'_>) -> Vec<u8> {
    let mut wire = vec![0u8; MAX_COBS_FRAME_LENGTH];
    let used = frame.encode_into(&mut wire).unwrap();
    wire.truncate(used);
    wire
}

/// A frame of reserved COBS type `raw` carrying `data`. The encoder refuses
/// the type, so this encodes a Type 33 frame and rewrites its type octet and
/// header CRC: the encoded fields, and their CRC-32K, are the same for every
/// COBS type (Clause 9.10).
fn reserved(raw: u8, destination: u8, source: u8, data: &[u8]) -> Vec<u8> {
    let mut wire = encode(Frame::new(
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        destination,
        source,
        data,
    ));
    wire[2] = raw;
    wire[7] = crc8(&wire[2..7]);
    wire
}

fn token(destination: u8, source: u8) -> Vec<u8> {
    encode(Frame::new(FrameType::TOKEN, destination, source, &[]))
}

fn token_seen(destination: u8, source: u8) -> Seen {
    Seen::Frame(FrameType::TOKEN, destination, source, Vec::new())
}

/// Push `wire` one octet at a time, draining after each, and collect
/// everything but `NeedMore`.
fn feed(decoder: &mut StreamDecoder<'_>, wire: &[u8]) -> Vec<Seen> {
    let mut seen = Vec::new();
    for &octet in wire {
        let mut input: &[u8] = &[octet];
        loop {
            match decoder.push_slice(input) {
                DecodeEvent::NeedMore => break,
                DecodeEvent::Frame { frame, .. } => seen.push(Seen::Frame(
                    frame.header.frame_type,
                    frame.header.destination,
                    frame.header.source,
                    frame.data.to_vec(),
                )),
                DecodeEvent::Invalid { discard } => seen.push(Seen::Invalid(discard)),
                DecodeEvent::Malformed {
                    discard,
                    error,
                    destination,
                } => seen.push(Seen::Malformed(discard, error, destination)),
                DecodeEvent::Error(error) => panic!("decoder wedged: {error:?}"),
            }
            input = &[];
        }
    }
    seen
}

/// Feed `wire` through a fresh decoder over `capacity` octets, with or
/// without the mode.
fn decode(capacity: usize, checking: bool, wire: &[u8]) -> Vec<Seen> {
    let mut storage = vec![0u8; capacity];
    let mut decoder = StreamDecoder::new(&mut storage);
    if checking {
        decoder = decoder.with_reserved_cobs();
    }
    assert_eq!(decoder.checks_reserved_cobs(), checking);
    feed(&mut decoder, wire)
}

/// Clause 9.5.4.9 (GoodCRC): in the mode a reserved type with a good CRC-32K
/// comes out whole, with its type and its decoded data. Without the mode the
/// same octets are skipped by count once the header passes, as a receiver
/// without COBS support skips them.
#[test]
fn a_good_reserved_frame_comes_out_only_in_the_mode() {
    let data: Vec<u8> = (0..40u8).map(|octet| octet.wrapping_mul(7)).collect();
    let frame = reserved(0x22, 9, 5, &data);
    let mut wire = frame.clone();
    wire.extend(token(1, 2));
    assert_eq!(
        decode(MAX_STANDARD_FRAME_LENGTH, true, &wire),
        [
            Seen::Frame(FrameType::from_raw(0x22), 9, 5, data),
            token_seen(1, 2)
        ]
    );
    assert_eq!(
        decode(MAX_STANDARD_FRAME_LENGTH, false, &wire),
        [
            Seen::Malformed(
                frame.len(),
                FrameError::UnsupportedCobsFrame { raw_type: 0x22 },
                Some(9)
            ),
            token_seen(1, 2)
        ]
    );
}

/// Clause 9.5.4.9 (BadCRC): in the mode a reserved type whose CRC-32K
/// fails is dropped as bad data, and skipped whole by its trusted Length.
#[test]
fn a_damaged_reserved_frame_is_bad_data_in_the_mode() {
    let mut frame = reserved(0x7F, 0xFF, 5, &[0x11; 30]);
    frame[12] ^= 0x01;
    let length = usize::from(u16::from_be_bytes([frame[5], frame[6]]));
    let mut wire = frame.clone();
    wire.extend(token(1, 2));
    assert_eq!(
        decode(MAX_STANDARD_FRAME_LENGTH, true, &wire),
        [
            Seen::Malformed(
                frame.len(),
                FrameError::DataCrcMismatch { length },
                Some(0xFF)
            ),
            token_seen(1, 2)
        ]
    );
}

/// A reserved frame longer than the storage can't be checked: in the mode
/// it is reported as too long for the storage and skipped by count, as an
/// extended frame would be.
#[test]
fn a_reserved_frame_too_long_for_the_storage_is_skipped() {
    let frame = reserved(0x40, 3, 5, &[0x22; 600]);
    let mut wire = frame.clone();
    wire.extend(token(1, 2));
    assert_eq!(
        decode(MAX_STANDARD_FRAME_LENGTH, true, &wire),
        [
            Seen::Malformed(
                frame.len(),
                FrameError::FrameTooLong {
                    needed: frame.len(),
                    capacity: MAX_STANDARD_FRAME_LENGTH
                },
                Some(3)
            ),
            token_seen(1, 2)
        ]
    );
}

/// The mode changes nothing for the two extended data types, and a bad
/// header stays a bad header: the Length bounds come before the type.
#[test]
fn the_mode_leaves_other_frames_alone() {
    let npdu = [0x01, 0x00, 0x10, 0x08];
    for frame_type in [
        FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
    ] {
        let wire = encode(Frame::new(frame_type, 4, 6, &npdu));
        for checking in [false, true] {
            assert_eq!(
                decode(MAX_EXTENDED_FRAME_LENGTH, checking, &wire),
                [Seen::Frame(frame_type, 4, 6, npdu.to_vec())]
            );
        }
    }
    // A reserved type with a Length below the COBS minimum.
    let mut short = reserved(0x22, 9, 5, &[1]);
    short[6] = 4;
    short[7] = crc8(&short[2..7]);
    let seen = decode(MAX_STANDARD_FRAME_LENGTH, true, &short);
    assert_eq!(
        seen[0],
        Seen::Malformed(1, FrameError::InvalidCobsLength { length: 4 }, Some(9))
    );
}

proptest! {
    /// Every reserved type round-trips its data through the mode, whatever
    /// the data, with the frame after it intact.
    #[test]
    fn every_reserved_type_round_trips(
        raw in 0x22u8..=0x7F,
        destination: u8,
        source in 0u8..=0xFE,
        data in proptest::collection::vec(any::<u8>(), 1..700),
    ) {
        let frame = reserved(raw, destination, source, &data);
        let mut wire = frame;
        wire.extend(token(1, 2));
        prop_assert_eq!(
            decode(MAX_EXTENDED_FRAME_LENGTH, true, &wire),
            vec![
                Seen::Frame(FrameType::from_raw(raw), destination, source, data),
                token_seen(1, 2),
            ]
        );
    }
}
