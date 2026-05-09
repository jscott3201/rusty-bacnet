use super::*;

fn encode_to_vec(apdu: &Apdu) -> Vec<u8> {
    let mut buf = BytesMut::with_capacity(64);
    encode_apdu(&mut buf, apdu).unwrap();
    buf.to_vec()
}

// --- Max-segments / max-APDU helpers ---

#[test]
fn max_segments_round_trip() {
    assert_eq!(decode_max_segments(encode_max_segments(None)), None);
    assert_eq!(decode_max_segments(encode_max_segments(Some(2))), Some(2));
    assert_eq!(decode_max_segments(encode_max_segments(Some(4))), Some(4));
    assert_eq!(decode_max_segments(encode_max_segments(Some(8))), Some(8));
    assert_eq!(decode_max_segments(encode_max_segments(Some(16))), Some(16));
    assert_eq!(decode_max_segments(encode_max_segments(Some(32))), Some(32));
    assert_eq!(decode_max_segments(encode_max_segments(Some(64))), Some(64));
    assert_eq!(
        decode_max_segments(encode_max_segments(Some(100))),
        Some(255)
    );
}

#[test]
fn max_apdu_round_trip() {
    for value in [50, 128, 206, 480, 1024, 1476] {
        assert_eq!(
            decode_max_apdu(encode_max_apdu(value).unwrap()).unwrap(),
            value
        );
    }
    assert!(encode_max_apdu(9999).is_err());
}

#[test]
fn decode_max_apdu_reserved_value_errors() {
    for value in 6..=15 {
        assert!(decode_max_apdu(value).is_err());
    }
}

#[test]
fn decode_confirmed_request_reserved_max_apdu_errors() {
    // Low nibble 0x06 is reserved by ASHRAE 135-2020 Clause 20.1.2.5.
    let data = Bytes::from_static(&[0x00, 0x06, 0x01, 0x0C]);
    assert!(decode_apdu(data).is_err());
}

// --- ConfirmedRequest ---

#[test]
fn confirmed_request_non_segmented_round_trip() {
    let pdu = ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: Some(4),
        max_apdu_length: 1476,
        invoke_id: 42,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: Bytes::from_static(&[0x0C, 0x02, 0x00, 0x00, 0x01]),
    };
    let apdu = Apdu::ConfirmedRequest(pdu);
    let encoded = encode_to_vec(&apdu);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

#[test]
fn confirmed_request_segmented_round_trip() {
    let pdu = ConfirmedRequest {
        segmented: true,
        more_follows: true,
        segmented_response_accepted: true,
        max_segments: Some(64),
        max_apdu_length: 480,
        invoke_id: 7,
        sequence_number: Some(3),
        proposed_window_size: Some(16),
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: Bytes::from_static(&[0xAA, 0xBB]),
    };
    let apdu = Apdu::ConfirmedRequest(pdu);
    let encoded = encode_to_vec(&apdu);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

#[test]
fn confirmed_request_wire_format() {
    let pdu = ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 1,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: Bytes::new(),
    };
    let encoded = encode_to_vec(&Apdu::ConfirmedRequest(pdu));
    // byte0: (0<<4) | 0 = 0x00
    // byte1: (0<<4) | 5 = 0x05  (unspecified segments, 1476 apdu)
    // invoke_id: 0x01
    // service_choice: ReadProperty = 12 = 0x0C
    assert_eq!(&encoded[..4], &[0x00, 0x05, 0x01, 0x0C]);
}

// --- UnconfirmedRequest ---

#[test]
fn unconfirmed_request_round_trip() {
    let pdu = UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::WHO_IS,
        service_request: Bytes::from_static(&[0x01, 0x02, 0x03]),
    };
    let apdu = Apdu::UnconfirmedRequest(pdu);
    let encoded = encode_to_vec(&apdu);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

#[test]
fn unconfirmed_request_wire_format() {
    let pdu = UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: Bytes::new(),
    };
    let encoded = encode_to_vec(&Apdu::UnconfirmedRequest(pdu));
    // byte0: (1<<4) = 0x10
    // service_choice: IAm = 0
    assert_eq!(encoded, vec![0x10, 0x00]);
}

// --- SimpleAck ---

#[test]
fn simple_ack_round_trip() {
    let pdu = SimpleAck {
        invoke_id: 99,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
    };
    let apdu = Apdu::SimpleAck(pdu);
    let encoded = encode_to_vec(&apdu);
    assert_eq!(encoded.len(), 3);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

#[test]
fn simple_ack_wire_format() {
    let pdu = SimpleAck {
        invoke_id: 5,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
    };
    let encoded = encode_to_vec(&Apdu::SimpleAck(pdu));
    // byte0: (2<<4) = 0x20
    assert_eq!(encoded, vec![0x20, 0x05, 0x0C]);
}

// --- ComplexAck ---

#[test]
fn complex_ack_non_segmented_round_trip() {
    let pdu = ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: 42,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(&[0xDE, 0xAD]),
    };
    let apdu = Apdu::ComplexAck(pdu);
    let encoded = encode_to_vec(&apdu);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

#[test]
fn complex_ack_segmented_round_trip() {
    let pdu = ComplexAck {
        segmented: true,
        more_follows: false,
        invoke_id: 10,
        sequence_number: Some(5),
        proposed_window_size: Some(8),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        service_ack: Bytes::from_static(&[0x01]),
    };
    let apdu = Apdu::ComplexAck(pdu);
    let encoded = encode_to_vec(&apdu);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

// --- SegmentAck ---

#[test]
fn segment_ack_round_trip() {
    let pdu = SegmentAck {
        negative_ack: true,
        sent_by_server: false,
        invoke_id: 55,
        sequence_number: 12,
        actual_window_size: 4,
    };
    let apdu = Apdu::SegmentAck(pdu);
    let encoded = encode_to_vec(&apdu);
    assert_eq!(encoded.len(), 4);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

#[test]
fn encode_segment_ack_invalid_window_errors() {
    for actual_window_size in [0, 128] {
        let apdu = Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: false,
            invoke_id: 1,
            sequence_number: 0,
            actual_window_size,
        });
        let mut buf = BytesMut::new();
        assert!(encode_apdu(&mut buf, &apdu).is_err());
    }
}

#[test]
fn decode_segment_ack_invalid_window_errors() {
    assert!(decode_apdu(Bytes::from_static(&[0x40, 0x01, 0x00, 0x00])).is_err());
    assert!(decode_apdu(Bytes::from_static(&[0x40, 0x01, 0x00, 0x80])).is_err());
}

#[test]
fn encode_segmented_proposed_window_invalid_errors() {
    for proposed_window_size in [0, 128] {
        let apdu = Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: true,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 1,
            sequence_number: Some(0),
            proposed_window_size: Some(proposed_window_size),
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: Bytes::new(),
        });
        let mut buf = BytesMut::new();
        assert!(encode_apdu(&mut buf, &apdu).is_err());
    }
}

#[test]
fn segment_ack_flags() {
    // Both flags set
    let pdu = SegmentAck {
        negative_ack: true,
        sent_by_server: true,
        invoke_id: 1,
        sequence_number: 0,
        actual_window_size: 1,
    };
    let encoded = encode_to_vec(&Apdu::SegmentAck(pdu));
    // byte0: (4<<4) | 0x02 | 0x01 = 0x43
    assert_eq!(encoded[0], 0x43);
}

// --- Error ---

#[test]
fn error_round_trip() {
    let pdu = ErrorPdu {
        invoke_id: 10,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        error_class: ErrorClass::PROPERTY,
        error_code: ErrorCode::UNKNOWN_PROPERTY,
        error_data: Bytes::new(),
    };
    let apdu = Apdu::Error(pdu);
    let encoded = encode_to_vec(&apdu);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

#[test]
fn error_with_trailing_data_round_trip() {
    let pdu = ErrorPdu {
        invoke_id: 20,
        service_choice: ConfirmedServiceChoice::CREATE_OBJECT,
        error_class: ErrorClass::OBJECT,
        error_code: ErrorCode::NO_OBJECTS_OF_SPECIFIED_TYPE,
        error_data: Bytes::from_static(&[0x01, 0x02, 0x03]),
    };
    let apdu = Apdu::Error(pdu);
    let encoded = encode_to_vec(&apdu);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

// --- Reject ---

#[test]
fn reject_round_trip() {
    let pdu = RejectPdu {
        invoke_id: 77,
        reject_reason: RejectReason::INVALID_TAG,
    };
    let apdu = Apdu::Reject(pdu);
    let encoded = encode_to_vec(&apdu);
    assert_eq!(encoded.len(), 3);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

// --- Abort ---

#[test]
fn abort_round_trip() {
    let pdu = AbortPdu {
        sent_by_server: true,
        invoke_id: 33,
        abort_reason: AbortReason::BUFFER_OVERFLOW,
    };
    let apdu = Apdu::Abort(pdu);
    let encoded = encode_to_vec(&apdu);
    assert_eq!(encoded.len(), 3);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

#[test]
fn abort_server_flag() {
    let pdu = AbortPdu {
        sent_by_server: true,
        invoke_id: 0,
        abort_reason: AbortReason::OTHER,
    };
    let encoded = encode_to_vec(&Apdu::Abort(pdu));
    // byte0: (7<<4) | 0x01 = 0x71
    assert_eq!(encoded[0], 0x71);

    let pdu = AbortPdu {
        sent_by_server: false,
        invoke_id: 0,
        abort_reason: AbortReason::OTHER,
    };
    let encoded = encode_to_vec(&Apdu::Abort(pdu));
    // byte0: (7<<4) = 0x70
    assert_eq!(encoded[0], 0x70);
}

// --- Decode errors ---

#[test]
fn decode_empty_data() {
    assert!(decode_apdu(Bytes::new()).is_err());
}

#[test]
fn decode_unknown_pdu_type() {
    // PDU type nibble 0x0F (reserved)
    assert!(decode_apdu(Bytes::from_static(&[0xF0])).is_err());
}

#[test]
fn decode_truncated_confirmed_request() {
    // Only 2 bytes, need at least 4
    assert!(decode_apdu(Bytes::from_static(&[0x00, 0x05])).is_err());
}

#[test]
fn decode_truncated_simple_ack() {
    // Only 2 bytes, need 3
    assert!(decode_apdu(Bytes::from_static(&[0x20, 0x01])).is_err());
}

// --- Segmented APDU edge cases ---

#[test]
fn decode_truncated_segmented_confirmed_request() {
    // Segmented flag set but not enough bytes for sequence/window
    // byte0: (0<<4) | 0x08 (segmented) = 0x08
    // byte1: max-segments/apdu = 0x05
    // invoke_id: 0x01
    // Missing: sequence_number, window_size, service_choice
    assert!(decode_apdu(Bytes::from_static(&[0x08, 0x05, 0x01])).is_err());
}

#[test]
fn decode_segmented_confirmed_request_missing_service() {
    // Segmented, has sequence/window, but no service choice
    // byte0: 0x08 (segmented), byte1: 0x05, invoke_id: 1, seq: 0, win: 1
    assert!(decode_apdu(Bytes::from_static(&[0x08, 0x05, 0x01, 0x00, 0x01])).is_err());
}

#[test]
fn decode_truncated_segmented_complex_ack() {
    // Segmented ComplexAck but too short for sequence/window
    // byte0: (3<<4) | 0x08 = 0x38
    // invoke_id: 0x01
    // Missing: sequence_number, window_size
    assert!(decode_apdu(Bytes::from_static(&[0x38, 0x01])).is_err());
}

#[test]
fn decode_complex_ack_missing_service_choice() {
    // Non-segmented ComplexAck, only 2 bytes (need 3 minimum)
    assert!(decode_apdu(Bytes::from_static(&[0x30, 0x01])).is_err());
}

#[test]
fn decode_truncated_segment_ack() {
    // SegmentAck needs exactly 4 bytes
    assert!(decode_apdu(Bytes::from_static(&[0x40, 0x01, 0x02])).is_err());
}

#[test]
fn decode_truncated_error_pdu() {
    // Error PDU needs at least 5 bytes (type, invoke, service, error_class tag+value)
    assert!(decode_apdu(Bytes::from_static(&[0x50, 0x01, 0x0C, 0x91])).is_err());
}

#[test]
fn decode_truncated_reject() {
    // Reject needs 3 bytes
    assert!(decode_apdu(Bytes::from_static(&[0x60, 0x01])).is_err());
}

#[test]
fn decode_truncated_abort() {
    // Abort needs 3 bytes
    assert!(decode_apdu(Bytes::from_static(&[0x70, 0x01])).is_err());
}

// --- APDU round-trip edge cases ---

#[test]
fn confirmed_request_empty_service_data() {
    let pdu = ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 0,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: Bytes::new(),
    };
    let apdu = Apdu::ConfirmedRequest(pdu);
    let encoded = encode_to_vec(&apdu);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

#[test]
fn confirmed_request_invoke_id_zero() {
    let pdu = ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: Some(64),
        max_apdu_length: 1476,
        invoke_id: 0,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: Bytes::from_static(&[0xAA]),
    };
    let apdu = Apdu::ConfirmedRequest(pdu);
    let encoded = encode_to_vec(&apdu);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

#[test]
fn confirmed_request_invoke_id_255() {
    let pdu = ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 255,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: Bytes::from_static(&[0x01]),
    };
    let apdu = Apdu::ConfirmedRequest(pdu);
    let encoded = encode_to_vec(&apdu);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

#[test]
fn segmented_request_sequence_zero() {
    let pdu = ConfirmedRequest {
        segmented: true,
        more_follows: true,
        segmented_response_accepted: true,
        max_segments: Some(64),
        max_apdu_length: 480,
        invoke_id: 5,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        service_request: Bytes::from_static(&[0x01, 0x02]),
    };
    let apdu = Apdu::ConfirmedRequest(pdu);
    let encoded = encode_to_vec(&apdu);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}

#[test]
fn error_pdu_truncated_error_class() {
    // Error PDU with invoke_id and service choice but error class tag truncated
    // type=5<<4=0x50, invoke=1, service=12, then truncated tag
    assert!(decode_apdu(Bytes::from_static(&[0x50, 0x01, 0x0C])).is_err());
}

#[test]
fn error_pdu_truncated_error_code() {
    // Error PDU with error class but error code tag truncated
    // type=0x50, invoke=1, service=12, error_class(enum 0, 1byte)=0x91 0x00, then truncated
    let mut buf = BytesMut::with_capacity(16);
    buf.put_u8(0x50); // Error PDU
    buf.put_u8(1); // invoke_id
    buf.put_u8(0x0C); // service_choice (ReadProperty)
    primitives::encode_app_enumerated(&mut buf, 2); // error_class = PROPERTY
    assert!(decode_apdu(buf.freeze()).is_err());
}

#[test]
fn unconfirmed_request_empty_service_data() {
    let pdu = UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::WHO_IS,
        service_request: Bytes::new(),
    };
    let apdu = Apdu::UnconfirmedRequest(pdu);
    let encoded = encode_to_vec(&apdu);
    let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
    assert_eq!(apdu, decoded);
}
