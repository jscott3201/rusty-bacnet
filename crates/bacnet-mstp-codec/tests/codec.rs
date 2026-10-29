//! Integration tests for one-shot MS/TP frame encode and decode, CRCs and sinks.

use bacnet_mstp_codec::crc::{crc8, crc8_valid, crc16, crc16_valid};
use bacnet_mstp_codec::{
    Frame, FrameError, FrameHeader, FrameSink, FrameType, MAX_STANDARD_FRAME_LENGTH,
    MAX_STANDARD_MPDU_DATA, PREAMBLE, SinkFull, decode_frame, find_preamble,
};
use proptest::prelude::*;

mod support;
use support::{DATA_CRC_RESIDUAL, HEADER_CRC_RESIDUAL};

/// Build a frame by hand, with valid CRCs, bypassing the encoder's checks.
fn raw_frame(raw_type: u8, destination: u8, source: u8, data: &[u8]) -> Vec<u8> {
    let length = u16::try_from(data.len()).unwrap().to_be_bytes();
    let fields = [raw_type, destination, source, length[0], length[1]];
    let mut wire = PREAMBLE.to_vec();
    wire.extend_from_slice(&fields);
    wire.push(crc8(&fields));
    if !data.is_empty() {
        wire.extend_from_slice(data);
        wire.extend_from_slice(&crc16(data).to_le_bytes());
    }
    wire
}

/// A header alone, with a correct CRC, for field values encode refuses.
fn raw_header(raw_type: u8, destination: u8, source: u8, length: u16) -> Vec<u8> {
    let [high, low] = length.to_be_bytes();
    let fields = [raw_type, destination, source, high, low];
    let mut wire = PREAMBLE.to_vec();
    wire.extend_from_slice(&fields);
    wire.push(crc8(&fields));
    wire
}

#[test]
fn golden_crc_vectors() {
    let headers = [
        ([0x00, 0x00, 0x07, 0x00, 0x00], 0x37),
        ([0x00, 0x07, 0x00, 0x00, 0x00], 0x40),
        ([0x01, 0x00, 0x07, 0x00, 0x00], 0xB1),
        ([0x01, 0x07, 0x00, 0x00, 0x00], 0xC6),
    ];
    for (header, expected) in headers {
        assert_eq!(crc8(&header), expected);
        let mut bytes = header.to_vec();
        bytes.push(expected);
        assert!(crc8_valid(&bytes));
        assert_eq!(!crc8(&bytes), HEADER_CRC_RESIDUAL);
    }
    // Computed here, not taken from the standard; the frames below use it.
    assert_eq!(crc16(&[0x01, 0x00]), 0x169F);
    assert!(crc16_valid(&[0x01, 0x00, 0x9F, 0x16]));
    assert_eq!(!crc16(&[0x01, 0x00, 0x9F, 0x16]), DATA_CRC_RESIDUAL);
    assert!(crc8_valid(&[crc8(&[])]));
}

/// The worked header CRC example of Annex G.1: a Token from station 0x05 to
/// station 0x10.
#[test]
fn annex_g_token_example() {
    let fields = [0x00, 0x10, 0x05, 0x00, 0x00];
    // The CRC register after each header octet, before the final complement.
    let registers = [0x55, 0xC2, 0xBC, 0x95, 0x73];
    for (count, register) in registers.into_iter().enumerate() {
        assert_eq!(!crc8(&fields[..=count]), register);
    }
    assert_eq!(crc8(&fields), 0x8C);
    assert_eq!(
        !crc8(&[0x00, 0x10, 0x05, 0x00, 0x00, 0x8C]),
        HEADER_CRC_RESIDUAL
    );

    let expected = [0x55, 0xFF, 0x00, 0x10, 0x05, 0x00, 0x00, 0x8C];
    let token = Frame::new(FrameType::TOKEN, 0x10, 0x05, &[]);
    let mut wire = [0u8; 8];
    assert_eq!(token.encode_into(&mut wire), Ok(8));
    assert_eq!(wire, expected);
    assert_eq!(decode_frame(&expected), Ok((token, 8)));
}

/// The worked data CRC example of Annex G.2: data octets 01 22 30.
#[test]
fn annex_g_data_crc_example() {
    let data = [0x01, 0x22, 0x30];
    // The CRC register after each data octet, before the final complement.
    let registers = [0x1E0E, 0xEB70, 0x42EF];
    for (count, register) in registers.into_iter().enumerate() {
        assert_eq!(!crc16(&data[..=count]), register);
    }
    assert_eq!(crc16(&data), 0xBD10);
    // Sent least significant octet first; the receiver's register after each.
    let received = [0x01, 0x22, 0x30, 0x10, 0xBD];
    assert_eq!(!crc16(&received[..4]), 0x0F3A);
    assert_eq!(!crc16(&received), DATA_CRC_RESIDUAL);
    assert!(crc16_valid(&received));

    let frame = Frame::new(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        0x10,
        0x05,
        &data,
    );
    let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
    let used = frame.encode_into(&mut wire).unwrap();
    assert_eq!(&wire[8..used], &received);
}

#[test]
fn all_standard_types_round_trip() {
    for raw in 0..=7 {
        let frame_type = FrameType::from_raw(raw);
        let data = if frame_type.allows_data() {
            &[0x01, 0x22, 0x30][..]
        } else {
            &[]
        };
        let frame = Frame::new(frame_type, 10, 5, data);
        let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
        let used = frame.encode_into(&mut wire).unwrap();
        let (decoded, consumed) = decode_frame(&wire[..used]).unwrap();
        assert_eq!(used, consumed);
        assert_eq!(decoded, frame);
    }
}

#[test]
fn standard_golden_frames() {
    let token = [0x55, 0xFF, 0x00, 0x00, 0x07, 0x00, 0x00, 0x37];
    let (frame, used) = decode_frame(&token).unwrap();
    assert_eq!(used, token.len());
    assert_eq!(frame.header.frame_type, FrameType::TOKEN);
    assert_eq!(frame.data, &[]);
    let data = [
        0x55, 0xFF, 0x06, 0x00, 0x07, 0x00, 0x02, 0xD9, 0x01, 0x00, 0x9F, 0x16,
    ];
    let (frame, used) = decode_frame(&data).unwrap();
    assert_eq!(used, data.len());
    assert_eq!(frame.data, &[0x01, 0x00]);
}

/// Clause 9.3 allows slaves the addresses 128-254, so a raw decode accepts
/// them as sources, and the encoder can produce them.
#[test]
fn slave_range_sources_decode_and_round_trip() {
    // Data Not Expecting Reply from 200 (0xC8) to station 1, data 01 00.
    let seed = [
        0x55, 0xFF, 0x06, 0x01, 0xC8, 0x00, 0x02, 0xEF, 0x01, 0x00, 0x9F, 0x16,
    ];
    let (frame, used) = decode_frame(&seed).unwrap();
    assert_eq!(used, seed.len());
    assert_eq!(
        frame,
        Frame::new(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            1,
            200,
            &[0x01, 0x00]
        )
    );

    for source in [128, 200, 254] {
        let response = Frame::new(FrameType::TEST_RESPONSE, 3, source, &[0xAA, 0x55]);
        let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
        let used = response.encode_into(&mut wire).unwrap();
        assert_eq!(wire[4], source);
        assert_eq!(decode_frame(&wire[..used]), Ok((response, used)));
        let hand_built = raw_frame(0x04, 3, source, &[0xAA, 0x55]);
        assert_eq!(&wire[..used], &hand_built[..]);
    }
}

#[test]
fn broadcast_source_is_refused_both_ways() {
    for raw_type in [0x00, 0x04, 0x06] {
        let data: &[u8] = if raw_type == 0 { &[] } else { &[1, 2] };
        let wire = raw_frame(raw_type, 3, 0xFF, data);
        assert_eq!(decode_frame(&wire), Err(FrameError::BroadcastSource));

        let frame = Frame::new(FrameType::from_raw(raw_type), 3, 0xFF, data);
        let mut output = [0xA5u8; MAX_STANDARD_FRAME_LENGTH];
        assert_eq!(
            frame.encode_into(&mut output),
            Err(FrameError::BroadcastSource)
        );
        assert!(output.iter().all(|&byte| byte == 0xA5));
        let mut sink = &mut output[..];
        assert_eq!(
            frame.encode_to_sink(&mut sink),
            Err(FrameError::BroadcastSource)
        );
    }
}

/// Clauses 9.3.1-9.3.3 and 9.3.8: these types carry no data. Encode refuses
/// it; a raw decode reports what arrived.
#[test]
fn no_data_types_refuse_data_on_encode_but_decode_it() {
    for frame_type in [
        FrameType::TOKEN,
        FrameType::POLL_FOR_MASTER,
        FrameType::REPLY_TO_POLL_FOR_MASTER,
        FrameType::REPLY_POSTPONED,
    ] {
        assert!(!frame_type.allows_data());
        let raw_type = frame_type.to_raw();
        let mut output = [0xA5u8; MAX_STANDARD_FRAME_LENGTH];
        assert_eq!(
            Frame::new(frame_type, 3, 4, &[9]).encode_into(&mut output),
            Err(FrameError::UnexpectedData {
                raw_type,
                length: 1
            })
        );
        assert!(output.iter().all(|&byte| byte == 0xA5));
        assert_eq!(
            Frame::new(frame_type, 3, 4, &[]).encode_into(&mut output),
            Ok(8)
        );

        let wire = raw_frame(raw_type, 3, 4, &[9]);
        let (decoded, used) = decode_frame(&wire).unwrap();
        assert_eq!(used, wire.len());
        assert_eq!(decoded.header.frame_type, FrameType::from_raw(raw_type));
        assert_eq!(decoded.data, [9]);
    }
    for raw_type in [3, 4, 5, 6, 8, 31, 128, 255] {
        let frame_type = FrameType::from_raw(raw_type);
        assert!(frame_type.allows_data());
        let mut output = [0u8; MAX_STANDARD_FRAME_LENGTH];
        let used = Frame::new(frame_type, 3, 4, &[9, 8])
            .encode_into(&mut output)
            .unwrap();
        assert_eq!(&output[..used], &raw_frame(raw_type, 3, 4, &[9, 8])[..]);
    }
}

/// Clause 9.3.11: proprietary data opens with a 2-octet vendor identifier.
/// Encode refuses less; a raw decode reports what arrived. Reserved types
/// 8-31 are not policed either way.
#[test]
fn proprietary_types_need_a_vendor_id_on_encode() {
    for raw_type in [0x80, 0xC3, 0xFF] {
        let frame_type = FrameType::from_raw(raw_type);
        assert!(frame_type.is_proprietary());
        for data in [&[][..], &[0x01][..]] {
            let mut output = [0xA5u8; MAX_STANDARD_FRAME_LENGTH];
            assert_eq!(
                Frame::new(frame_type, 3, 4, data).encode_into(&mut output),
                Err(FrameError::MissingVendorId {
                    raw_type,
                    length: data.len()
                })
            );
            assert!(output.iter().all(|&byte| byte == 0xA5));
            let wire = raw_frame(raw_type, 3, 4, data);
            assert_eq!(decode_frame(&wire).unwrap().0.data, data);
        }
        let vendor = [0x01, 0x04];
        let mut output = [0u8; MAX_STANDARD_FRAME_LENGTH];
        let used = Frame::new(frame_type, 3, 4, &vendor)
            .encode_into(&mut output)
            .unwrap();
        assert_eq!(&output[..used], &raw_frame(raw_type, 3, 4, &vendor)[..]);
    }
    for raw_type in [0x08, 0x1F] {
        let frame_type = FrameType::from_raw(raw_type);
        assert!(!frame_type.is_proprietary());
        let mut output = [0u8; MAX_STANDARD_FRAME_LENGTH];
        assert_eq!(
            Frame::new(frame_type, 3, 4, &[]).encode_into(&mut output),
            Ok(8)
        );
        assert_eq!(
            Frame::new(frame_type, 3, 4, &[7]).encode_into(&mut output),
            Ok(11)
        );
    }
    assert!(!FrameType::from_raw(0x7F).is_proprietary());
}

#[test]
fn truncation_oversize_and_corruption_are_typed() {
    let frame = Frame::new(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 0, 7, &[1, 2, 3]);
    let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
    let used = frame.encode_into(&mut wire).unwrap();
    assert!(matches!(
        decode_frame(&wire[..used - 1]),
        Err(FrameError::Truncated { .. })
    ));
    wire[7] ^= 1;
    assert_eq!(
        decode_frame(&wire[..used]),
        Err(FrameError::HeaderCrcMismatch)
    );
    let too_long = [0u8; MAX_STANDARD_MPDU_DATA + 1];
    assert_eq!(
        Frame::new(FrameType::TEST_REQUEST, 0, 0, &too_long).encode_into(&mut wire),
        Err(FrameError::Oversize {
            length: 502,
            maximum: MAX_STANDARD_MPDU_DATA
        })
    );
    let largest = [0u8; MAX_STANDARD_MPDU_DATA];
    assert_eq!(
        Frame::new(FrameType::TEST_REQUEST, 0, 0, &largest).encode_into(&mut wire),
        Ok(MAX_STANDARD_FRAME_LENGTH)
    );
}

/// The reserved COBS types, 34-127, are refused both ways once the rest of
/// the header has passed; 32 and 33 are the two the codec handles.
#[test]
fn reserved_cobs_types_are_rejected_after_the_header_checks() {
    for raw in 34..=127 {
        // A header with a Length inside the COBS bounds: good but unsupported,
        // which shows before the missing body would.
        assert_eq!(
            decode_frame(&raw_header(raw, 0, 1, 5)),
            Err(FrameError::UnsupportedCobsFrame { raw_type: raw })
        );
        let mut output = [0xA5u8; 64];
        assert_eq!(
            Frame::new(FrameType::from_raw(raw), 0, 1, &[1]).encode_into(&mut output),
            Err(FrameError::UnsupportedCobsFrame { raw_type: raw })
        );
        assert!(output.iter().all(|&octet| octet == 0xA5));
    }
    assert!(FrameType::from_raw(0x22).is_cobs());
    assert_eq!(
        FrameType::from_raw(0x20),
        FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY
    );
    assert_eq!(
        FrameType::from_raw(0x21),
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
    );
}

/// Header checks run in one order both ways (Clause 9.5.8, #1611): source
/// 255, then the Length against its class's bounds, then the COBS type. So
/// `UnsupportedCobsFrame` only ever comes with a good header.
#[test]
fn header_checks_run_source_then_length_then_type() {
    // Decode. A reserved COBS type with source 255 and a Length of 0 fails
    // on the source; with a good source, on the Length; and with a good
    // Length, on the type.
    assert_eq!(
        decode_frame(&raw_header(0x22, 3, 0xFF, 0)),
        Err(FrameError::BroadcastSource)
    );
    for length in [0, 4, 2044, u16::MAX] {
        assert_eq!(
            decode_frame(&raw_header(0x22, 3, 4, length)),
            Err(FrameError::InvalidCobsLength {
                length: usize::from(length)
            }),
            "Length {length}"
        );
    }
    assert!(matches!(
        decode_frame(&raw_header(0x22, 3, 4, 5)),
        Err(FrameError::UnsupportedCobsFrame { raw_type: 0x22 })
    ));
    // A standard type is held to 501 instead, after the source.
    assert_eq!(
        decode_frame(&raw_header(0x06, 3, 0xFF, 502)),
        Err(FrameError::BroadcastSource)
    );
    assert_eq!(
        decode_frame(&raw_header(0x06, 3, 4, 502)),
        Err(FrameError::Oversize {
            length: 502,
            maximum: MAX_STANDARD_MPDU_DATA
        })
    );

    // Encode, the same order: a reserved COBS type with no data has a
    // Length of 4, too short.
    let mut output = [0u8; MAX_STANDARD_FRAME_LENGTH];
    let reserved = FrameType::from_raw(0x22);
    assert_eq!(
        Frame::new(reserved, 3, 0xFF, &[]).encode_into(&mut output),
        Err(FrameError::BroadcastSource)
    );
    assert_eq!(
        Frame::new(reserved, 3, 4, &[]).encode_into(&mut output),
        Err(FrameError::InvalidCobsLength { length: 4 })
    );
    assert_eq!(
        Frame::new(reserved, 3, 4, &[1]).encode_into(&mut output),
        Err(FrameError::UnsupportedCobsFrame { raw_type: 0x22 })
    );
    let oversized = [0u8; MAX_STANDARD_MPDU_DATA + 1];
    assert_eq!(
        Frame::new(FrameType::TEST_REQUEST, 3, 0xFF, &oversized).encode_into(&mut output),
        Err(FrameError::BroadcastSource)
    );
}

#[test]
fn sink_exhaustion_and_slice_sink() {
    let frame = Frame::new(FrameType::TOKEN, 0, 7, &[]);
    let mut short = [0xA5; 7];
    let mut sink = &mut short[..];
    assert_eq!(
        frame.encode_to_sink(&mut sink),
        Err(FrameError::BufferExhausted {
            needed: 8,
            available: 7
        })
    );
    assert_eq!(short, [0xA5; 7]);
    let mut output = [0u8; 10];
    let mut sink = &mut output[..];
    assert_eq!(frame.encode_to_sink(&mut sink), Ok(8));
    // The slice sink advances past what it wrote.
    assert_eq!(sink.len(), 2);
    assert_eq!(&output[..2], &PREAMBLE);
}

#[test]
fn header_and_data_build_a_frame() {
    let header = FrameHeader::new(FrameType::TOKEN, 0x10, 5);
    let frame = Frame { header, data: &[] };
    let mut output = [0u8; 8];
    assert_eq!(frame.encode_into(&mut output), Ok(8));
    assert_eq!(decode_frame(&output).unwrap().0.header, header);
}

#[test]
fn find_preamble_locates_the_first_pair() {
    assert_eq!(find_preamble(&[0x00, 0x55, 0x55, 0xFF, 0x00]), Some(2));
    assert_eq!(find_preamble(&[0x55]), None);
    assert_eq!(find_preamble(&[]), None);
}

#[test]
fn frame_error_is_a_core_error() {
    fn describe(error: &dyn core::error::Error) -> String {
        error.to_string()
    }
    assert_eq!(
        describe(&FrameError::BroadcastSource),
        "source address is broadcast (0xFF)"
    );
    assert!(
        describe(&FrameError::FrameTooLong {
            needed: 13,
            capacity: 12
        })
        .contains("13")
    );
}

/// A sink that accepts a fixed number of writes, then fails. It reports no
/// capacity, like a growable sink, so encoding cannot check up front.
struct FailingSink {
    written: Vec<u8>,
    writes_left: usize,
}

impl FrameSink for FailingSink {
    fn remaining_capacity(&self) -> Option<usize> {
        None
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        if self.writes_left == 0 {
            return Err(SinkFull {
                needed: bytes.len(),
                available: 0,
            });
        }
        self.writes_left -= 1;
        self.written.extend_from_slice(bytes);
        Ok(())
    }
}

#[test]
fn growable_sink_failing_mid_frame_keeps_the_prefix() {
    let mut sink = FailingSink {
        written: Vec::new(),
        writes_left: 1,
    };
    let frame = Frame::new(FrameType::TEST_REQUEST, 1, 2, &[7, 7, 7]);
    // The failing write is the data's, three octets into no room: its
    // SinkFull comes back field for field.
    assert_eq!(
        frame.encode_to_sink(&mut sink),
        Err(FrameError::BufferExhausted {
            needed: 3,
            available: 0
        })
    );
    assert_eq!(sink.written.len(), 8);
    assert_eq!(&sink.written[..2], &PREAMBLE);
}

#[test]
fn sink_full_becomes_buffer_exhausted_field_for_field() {
    let full = SinkFull {
        needed: 5,
        available: 2,
    };
    assert_eq!(
        FrameError::from(full),
        FrameError::BufferExhausted {
            needed: 5,
            available: 2
        }
    );
    assert_eq!(full.to_string(), "sink full: need 5, have 2");

    // A slice refuses a write longer than its room and keeps its room.
    let mut storage = [0u8; 2];
    let mut sink: &mut [u8] = &mut storage;
    assert_eq!(sink.write(&[1, 2, 3, 4, 5]), Err(full));
    assert_eq!(sink.remaining_capacity(), Some(2));
}

#[cfg(feature = "alloc")]
#[test]
fn vec_sink_appends_without_replacing_prefix() {
    let frame = Frame::new(FrameType::TOKEN, 0, 7, &[]);
    let mut output = b"prefix".to_vec();
    assert_eq!(frame.encode_to_sink(&mut output), Ok(8));
    assert_eq!(&output[..6], b"prefix");
    assert_eq!(&output[6..8], &PREAMBLE);
}

#[cfg(feature = "bytes")]
#[test]
fn bytes_mut_sink_appends_without_replacing_prefix() {
    let frame = Frame::new(FrameType::TOKEN, 0, 7, &[]);
    let mut output = bytes::BytesMut::from(&b"prefix"[..]);
    assert_eq!(frame.encode_to_sink(&mut output), Ok(8));
    assert_eq!(&output[..6], b"prefix");
    assert_eq!(&output[6..8], &PREAMBLE);
}

#[cfg(feature = "heapless")]
#[test]
fn heapless_sink() {
    let frame = Frame::new(FrameType::TOKEN, 0, 7, &[]);
    let mut output = heapless::Vec::<u8, 8>::new();
    assert_eq!(frame.encode_to_sink(&mut output), Ok(8));
    assert_eq!(output.len(), 8);
    let mut short = heapless::Vec::<u8, 7>::new();
    assert_eq!(
        frame.encode_to_sink(&mut short),
        Err(FrameError::BufferExhausted {
            needed: 8,
            available: 7
        })
    );
    assert!(short.is_empty());
}

proptest! {
    #[test]
    fn encoded_frames_only_decode_when_complete(payload in prop::collection::vec(any::<u8>(), 0..=501)) {
        let frame = Frame::new(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 0, 7, &payload);
        let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
        let used = frame.encode_into(&mut wire).unwrap();
        for end in 0..used {
            prop_assert!(decode_frame(&wire[..end]).is_err());
        }
        prop_assert_eq!(decode_frame(&wire[..used]).unwrap().1, used);
    }

    #[test]
    fn one_byte_corruptions_are_rejected(
        payload in prop::collection::vec(any::<u8>(), 1..=64),
        index in 0usize..80,
    ) {
        let frame = Frame::new(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 0, 7, &payload);
        let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
        let used = frame.encode_into(&mut wire).unwrap();
        let index = index % used;
        wire[index] ^= 1;
        prop_assert!(decode_frame(&wire[..used]).is_err());
    }

    #[test]
    fn any_source_but_broadcast_round_trips(
        source in any::<u8>(),
        destination in any::<u8>(),
        payload in prop::collection::vec(any::<u8>(), 0..32),
    ) {
        let frame = Frame::new(FrameType::BACNET_DATA_EXPECTING_REPLY, destination, source, &payload);
        let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
        let encoded = frame.encode_into(&mut wire);
        if source == 0xFF {
            prop_assert_eq!(encoded, Err(FrameError::BroadcastSource));
        } else {
            let used = encoded.unwrap();
            prop_assert_eq!(decode_frame(&wire[..used]), Ok((frame, used)));
        }
    }

    #[test]
    fn sink_capacity_is_never_overrun(capacity in 0usize..512) {
        let frame = Frame::new(FrameType::TOKEN, 0, 7, &[]);
        let mut storage = [0u8; 511];
        let available = capacity.min(storage.len());
        let mut sink = &mut storage[..available];
        let result = frame.encode_to_sink(&mut sink);
        if available < 8 {
            let exhausted = matches!(result, Err(FrameError::BufferExhausted { .. }));
            prop_assert!(exhausted);
        } else {
            prop_assert_eq!(result.unwrap(), 8);
        }
    }

    #[test]
    fn declared_oversize_wire_lengths_are_rejected(length in 502u16..=u16::MAX) {
        let fields = [
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY.to_raw(),
            0,
            7,
            (length >> 8) as u8,
            length as u8,
        ];
        let mut wire = [0u8; 8];
        wire[..2].copy_from_slice(&PREAMBLE);
        wire[2..7].copy_from_slice(&fields);
        wire[7] = crc8(&fields);
        prop_assert_eq!(
            decode_frame(&wire),
            Err(FrameError::Oversize {
                length: usize::from(length),
                maximum: MAX_STANDARD_MPDU_DATA,
            })
        );
    }
}
