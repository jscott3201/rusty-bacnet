//! Integration tests for the MS/TP wire codec.

use bacnet_mstp_codec::{
    crc16, crc16_accumulate_all, crc16_valid, crc8, crc8_accumulate_all, crc8_valid, decode_frame,
    encode_into, DecodeEvent, Frame, FrameError, FrameHeader, FrameSink, FrameType, StreamDecoder,
    StreamPhase, DATA_CRC_RESIDUAL, HEADER_CRC_RESIDUAL, MAX_STANDARD_MPDU_DATA, PREAMBLE,
};
use proptest::prelude::*;

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
        assert_eq!(crc8_accumulate_all(&bytes), HEADER_CRC_RESIDUAL);
    }
    assert_eq!(crc16(&[0x01, 0x00]), 0x169F);
    assert!(crc16_valid(&[0x01, 0x00, 0x9F, 0x16]));
    assert_eq!(
        crc16_accumulate_all(&[0x01, 0x00, 0x9F, 0x16]),
        DATA_CRC_RESIDUAL
    );
    assert!(crc8_valid(&[crc8(&[])]));
}

#[test]
fn all_standard_types_round_trip() {
    for raw in 0..=7 {
        let data = if raw >= 3 && raw <= 6 {
            &[0x01, 0x22, 0x30][..]
        } else {
            &[]
        };
        let frame = Frame::new(FrameType::from_raw(raw), 10, 5, data);
        let mut wire = [0u8; 511];
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
    assert_eq!(frame.header.frame_type, FrameType::Token);
    assert_eq!(frame.data, &[]);
    let data = [
        0x55, 0xFF, 0x06, 0x00, 0x07, 0x00, 0x02, 0xD9, 0x01, 0x00, 0x9F, 0x16,
    ];
    let (frame, used) = decode_frame(&data).unwrap();
    assert_eq!(used, data.len());
    assert_eq!(frame.data, &[0x01, 0x00]);
}

#[test]
fn truncation_oversize_and_corruption_are_typed() {
    let frame = Frame::new(FrameType::BACnetDataNotExpectingReply, 0, 7, &[1, 2, 3]);
    let mut wire = [0u8; 511];
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
    let too_long = [0u8; 502];
    assert_eq!(
        Frame::new(FrameType::Token, 0, 0, &too_long).encode_into(&mut wire),
        Err(FrameError::EncodeOversize {
            length: 502,
            maximum: MAX_STANDARD_MPDU_DATA
        })
    );
}

#[test]
fn every_reserved_type_is_rejected() {
    for raw in 32..=127 {
        let header = [raw, 0, 1, 0, 0];
        let mut wire = [0u8; 8];
        wire[..2].copy_from_slice(&PREAMBLE);
        wire[2..7].copy_from_slice(&header);
        wire[7] = crc8(&header);
        assert_eq!(
            decode_frame(&wire),
            Err(FrameError::UnsupportedCobsFrame { raw_type: raw })
        );
        assert_eq!(
            Frame::new(FrameType::from_raw(raw), 0, 1, &[]).encode_into(&mut wire),
            Err(FrameError::EncodeUnsupportedCobsFrame { raw_type: raw })
        );
    }
}

#[test]
fn stream_resync_and_bad_data_crc_discard() {
    let frame = Frame::new(FrameType::BACnetDataNotExpectingReply, 0, 7, &[1, 2]);
    let mut wire = [0u8; 511];
    let used = frame.encode_into(&mut wire).unwrap();
    wire[used - 1] ^= 0x80;
    let mut storage = [0u8; 32];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert_eq!(
        decoder.push_slice(&wire[..used]),
        DecodeEvent::Malformed {
            discard: used,
            error: FrameError::DataCrcMismatch { data_length: 2 },
        }
    );
    let mut valid = [0u8; 511];
    let valid_len = frame.encode_into(&mut valid).unwrap();
    assert_eq!(
        decoder.push_slice(&[9]),
        DecodeEvent::Invalid { discard: 1 }
    );
    assert!(matches!(
        decoder.push_slice(&valid[..valid_len]),
        DecodeEvent::Frame { .. }
    ));
    assert_eq!(decoder.buffered_len(), valid_len);
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

    let mut encoded = [0; 64];
    let length = Frame::new(FrameType::BACnetDataNotExpectingReply, 1, 2, &[9])
        .encode_into(&mut encoded)
        .unwrap();
    decoder.reset();
    assert_eq!(decoder.push_slice(&encoded[..8]), DecodeEvent::NeedMore);
    assert_eq!(decoder.phase(), StreamPhase::Data);

    decoder.reset();
    encoded[7] ^= 0x01;
    assert!(matches!(
        decoder.push_slice(&encoded[..length]),
        DecodeEvent::Malformed { .. }
    ));
    assert_eq!(decoder.phase(), StreamPhase::Sync);
}

#[test]
fn sink_exhaustion_and_slice_sink() {
    let frame = Frame::new(FrameType::Token, 0, 7, &[]);
    let mut short = [0xA5; 7];
    let mut sink = &mut short[..];
    assert!(matches!(
        sink.encode(frame),
        Err(FrameError::BufferExhausted { .. })
    ));
    assert_eq!(short, [0xA5; 7]);
    let mut output = [0u8; 8];
    let mut sink = &mut output[..];
    assert_eq!(sink.encode(frame).unwrap(), 8);
    assert_eq!(&output[..2], &PREAMBLE);
}

#[test]
fn cobs_error_precedes_oversize_error() {
    let oversized = [0u8; MAX_STANDARD_MPDU_DATA + 1];
    let frame = Frame::new(FrameType::Unknown(0x20), 0, 7, &oversized);
    let mut output = [0u8; 511];
    assert_eq!(
        frame.encode_into(&mut output),
        Err(FrameError::EncodeUnsupportedCobsFrame { raw_type: 0x20 })
    );
    let mut sink = &mut output[..];
    assert_eq!(
        sink.encode(frame),
        Err(FrameError::EncodeUnsupportedCobsFrame { raw_type: 0x20 })
    );
}

#[cfg(feature = "std")]
#[test]
fn bytes_mut_sink_appends_without_replacing_prefix() {
    let frame = Frame::new(FrameType::Token, 0, 7, &[]);
    let mut output = bytes::BytesMut::from(&b"prefix"[..]);
    assert_eq!(output.encode(frame).unwrap(), 8);
    assert_eq!(&output[..6], b"prefix");
    assert_eq!(&output[6..8], &PREAMBLE);
}

#[test]
fn header_type_can_be_encoded_directly() {
    let header = FrameHeader::new(FrameType::Token, 0x10, 5, 0);
    let mut output = [0u8; 8];
    assert_eq!(encode_into(&mut output, &header, &[]).unwrap(), 8);
}

#[cfg(feature = "alloc")]
#[test]
fn legacy_error_conversion_keeps_offsets_and_messages() {
    let error: bacnet_types::error::Error = FrameError::DataCrcMismatch { data_length: 2 }.into();
    match error {
        bacnet_types::error::Error::Decoding {
            offset, message, ..
        } => {
            assert_eq!(offset, 10);
            assert_eq!(message, "MS/TP data CRC mismatch");
        }
        other => panic!("unexpected error: {other:?}"),
    }
    let error: bacnet_types::error::Error = FrameError::EncodeOversize {
        length: 502,
        maximum: 501,
    }
    .into();
    assert!(
        matches!(error, bacnet_types::error::Error::Encoding(message) if message == "MS/TP data length 502 exceeds maximum 501")
    );
}

#[test]
fn stream_retains_suffix_after_two_frames_in_one_slice() {
    let first = Frame::new(FrameType::Token, 0, 7, &[]);
    let second = Frame::new(FrameType::PollForMaster, 7, 0, &[]);
    let mut wire = [0u8; 16];
    let first_len = first.encode_into(&mut wire[..8]).unwrap();
    let second_len = second.encode_into(&mut wire[8..]).unwrap();
    assert_eq!((first_len, second_len), (8, 8));
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
}

#[test]
fn stream_exhaustion_preserves_state_and_recovers() {
    let token = Frame::new(FrameType::Token, 0, 7, &[]);
    let mut wire = [0u8; 8];
    token.encode_into(&mut wire).unwrap();
    let mut storage = [0u8; 8];
    let mut decoder = StreamDecoder::new(&mut storage);
    assert!(matches!(
        decoder.push_slice(&[0u8; 9]),
        DecodeEvent::Error(FrameError::BufferExhausted {
            needed: 9,
            available: 8
        })
    ));
    assert_eq!(decoder.buffered_len(), 0);
    assert!(matches!(
        decoder.push_slice(&wire),
        DecodeEvent::Frame { consumed: 8, .. }
    ));
}

proptest! {
    #[test]
    fn encoded_frames_only_decode_when_complete(payload in prop::collection::vec(any::<u8>(), 0..=501)) {
        let frame = Frame::new(FrameType::BACnetDataNotExpectingReply, 0, 7, &payload);
        let mut wire = [0u8; 511];
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
        let frame = Frame::new(FrameType::BACnetDataNotExpectingReply, 0, 7, &payload);
        let mut wire = [0u8; 511];
        let used = frame.encode_into(&mut wire).unwrap();
        let index = index % used;
        wire[index] ^= 1;
        prop_assert!(decode_frame(&wire[..used]).is_err());
    }

    #[test]
    fn sink_capacity_is_never_overrun(capacity in 0usize..512) {
        let frame = Frame::new(FrameType::Token, 0, 7, &[]);
        let mut storage = [0u8; 511];
        let available = capacity.min(storage.len());
        let mut sink = &mut storage[..available];
        let result = sink.encode(frame);
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
            FrameType::BACnetDataNotExpectingReply.to_raw(),
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

    #[test]
    fn byte_stream_resynchronizes_then_decodes(
        garbage in prop::collection::vec(0u8..=0x54, 0..64),
        payload in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let frame = Frame::new(FrameType::BACnetDataNotExpectingReply, 0, 7, &payload);
        let mut wire = [0u8; 511];
        let used = frame.encode_into(&mut wire).unwrap();
        let mut storage = [0u8; 511];
        let mut decoder = StreamDecoder::new(&mut storage);
        for byte in garbage {
            prop_assert_eq!(decoder.push(byte), DecodeEvent::Invalid { discard: 1 });
        }
        for (index, byte) in wire[..used].iter().copied().enumerate() {
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
}

#[cfg(feature = "heapless")]
#[test]
fn heapless_sink() {
    let frame = Frame::new(FrameType::Token, 0, 7, &[]);
    let mut output = heapless::Vec::<u8, 8>::new();
    assert_eq!(output.encode(frame).unwrap(), 8);
    assert_eq!(output.len(), 8);
}
