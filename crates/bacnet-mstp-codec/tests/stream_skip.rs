//! The stream decoder's counted skip and noise runs (#1609), and extended
//! frames through it (#1611).

use bacnet_mstp_codec::crc::crc8;
use bacnet_mstp_codec::{
    DecodeEvent, Frame, FrameError, FrameType, StreamDecoder, StreamPhase, MAX_COBS_FRAME_LENGTH,
    MAX_EXTENDED_FRAME_LENGTH, MAX_STANDARD_FRAME_LENGTH, PREAMBLE,
};
use proptest::prelude::*;

fn encode(frame: Frame<'_>) -> Vec<u8> {
    let mut wire = vec![0u8; MAX_COBS_FRAME_LENGTH];
    let used = frame.encode_into(&mut wire).unwrap();
    wire.truncate(used);
    wire
}

/// A header with a correct CRC, for field values encode refuses.
fn raw_header(raw_type: u8, destination: u8, source: u8, length: u16) -> Vec<u8> {
    let [high, low] = length.to_be_bytes();
    let fields = [raw_type, destination, source, high, low];
    let mut wire = PREAMBLE.to_vec();
    wire.extend_from_slice(&fields);
    wire.push(crc8(&fields));
    wire
}

/// What a decoder reports, with frames reduced to their fields and data.
#[derive(Debug, PartialEq)]
enum Seen {
    Frame(FrameType, u8, u8, Vec<u8>),
    Invalid(usize),
    Malformed(usize, FrameError, Option<u8>),
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

fn token(destination: u8, source: u8) -> Vec<u8> {
    encode(Frame::new(FrameType::TOKEN, destination, source, &[]))
}

fn token_seen(destination: u8, source: u8) -> Seen {
    Seen::Frame(FrameType::TOKEN, destination, source, Vec::new())
}

/// A run of noise goes in one event, up to the next 0x55, which might start
/// a preamble; a 0x55 that doesn't goes with the run after it.
#[test]
fn a_noise_run_is_one_event() {
    let mut storage = [0u8; 64];
    let mut decoder = StreamDecoder::new(&mut storage);
    let mut wire = vec![0x00, 0x11, 0x22, 0x55, 0x00, 0x33, 0x55];
    wire.extend(token(1, 2));
    assert_eq!(
        decoder.push_slice(&wire),
        DecodeEvent::Invalid { discard: 3 }
    );
    assert_eq!(decoder.push_slice(&[]), DecodeEvent::Invalid { discard: 3 });
    // 55 55 FF: the first 0x55 is noise, the second starts the Token.
    assert_eq!(decoder.push_slice(&[]), DecodeEvent::Invalid { discard: 1 });
    assert!(matches!(
        decoder.push_slice(&[]),
        DecodeEvent::Frame { consumed: 8, .. }
    ));
    // Noise with no 0x55 at all goes whole.
    assert_eq!(
        decoder.push_slice(&[0x01; 40]),
        DecodeEvent::Invalid { discard: 40 }
    );
    assert_eq!(decoder.buffered_len(), 0);
}

/// The review's case for #1609: a data frame carrying a Token's octets,
/// longer than the storage. Its first octet used to be dropped and the rest
/// rescanned, and the Token inside came out; now the frame is skipped whole.
#[test]
fn no_frame_comes_out_of_a_frame_too_long_for_the_storage() {
    let inner = token(4, 3);
    let mut data = vec![0x01, 0x00];
    data.extend_from_slice(&inner);
    let outer = encode(Frame::new(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        9,
        5,
        &data,
    ));
    assert_eq!(outer.len(), 20);
    let mut wire = outer.clone();
    wire.extend(token(1, 2));
    let mut storage = [0u8; 16];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert_eq!(
        feed(&mut decoder, &wire),
        [
            Seen::Malformed(
                20,
                FrameError::FrameTooLong {
                    needed: 20,
                    capacity: 16
                },
                Some(9)
            ),
            token_seen(1, 2),
        ]
    );
}

/// Frames dropped after a good header for other reasons are skipped whole
/// too: a bad data CRC, and a reserved COBS type.
#[test]
fn no_frame_comes_out_of_a_dropped_body() {
    let inner = token(4, 3);
    let mut bad_crc = encode(Frame::new(FrameType::TEST_REQUEST, 9, 5, &inner));
    let last = bad_crc.len() - 1;
    bad_crc[last] ^= 0x01;
    // A reserved COBS type whose Length covers a Token's octets and two more.
    let mut reserved = raw_header(0x22, 9, 5, 8);
    reserved.extend_from_slice(&inner);
    reserved.extend_from_slice(&[0x00, 0x00]);
    for (candidate, error) in [
        (bad_crc, FrameError::DataCrcMismatch { length: 8 }),
        (
            reserved,
            FrameError::UnsupportedCobsFrame { raw_type: 0x22 },
        ),
    ] {
        let mut wire = candidate.clone();
        wire.extend(token(1, 2));
        let mut storage = [0u8; MAX_STANDARD_FRAME_LENGTH];
        let mut decoder = StreamDecoder::new(&mut storage);
        assert_eq!(
            feed(&mut decoder, &wire),
            [
                Seen::Malformed(candidate.len(), error, Some(9)),
                token_seen(1, 2)
            ],
            "{error:?}"
        );
    }
}

/// A bad header says nothing reliable about the frame's length (a Length
/// over 501 on a standard type, source 255): only its first octet goes, and
/// what follows is scanned again, so a Token right after it still comes out.
#[test]
fn a_bad_header_is_not_skipped_by_its_length() {
    for (header, error) in [
        (
            raw_header(0x06, 9, 5, 502),
            FrameError::Oversize {
                length: 502,
                maximum: 501,
            },
        ),
        (raw_header(0x06, 9, 0xFF, 40), FrameError::BroadcastSource),
        (
            raw_header(0x21, 9, 5, 2044),
            FrameError::InvalidCobsLength { length: 2044 },
        ),
    ] {
        let mut wire = header.clone();
        wire.extend(token(1, 2));
        let mut storage = [0u8; MAX_STANDARD_FRAME_LENGTH];
        let mut decoder = StreamDecoder::new(&mut storage);
        let seen = feed(&mut decoder, &wire);
        assert_eq!(seen[0], Seen::Malformed(1, error, Some(9)));
        assert_eq!(seen.last(), Some(&token_seen(1, 2)), "{seen:?}");
    }
}

/// While the decoder skips, it reports the data phase, needs no room for the
/// skipped octets, and a reset ends the skip.
#[test]
fn skipping_needs_no_room_and_a_reset_ends_it() {
    let long = encode(Frame::new(FrameType::TEST_REQUEST, 9, 5, &[0x11; 20]));
    assert_eq!(long.len(), 30);
    let mut storage = [0u8; 16];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert!(matches!(
        decoder.push_slice(&long[..8]),
        DecodeEvent::Malformed { discard: 30, .. }
    ));
    assert_eq!(decoder.phase(), StreamPhase::Data);
    assert_eq!(decoder.buffered_len(), 0);
    // The 22 octets left, then 17 more: one more than the storage takes.
    let mut rest = long[8..].to_vec();
    rest.extend_from_slice(&[0x01; 17]);
    assert_eq!(
        decoder.push_slice(&rest),
        DecodeEvent::Error(FrameError::BufferExhausted {
            needed: 17,
            available: 16
        })
    );
    // Nothing was taken: with one octet fewer, the push fits.
    assert_eq!(
        decoder.push_slice(&rest[..rest.len() - 1]),
        DecodeEvent::Invalid { discard: 16 }
    );
    assert_eq!(decoder.phase(), StreamPhase::Sync);

    assert!(matches!(
        decoder.push_slice(&long[..8]),
        DecodeEvent::Malformed { discard: 30, .. }
    ));
    decoder.reset();
    assert_eq!(decoder.phase(), StreamPhase::Sync);
    assert!(matches!(
        decoder.push_slice(&token(1, 2)),
        DecodeEvent::Frame { consumed: 8, .. }
    ));
}

/// Storage for standard frames reports an extended frame too long for it,
/// with its destination, and skips it; storage for extended frames decodes
/// it, in place.
#[test]
fn extended_frames_by_storage_size() {
    let npdu: Vec<u8> = (0..1200u32).map(|index| (index % 251) as u8).collect();
    let frame = Frame::new(FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY, 9, 5, &npdu);
    let extended = encode(frame);
    let mut wire = extended.clone();
    wire.extend(token(1, 2));

    let mut storage = [0u8; MAX_STANDARD_FRAME_LENGTH];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert_eq!(
        feed(&mut decoder, &wire),
        [
            Seen::Malformed(
                extended.len(),
                FrameError::FrameTooLong {
                    needed: extended.len(),
                    capacity: MAX_STANDARD_FRAME_LENGTH
                },
                Some(9)
            ),
            token_seen(1, 2),
        ]
    );

    let mut storage = [0u8; MAX_EXTENDED_FRAME_LENGTH];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert_eq!(
        feed(&mut decoder, &wire),
        [
            Seen::Frame(FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY, 9, 5, npdu),
            token_seen(1, 2),
        ]
    );
}

proptest! {
    /// An extended frame and a Token, pushed in pieces split anywhere, come
    /// out whole, with the decoded data.
    #[test]
    fn extended_frames_split_anywhere(
        npdu in prop::collection::vec(prop_oneof![Just(0u8), Just(0x55), any::<u8>()], 1..1497),
        cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..8),
    ) {
        let frame = Frame::new(FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY, 0xFF, 200, &npdu);
        let mut wire = encode(frame);
        wire.extend(token(1, 2));
        let mut cuts: Vec<usize> = cuts.iter().map(|cut| cut.index(wire.len())).collect();
        cuts.push(wire.len());
        cuts.sort_unstable();
        let mut storage = vec![0u8; MAX_EXTENDED_FRAME_LENGTH + 8];
        let mut decoder = StreamDecoder::new(&mut storage);
        let mut seen = Vec::new();
        let mut start = 0;
        for cut in cuts {
            let mut input = &wire[start..cut];
            start = cut;
            loop {
                match decoder.push_slice(input) {
                    DecodeEvent::NeedMore => break,
                    DecodeEvent::Frame { frame, .. } => {
                        seen.push((frame.header.frame_type, frame.data.to_vec()));
                    }
                    other => prop_assert!(false, "unexpected {:?}", other),
                }
                input = &[];
            }
        }
        prop_assert_eq!(
            seen,
            vec![
                (FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY, npdu.clone()),
                (FrameType::TOKEN, Vec::new()),
            ]
        );
    }

    /// Whatever the storage, a frame of any class followed by noise and a
    /// Token never wedges the decoder, and the Token comes out once the
    /// storage holds it, after the frame or its skip.
    #[test]
    fn skips_never_wedge(
        capacity in 8usize..1600,
        data in prop::collection::vec(any::<u8>(), 1..1500),
        extended in any::<bool>(),
        noise in prop::collection::vec(0u8..=0x54, 0..32),
    ) {
        let frame_type = if extended {
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
        } else {
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
        };
        let data = if extended { &data[..] } else { &data[..data.len().min(501)] };
        let mut wire = encode(Frame::new(frame_type, 3, 4, data));
        let frame_length = wire.len();
        wire.extend_from_slice(&noise);
        wire.extend(token(1, 2));
        let mut storage = vec![0u8; capacity];
        let mut decoder = StreamDecoder::new(&mut storage);
        let seen = feed(&mut decoder, &wire);
        prop_assert_eq!(seen.last(), Some(&token_seen(1, 2)));
        let first_is_frame = matches!(seen.first(), Some(Seen::Frame(..)));
        prop_assert_eq!(first_is_frame, frame_length <= capacity);
    }
}
