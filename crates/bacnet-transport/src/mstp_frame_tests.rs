use super::*;

// -----------------------------------------------------------------------
// CRC tests — Clause 9.6 golden vectors (literal expected bytes; not from crc8/crc16)
// -----------------------------------------------------------------------

#[test]
fn crc8_clause9_header_vectors() {
    // [frame_type, dest, src, len_hi, len_lo] -> header CRC
    let vectors: &[([u8; 5], u8)] = &[
        ([0x00, 0x00, 0x07, 0x00, 0x00], 0x37),
        ([0x00, 0x07, 0x00, 0x00, 0x00], 0x40),
        ([0x01, 0x00, 0x07, 0x00, 0x00], 0xB1),
        ([0x01, 0x07, 0x00, 0x00, 0x00], 0xC6),
    ];
    for (header, expected) in vectors {
        assert_eq!(crc8(header), *expected, "header={header:02X?}");
        let mut with_crc = header.to_vec();
        with_crc.push(*expected);
        assert!(crc8_valid(&with_crc));
        assert_eq!(crc8_accumulate_all(&with_crc), HEADER_CRC_RESIDUAL);
    }
}

#[test]
fn crc8_one_bit_corruption_rejected() {
    let header = [0x00, 0x00, 0x07, 0x00, 0x00];
    let mut with_crc = header.to_vec();
    with_crc.push(0x37);
    assert!(crc8_valid(&with_crc));
    with_crc[0] ^= 0x01;
    assert!(!crc8_valid(&with_crc));
    assert_ne!(crc8_accumulate_all(&with_crc), HEADER_CRC_RESIDUAL);
}

#[test]
fn crc16_clause9_data_vector_01_00() {
    // Data 01 00 → CRC 0x169F, wire order LS first: 9F 16
    assert_eq!(crc16(&[0x01, 0x00]), 0x169F);
    let with_crc = [0x01, 0x00, 0x9F, 0x16];
    assert!(crc16_valid(&with_crc));
    assert_eq!(crc16_accumulate_all(&with_crc), DATA_CRC_RESIDUAL);
}

#[test]
fn crc16_annex_g_data_vector() {
    // Annex G: data 01 22 30 has complemented CRC 0xBD10,
    // transmitted least-significant octet first as 10 BD.
    assert_eq!(crc16(&[0x01, 0x22, 0x30]), 0xBD10);
    let with_crc = [0x01, 0x22, 0x30, 0x10, 0xBD];
    assert!(crc16_valid(&with_crc));
    assert_eq!(crc16_accumulate_all(&with_crc), DATA_CRC_RESIDUAL);
}

#[test]
fn crc16_one_bit_corruption_rejected() {
    let mut with_crc = [0x01, 0x00, 0x9F, 0x16];
    assert!(crc16_valid(&with_crc));
    with_crc[0] ^= 0x01;
    assert!(!crc16_valid(&with_crc));
    assert_ne!(crc16_accumulate_all(&with_crc), DATA_CRC_RESIDUAL);
}

#[test]
fn crc8_validate_round_trip() {
    let data = [0x05, 0xFF, 0x03, 0x00, 0x0C];
    let crc = crc8(&data);
    let mut with_crc = data.to_vec();
    with_crc.push(crc);
    assert!(crc8_valid(&with_crc));
    assert_eq!(crc8_accumulate_all(&with_crc), HEADER_CRC_RESIDUAL);
}

#[test]
fn crc16_validate_round_trip() {
    let data = vec![0xAA; 100];
    let crc = crc16(&data);
    let mut with_crc = data;
    with_crc.push(crc as u8);
    with_crc.push((crc >> 8) as u8);
    assert!(crc16_valid(&with_crc));
    assert_eq!(crc16_accumulate_all(&with_crc), DATA_CRC_RESIDUAL);
}

#[test]
fn literal_token_frame_0_from_7() {
    // Live trunk Token: dest BASRT(0) <- FEC(7), header CRC 0x37
    let wire: &[u8] = &[0x55, 0xFF, 0x00, 0x00, 0x07, 0x00, 0x00, 0x37];
    let (frame, consumed) = decode_frame(wire).expect("decode Token 0<-7");
    assert_eq!(consumed, 8);
    assert_eq!(frame.frame_type, FrameType::Token);
    assert_eq!(frame.destination, 0);
    assert_eq!(frame.source, 7);
    assert!(frame.data.is_empty());

    let mut enc = BytesMut::new();
    encode_frame(&mut enc, &frame).unwrap();
    assert_eq!(&enc[..], wire);
}

#[test]
fn literal_annex_g_token_frame() {
    // Annex G: Token, DA 10, SA 05, length 0000, header CRC 8C.
    let wire: &[u8] = &[0x55, 0xFF, 0x00, 0x10, 0x05, 0x00, 0x00, 0x8C];
    let (frame, consumed) = decode_frame(wire).expect("decode Annex G Token");
    assert_eq!(consumed, wire.len());
    assert_eq!(frame.frame_type, FrameType::Token);
    assert_eq!(frame.destination, 0x10);
    assert_eq!(frame.source, 0x05);
    assert!(frame.data.is_empty());

    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &frame).unwrap();
    assert_eq!(&encoded[..], wire);
}

#[test]
fn literal_data_not_expecting_reply_frame() {
    // Frame type 06, dest 0, src 7, len 2, HDR D9, data 01 00, DCRC 9F 16
    let wire: &[u8] = &[
        0x55, 0xFF, 0x06, 0x00, 0x07, 0x00, 0x02, 0xD9, 0x01, 0x00, 0x9F, 0x16,
    ];
    let (frame, consumed) = decode_frame(wire).expect("decode data frame");
    assert_eq!(consumed, 12);
    assert_eq!(frame.frame_type, FrameType::BACnetDataNotExpectingReply);
    assert_eq!(frame.destination, 0);
    assert_eq!(frame.source, 7);
    assert_eq!(&frame.data[..], &[0x01, 0x00]);

    let mut enc = BytesMut::new();
    encode_frame(&mut enc, &frame).unwrap();
    assert_eq!(&enc[..], wire);
}

#[test]
fn truncated_header_need_more_or_err() {
    let partial = [0x55, 0xFF, 0x00, 0x00, 0x07];
    assert!(decode_frame(&partial).is_err());
    assert_eq!(decode_frame_stream(&partial), StreamDecode::NeedMore);
}

#[test]
fn truncated_data_crc_need_more() {
    // Header complete for len=2 but missing data CRC octets
    let partial = [0x55, 0xFF, 0x06, 0x00, 0x07, 0x00, 0x02, 0xD9, 0x01, 0x00];
    assert_eq!(decode_frame_stream(&partial), StreamDecode::NeedMore);
    assert!(decode_frame(&partial).is_err());
}

// -----------------------------------------------------------------------
// Frame encode/decode tests
// -----------------------------------------------------------------------

#[test]
fn token_frame_round_trip() {
    let frame = MstpFrame {
        frame_type: FrameType::Token,
        destination: 1,
        source: 0,
        data: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();

    // Token has no data, so: preamble(2) + header(5) + crc(1) = 8
    assert_eq!(buf.len(), 8);
    assert_eq!(&buf[..2], &PREAMBLE);

    let (decoded, consumed) = decode_frame(&buf).unwrap();
    assert_eq!(consumed, 8);
    assert_eq!(decoded, frame);
}

#[test]
fn poll_for_master_round_trip() {
    let frame = MstpFrame {
        frame_type: FrameType::PollForMaster,
        destination: 42,
        source: 0,
        data: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();

    let (decoded, _) = decode_frame(&buf).unwrap();
    assert_eq!(decoded, frame);
}

#[test]
fn data_expecting_reply_round_trip() {
    let npdu = vec![0x01, 0x00, 0x10, 0x02, 0x03, 0x04, 0x05];
    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: 5,
        source: 0,
        data: Bytes::from(npdu.clone()),
    };
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();

    // preamble(2) + header(5) + hcrc(1) + data(7) + dcrc(2) = 17
    assert_eq!(buf.len(), 17);

    let (decoded, consumed) = decode_frame(&buf).unwrap();
    assert_eq!(consumed, 17);
    assert_eq!(decoded.frame_type, FrameType::BACnetDataExpectingReply);
    assert_eq!(decoded.destination, 5);
    assert_eq!(decoded.source, 0);
    assert_eq!(decoded.data, npdu);
}

#[test]
fn data_not_expecting_reply_round_trip() {
    let npdu = vec![0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataNotExpectingReply,
        destination: BROADCAST_MAC,
        source: 3,
        data: Bytes::from(npdu.clone()),
    };
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();

    let (decoded, _) = decode_frame(&buf).unwrap();
    assert_eq!(decoded, frame);
}

#[test]
fn broadcast_destination() {
    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataNotExpectingReply,
        destination: BROADCAST_MAC,
        source: 10,
        data: Bytes::from_static(&[0x01, 0x00]),
    };
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();

    let (decoded, _) = decode_frame(&buf).unwrap();
    assert_eq!(decoded.destination, BROADCAST_MAC);
}

#[test]
fn reply_to_poll_for_master_round_trip() {
    let frame = MstpFrame {
        frame_type: FrameType::ReplyToPollForMaster,
        destination: 0,
        source: 42,
        data: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();

    let (decoded, _) = decode_frame(&buf).unwrap();
    assert_eq!(decoded, frame);
}

#[test]
fn test_request_with_data_round_trip() {
    let test_data = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let frame = MstpFrame {
        frame_type: FrameType::TestRequest,
        destination: 5,
        source: 0,
        data: Bytes::from(test_data.clone()),
    };
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();

    let (decoded, _) = decode_frame(&buf).unwrap();
    assert_eq!(decoded.data, test_data);
}

#[test]
fn decode_too_short() {
    assert!(decode_frame(&[0x55, 0xFF, 0x00]).is_err());
}

#[test]
fn decode_bad_preamble() {
    let data = [0x00, 0xFF, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];
    assert!(decode_frame(&data).is_err());
}

#[test]
fn decode_bad_header_crc() {
    let mut buf = BytesMut::new();
    let frame = MstpFrame {
        frame_type: FrameType::Token,
        destination: 1,
        source: 0,
        data: Bytes::new(),
    };
    encode_frame(&mut buf, &frame).unwrap();
    // Corrupt header CRC (byte 7)
    buf[7] ^= 0xFF;
    assert!(decode_frame(&buf).is_err());
}

#[test]
fn decode_bad_data_crc() {
    let mut buf = BytesMut::new();
    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataNotExpectingReply,
        destination: 1,
        source: 0,
        data: Bytes::from_static(&[0x01, 0x00]),
    };
    encode_frame(&mut buf, &frame).unwrap();
    // Corrupt last byte (data CRC high)
    let last = buf.len() - 1;
    buf[last] ^= 0xFF;
    assert!(decode_frame(&buf).is_err());
}

#[test]
fn decode_truncated_data() {
    let mut buf = BytesMut::new();
    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: 5,
        source: 0,
        data: Bytes::from_static(&[0x01, 0x02, 0x03, 0x04]),
    };
    encode_frame(&mut buf, &frame).unwrap();
    // Truncate: remove data CRC
    buf.truncate(buf.len() - 2);
    assert!(decode_frame(&buf).is_err());
}

#[test]
fn find_preamble_at_start() {
    let data = [0x55, 0xFF, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];
    assert_eq!(find_preamble(&data), Some(0));
}

#[test]
fn find_preamble_with_garbage() {
    let data = [0x00, 0x00, 0x12, 0x55, 0xFF, 0x00, 0x01, 0x00];
    assert_eq!(find_preamble(&data), Some(3));
}

#[test]
fn find_preamble_none() {
    let data = [0x00, 0x55, 0x00, 0xFF, 0x01];
    assert_eq!(find_preamble(&data), None);
}

#[test]
fn frame_type_round_trip() {
    for raw in 0..=0x07 {
        let ft = FrameType::from_raw(raw);
        assert_eq!(ft.to_raw(), raw);
    }
    // Unknown type
    let ft = FrameType::from_raw(0x42);
    assert_eq!(ft.to_raw(), 0x42);
    assert_eq!(ft, FrameType::Unknown(0x42));
}

#[test]
fn frame_type_has_data() {
    assert!(!FrameType::Token.has_data());
    assert!(!FrameType::PollForMaster.has_data());
    assert!(!FrameType::ReplyToPollForMaster.has_data());
    assert!(FrameType::TestRequest.has_data());
    assert!(FrameType::TestResponse.has_data());
    assert!(FrameType::BACnetDataExpectingReply.has_data());
    assert!(FrameType::BACnetDataNotExpectingReply.has_data());
    assert!(!FrameType::ReplyPostponed.has_data());
}

fn data_frame_with_length(data_len: usize) -> MstpFrame {
    MstpFrame {
        frame_type: FrameType::BACnetDataNotExpectingReply,
        destination: BROADCAST_MAC,
        source: 0,
        data: Bytes::from(vec![0xAA; data_len]),
    }
}

fn header_only_wire(frame_type: u8, data_len: usize) -> Vec<u8> {
    let header = [
        frame_type,
        1,
        0,
        (data_len >> 8) as u8,
        (data_len & 0xFF) as u8,
    ];
    let mut wire = PREAMBLE.to_vec();
    wire.extend_from_slice(&header);
    wire.push(crc8(&header));
    wire
}

#[test]
fn encode_standard_data_length_boundary() {
    let frame = data_frame_with_length(MAX_STANDARD_MPDU_DATA);
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();
    assert_eq!(buf.len(), MAX_STANDARD_FRAME_LENGTH);

    let oversized = data_frame_with_length(MAX_STANDARD_MPDU_DATA + 1);
    let mut oversized_buf = BytesMut::new();
    assert!(encode_frame(&mut oversized_buf, &oversized).is_err());
    assert!(oversized_buf.is_empty(), "oversized frame wrote wire bytes");
}

#[test]
fn decode_standard_data_length_boundary() {
    let frame = data_frame_with_length(MAX_STANDARD_MPDU_DATA);
    let mut wire = BytesMut::new();
    encode_frame(&mut wire, &frame).unwrap();

    let (decoded, consumed) = decode_frame(&wire).unwrap();
    assert_eq!(decoded, frame);
    assert_eq!(consumed, MAX_STANDARD_FRAME_LENGTH);

    let oversized = header_only_wire(0x06, MAX_STANDARD_MPDU_DATA + 1);
    assert!(decode_frame(&oversized).is_err());
}

#[test]
fn stream_decode_standard_data_length_boundary() {
    let frame = data_frame_with_length(MAX_STANDARD_MPDU_DATA);
    let mut wire = BytesMut::new();
    encode_frame(&mut wire, &frame).unwrap();

    assert_eq!(
        decode_frame_stream(&wire),
        StreamDecode::Complete {
            frame,
            consumed: MAX_STANDARD_FRAME_LENGTH,
        }
    );

    let oversized = header_only_wire(0x06, MAX_STANDARD_MPDU_DATA + 1);
    assert_eq!(
        decode_frame_stream(&oversized),
        StreamDecode::Invalid { discard: 1 }
    );
}

#[test]
fn cobs_encoded_frame_type_range_is_rejected() {
    for (raw, cobs_encoded) in [(0x1F, false), (0x20, true), (0x7F, true), (0x80, false)] {
        let frame = MstpFrame {
            frame_type: FrameType::Unknown(raw),
            destination: 1,
            source: 0,
            data: Bytes::new(),
        };
        let mut encoded = BytesMut::new();
        let wire = header_only_wire(raw, 0);
        if cobs_encoded {
            assert!(encode_frame(&mut encoded, &frame).is_err(), "raw={raw}");
            assert!(encoded.is_empty());
            assert!(decode_frame(&wire).is_err(), "raw={raw}");
            assert_eq!(
                decode_frame_stream(&wire),
                StreamDecode::Invalid { discard: 1 },
                "raw={raw}"
            );
        } else {
            encode_frame(&mut encoded, &frame).unwrap();
            assert_eq!(&encoded[..], wire, "raw={raw}");
            assert_eq!(decode_frame(&wire).unwrap(), (frame.clone(), wire.len()));
            assert_eq!(
                decode_frame_stream(&wire),
                StreamDecode::Complete {
                    frame,
                    consumed: wire.len(),
                },
                "raw={raw}"
            );
        }
    }
}

#[test]
fn max_standard_data_frame_round_trip() {
    let npdu = vec![0xAA; MAX_STANDARD_MPDU_DATA];
    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataNotExpectingReply,
        destination: BROADCAST_MAC,
        source: 0,
        data: Bytes::from(npdu.clone()),
    };
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();

    let (decoded, _) = decode_frame(&buf).unwrap();
    assert_eq!(decoded.data, npdu);
}

#[test]
fn encode_oversized_data_returns_error() {
    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataNotExpectingReply,
        destination: 1,
        source: 0,
        data: Bytes::from_static(&[0xAA; MAX_STANDARD_MPDU_DATA + 1]),
    };
    let mut buf = BytesMut::new();
    assert!(encode_frame(&mut buf, &frame).is_err());
}

#[test]
fn decode_rejects_source_above_max_master() {
    // Encode a valid frame then patch the source to 128 (above MAX_MASTER=127)
    let frame = MstpFrame {
        frame_type: FrameType::Token,
        destination: 1,
        source: 0,
        data: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();

    // Patch source byte (offset 4) to 128
    buf[4] = 128;
    // Recompute header CRC (bytes 2..7, CRC at byte 7)
    let header_crc = crc8(&buf[2..7]);
    buf[7] = header_crc;

    assert!(decode_frame(&buf).is_err());
}

#[test]
fn decode_rejects_broadcast_source() {
    // Encode a valid frame then patch the source to BROADCAST_MAC (0xFF)
    let frame = MstpFrame {
        frame_type: FrameType::Token,
        destination: 1,
        source: 0,
        data: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();

    // Patch source byte to 0xFF
    buf[4] = BROADCAST_MAC;
    // Recompute header CRC
    let header_crc = crc8(&buf[2..7]);
    buf[7] = header_crc;

    assert!(decode_frame(&buf).is_err());
}

#[test]
fn decode_accepts_max_master_source() {
    // Source = MAX_MASTER (127) should be valid
    let frame = MstpFrame {
        frame_type: FrameType::Token,
        destination: 1,
        source: MAX_MASTER,
        data: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();

    let (decoded, _) = decode_frame(&buf).unwrap();
    assert_eq!(decoded.source, MAX_MASTER);
}

// -----------------------------------------------------------------------
// Streaming decode tests
// -----------------------------------------------------------------------

fn encode_token_frame() -> Vec<u8> {
    let frame = MstpFrame {
        frame_type: FrameType::Token,
        destination: 1,
        source: 0,
        data: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();
    buf.to_vec()
}

fn encode_data_frame() -> (MstpFrame, Vec<u8>) {
    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataNotExpectingReply,
        destination: 5,
        source: 0,
        data: Bytes::from_static(&[0x01, 0x02, 0x03, 0x04]),
    };
    let mut buf = BytesMut::new();
    encode_frame(&mut buf, &frame).unwrap();
    (frame, buf.to_vec())
}

#[test]
fn stream_decode_token_complete() {
    let wire = encode_token_frame();
    assert_eq!(
        decode_frame_stream(&wire),
        StreamDecode::Complete {
            frame: MstpFrame {
                frame_type: FrameType::Token,
                destination: 1,
                source: 0,
                data: Bytes::new(),
            },
            consumed: 8,
        }
    );
}

#[test]
fn stream_decode_split_header_then_body() {
    let (_frame, wire) = encode_data_frame();
    let split = wire.len() - 3;

    assert_eq!(decode_frame_stream(&wire[..split]), StreamDecode::NeedMore);

    let mut assembled = wire[..split].to_vec();
    assembled.extend_from_slice(&wire[split..]);
    let StreamDecode::Complete { frame, consumed } = decode_frame_stream(&assembled) else {
        panic!("expected complete frame after reassembly");
    };
    assert_eq!(consumed, wire.len());
    assert_eq!(frame.data.len(), 4);
}

#[test]
fn stream_decode_preamble_split_across_chunks() {
    let wire = encode_token_frame();
    assert_eq!(decode_frame_stream(&wire[..1]), StreamDecode::NeedMore);

    let mut assembled = wire[..1].to_vec();
    assembled.extend_from_slice(&wire[1..]);
    assert!(matches!(
        decode_frame_stream(&assembled),
        StreamDecode::Complete { .. }
    ));
}

#[test]
fn stream_decode_host_gap_simulation_without_clear() {
    let wire = encode_token_frame();
    let mut buf = wire[..4].to_vec();
    assert_eq!(decode_frame_stream(&buf), StreamDecode::NeedMore);

    // Simulate a host gap far beyond wire T_frame_abort without clearing the buffer.
    buf.extend_from_slice(&wire[4..]);
    assert!(matches!(
        decode_frame_stream(&buf),
        StreamDecode::Complete { .. }
    ));
}

#[test]
fn retain_lone_preamble_byte_preserves_trailing_0x55() {
    let mut buf = vec![0x00, 0x12, 0x55];
    retain_lone_preamble_byte(&mut buf);
    assert_eq!(buf, vec![0x55]);

    let mut no_preamble = vec![0x00, 0x12, 0x34];
    retain_lone_preamble_byte(&mut no_preamble);
    assert!(no_preamble.is_empty());
}

#[test]
fn stream_decode_bad_header_crc_invalid() {
    let mut wire = encode_token_frame();
    wire[7] ^= 0xFF;
    assert_eq!(
        decode_frame_stream(&wire),
        StreamDecode::Invalid { discard: 1 }
    );
}

#[test]
fn stream_decode_bad_data_crc_invalid_discards_frame() {
    let (_frame, mut wire) = encode_data_frame();
    let last = wire.len() - 1;
    wire[last] ^= 0xFF;
    assert_eq!(
        decode_frame_stream(&wire),
        StreamDecode::Invalid {
            discard: wire.len()
        }
    );
}

#[test]
fn stream_decode_need_more_on_short_header() {
    assert_eq!(
        decode_frame_stream(&[0x55, 0xFF, 0x00, 0x01]),
        StreamDecode::NeedMore
    );
}

#[test]
fn stream_decode_invalid_on_bad_second_preamble_byte() {
    assert_eq!(
        decode_frame_stream(&[0x55, 0x00]),
        StreamDecode::Invalid { discard: 1 }
    );
}
