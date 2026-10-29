//! Cases from the host transport's own frame codec, deleted when the MS/TP
//! transport moved onto this crate (#1608), that the other tests here did not
//! already pin: literal encodings, deterministic length boundaries in both
//! decoders, a bad preamble, and the preamble search at its edges.

use bacnet_mstp_codec::crc::{crc8, crc8_valid, crc16, crc16_valid};
use bacnet_mstp_codec::{
    DecodeEvent, Frame, FrameError, FrameType, MAX_STANDARD_FRAME_LENGTH, MAX_STANDARD_MPDU_DATA,
    PREAMBLE, StreamDecoder, decode_frame, find_preamble,
};

mod support;
use support::{DATA_CRC_RESIDUAL, HEADER_CRC_RESIDUAL};

fn encode(frame: Frame<'_>) -> Vec<u8> {
    let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
    let length = frame.encode_into(&mut wire).unwrap();
    wire[..length].to_vec()
}

/// A header whose Length field says `length`, with a good header CRC.
fn header(raw_type: u8, destination: u8, source: u8, length: u16) -> Vec<u8> {
    let [high, low] = length.to_be_bytes();
    let fields = [raw_type, destination, source, high, low];
    let mut wire = PREAMBLE.to_vec();
    wire.extend_from_slice(&fields);
    wire.push(crc8(&fields));
    wire
}

#[test]
fn literal_frames_encode_to_their_known_octets() {
    let token = Frame::new(FrameType::TOKEN, 0, 7, &[]);
    assert_eq!(
        encode(token),
        [0x55, 0xFF, 0x00, 0x00, 0x07, 0x00, 0x00, 0x37]
    );
    let data = Frame::new(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        0,
        7,
        &[0x01, 0x00],
    );
    let wire = encode(data);
    assert_eq!(
        wire,
        [
            0x55, 0xFF, 0x06, 0x00, 0x07, 0x00, 0x02, 0xD9, 0x01, 0x00, 0x9F, 0x16
        ]
    );
    let (decoded, consumed) = decode_frame(&wire).unwrap();
    assert_eq!(
        (
            decoded.header.frame_type,
            decoded.header.destination,
            decoded.header.source
        ),
        (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 0, 7)
    );
    assert_eq!((decoded.data, consumed), (&[0x01, 0x00][..], 12));
}

#[test]
fn a_one_bit_header_or_data_error_fails_its_crc() {
    // The Token 0 from 7 header with its type octet's low bit set.
    let header = [0x01, 0x00, 0x07, 0x00, 0x00, 0x37];
    assert!(!crc8_valid(&header));
    assert_ne!(!crc8(&header), HEADER_CRC_RESIDUAL);
    // Data 01 00 with its first octet's low bit cleared.
    let data = [0x00, 0x00, 0x9F, 0x16];
    assert!(!crc16_valid(&data));
    assert_ne!(!crc16(&data), DATA_CRC_RESIDUAL);
    // And the good ones, for contrast.
    assert!(crc8_valid(&[0x00, 0x00, 0x07, 0x00, 0x00, 0x37]));
    assert!(crc16_valid(&[0x01, 0x00, 0x9F, 0x16]));
    assert_eq!(crc16(&[0x01, 0x00]).to_le_bytes(), [0x9F, 0x16]);
}

#[test]
fn a_broadcast_destination_round_trips() {
    let frame = Frame::new(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        0xFF,
        0x10,
        &[1, 0],
    );
    let wire = encode(frame);
    assert_eq!(decode_frame(&wire).unwrap(), (frame, wire.len()));
}

#[test]
fn a_bad_first_preamble_octet_is_named() {
    assert_eq!(
        decode_frame(&[0x00, 0xFF, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]),
        Err(FrameError::InvalidPreamble {
            first: 0x00,
            second: 0xFF
        })
    );
}

#[test]
fn the_preamble_search_finds_offset_zero_and_needs_adjacent_octets() {
    assert_eq!(
        find_preamble(&[0x55, 0xFF, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]),
        Some(0)
    );
    assert_eq!(find_preamble(&[0x00, 0x55, 0x00, 0xFF, 0x01]), None);
}

#[test]
fn the_largest_standard_frame_decodes_and_one_more_octet_does_not() {
    let data = [0xAA; MAX_STANDARD_MPDU_DATA];
    let wire = encode(Frame::new(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        0xFF,
        0,
        &data,
    ));
    assert_eq!(wire.len(), MAX_STANDARD_FRAME_LENGTH);
    let (frame, consumed) = decode_frame(&wire).unwrap();
    assert_eq!(
        (frame.data.len(), consumed),
        (MAX_STANDARD_MPDU_DATA, MAX_STANDARD_FRAME_LENGTH)
    );

    let oversize = header(0x06, 0x01, 0x00, 502);
    assert_eq!(
        decode_frame(&oversize),
        Err(FrameError::Oversize {
            length: 502,
            maximum: MAX_STANDARD_MPDU_DATA
        })
    );
}

#[test]
fn the_stream_decoder_takes_the_largest_frame_and_drops_a_longer_header() {
    let data = [0xAA; MAX_STANDARD_MPDU_DATA];
    let wire = encode(Frame::new(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        0xFF,
        0,
        &data,
    ));
    let mut storage = [0u8; MAX_STANDARD_FRAME_LENGTH];
    let mut decoder = StreamDecoder::new(&mut storage);
    match decoder.push_slice(&wire) {
        DecodeEvent::Frame { frame, consumed } => {
            assert_eq!(
                (frame.data.len(), consumed),
                (MAX_STANDARD_MPDU_DATA, MAX_STANDARD_FRAME_LENGTH)
            );
        }
        other => panic!("expected the frame, got {other:?}"),
    }
    assert_eq!(decoder.push_slice(&[]), DecodeEvent::NeedMore);

    // A standard-type header declaring more than 501 octets is a bad
    // header: it drops its first octet, and the rest of it drains as one
    // run of noise.
    let oversize = header(0x06, 0x01, 0x00, 502);
    assert_eq!(
        decoder.push_slice(&oversize),
        DecodeEvent::Malformed {
            discard: 1,
            error: FrameError::Oversize {
                length: 502,
                maximum: MAX_STANDARD_MPDU_DATA
            },
            destination: Some(0x01),
        }
    );
    assert_eq!(
        decoder.push_slice(&[]),
        DecodeEvent::Invalid {
            discard: oversize.len() - 1
        }
    );
    assert_eq!(decoder.push_slice(&[]), DecodeEvent::NeedMore);
}

#[test]
fn a_bad_second_preamble_octet_is_noise() {
    // Both octets go as one run of noise.
    let mut storage = [0u8; 16];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert_eq!(
        decoder.push_slice(&[0x55, 0x00]),
        DecodeEvent::Invalid { discard: 2 }
    );
    assert_eq!(decoder.push_slice(&[]), DecodeEvent::NeedMore);
    assert_eq!(decoder.buffered_len(), 0);
}
