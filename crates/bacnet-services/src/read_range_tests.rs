use super::*;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{Date, Time};

fn make_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap()
}

#[test]
fn request_round_trip() {
    let req = ReadRangeRequest {
        object_identifier: make_oid(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(RangeSpec::ByPosition {
            reference_index: 1,
            count: 10,
        }),
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = ReadRangeRequest::decode(&buf).unwrap();
    assert_eq!(decoded.object_identifier, req.object_identifier);
    assert_eq!(decoded.property_identifier, req.property_identifier);
    assert_eq!(decoded.range, req.range);
}

#[test]
fn request_no_range() {
    let req = ReadRangeRequest {
        object_identifier: make_oid(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: None,
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = ReadRangeRequest::decode(&buf).unwrap();
    assert!(decoded.range.is_none());
}

#[test]
fn request_by_sequence_number() {
    let req = ReadRangeRequest {
        object_identifier: make_oid(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(RangeSpec::BySequenceNumber {
            reference_seq: 100,
            count: -5,
        }),
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = ReadRangeRequest::decode(&buf).unwrap();
    assert_eq!(decoded.range, req.range);
}

#[test]
fn ack_round_trip() {
    let ack = ReadRangeAck {
        object_identifier: make_oid(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        result_flags: (true, false, true),
        item_count: 2,
        item_data: vec![0xAA, 0xBB, 0xCC],
        first_sequence_number: None,
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    let decoded = ReadRangeAck::decode(&buf).unwrap();
    assert_eq!(decoded.object_identifier, ack.object_identifier);
    assert_eq!(decoded.result_flags, (true, false, true));
    assert_eq!(decoded.item_count, 2);
    assert_eq!(decoded.item_data, vec![0xAA, 0xBB, 0xCC]);
    assert_eq!(decoded.first_sequence_number, None);
}

#[test]
fn ack_round_trip_with_first_sequence_number() {
    let ack = ReadRangeAck {
        object_identifier: make_oid(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        result_flags: (true, true, false),
        item_count: 5,
        item_data: vec![0x01, 0x02],
        first_sequence_number: Some(42),
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    let decoded = ReadRangeAck::decode(&buf).unwrap();
    assert_eq!(decoded.object_identifier, ack.object_identifier);
    assert_eq!(decoded.result_flags, (true, true, false));
    assert_eq!(decoded.item_count, 5);
    assert_eq!(decoded.item_data, vec![0x01, 0x02]);
    assert_eq!(decoded.first_sequence_number, Some(42));
}

#[test]
fn request_by_time() {
    let req = ReadRangeRequest {
        object_identifier: make_oid(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(RangeSpec::ByTime {
            reference_time: (
                Date {
                    year: 126, // 2026
                    month: 3,
                    day: 1,
                    day_of_week: 7, // Sunday
                },
                Time {
                    hour: 14,
                    minute: 30,
                    second: 0,
                    hundredths: 0,
                },
            ),
            count: -10,
        }),
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = ReadRangeRequest::decode(&buf).unwrap();
    assert_eq!(decoded.range, req.range);
}

// -----------------------------------------------------------------------
// Malformed-input decode error tests
// -----------------------------------------------------------------------

#[test]
fn test_decode_read_range_request_empty_input() {
    assert!(ReadRangeRequest::decode(&[]).is_err());
}

#[test]
fn test_decode_read_range_request_truncated_1_byte() {
    let req = ReadRangeRequest {
        object_identifier: make_oid(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(RangeSpec::ByPosition {
            reference_index: 1,
            count: 10,
        }),
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    assert!(ReadRangeRequest::decode(&buf[..1]).is_err());
}

#[test]
fn test_decode_read_range_request_truncated_3_bytes() {
    let req = ReadRangeRequest {
        object_identifier: make_oid(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(RangeSpec::ByPosition {
            reference_index: 1,
            count: 10,
        }),
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    assert!(ReadRangeRequest::decode(&buf[..3]).is_err());
}

#[test]
fn test_decode_read_range_request_invalid_tag() {
    assert!(ReadRangeRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
}

#[test]
fn test_decode_read_range_ack_empty_input() {
    assert!(ReadRangeAck::decode(&[]).is_err());
}

#[test]
fn test_decode_read_range_ack_truncated_1_byte() {
    let ack = ReadRangeAck {
        object_identifier: make_oid(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        result_flags: (true, false, true),
        item_count: 2,
        item_data: vec![0xAA, 0xBB, 0xCC],
        first_sequence_number: None,
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    assert!(ReadRangeAck::decode(&buf[..1]).is_err());
}

#[test]
fn test_decode_read_range_ack_truncated_3_bytes() {
    let ack = ReadRangeAck {
        object_identifier: make_oid(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        result_flags: (true, false, true),
        item_count: 2,
        item_data: vec![0xAA, 0xBB, 0xCC],
        first_sequence_number: None,
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    assert!(ReadRangeAck::decode(&buf[..3]).is_err());
}

#[test]
fn test_decode_read_range_ack_truncated_half() {
    let ack = ReadRangeAck {
        object_identifier: make_oid(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        result_flags: (true, false, true),
        item_count: 2,
        item_data: vec![0xAA, 0xBB, 0xCC],
        first_sequence_number: None,
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    let half = buf.len() / 2;
    assert!(ReadRangeAck::decode(&buf[..half]).is_err());
}

#[test]
fn test_decode_read_range_ack_invalid_tag() {
    assert!(ReadRangeAck::decode(&[0xFF, 0xFF, 0xFF]).is_err());
}

#[test]
fn read_range_request_truncated_inner_tag() {
    // Craft a ReadRangeRequest with truncated inner content in byPosition
    let data = [
        0x0C, 0x05, 0x00, 0x00, 0x01, // [0] object id (TrendLog:1)
        0x19, 0x83, // [1] property id (LOG_BUFFER=131)
        // Opening tag [3] byPosition
        0x3E, // Inner tag claiming 50 bytes but only 1 byte present
        0x21, 50, 0x01, // Closing tag [3]
        0x3F,
    ];
    assert!(ReadRangeRequest::decode(&data).is_err());
}
