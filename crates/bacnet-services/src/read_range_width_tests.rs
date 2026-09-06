use super::*;
use bacnet_types::enums::ObjectType;

#[derive(Clone, Copy)]
enum TestRange {
    Position,
    Sequence,
    Time,
}

fn object_identifier() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap()
}

fn encode_context_value(buf: &mut BytesMut, number: u8, value: &[u8]) {
    tags::encode_tag(buf, number, tags::TagClass::Context, value.len() as u32);
    buf.extend_from_slice(value);
}

fn encode_application_value(buf: &mut BytesMut, number: u8, value: &[u8]) {
    tags::encode_tag(buf, number, tags::TagClass::Application, value.len() as u32);
    buf.extend_from_slice(value);
}

fn request_prefix(property: &[u8], array_index: Option<&[u8]>) -> BytesMut {
    let mut buf = BytesMut::new();
    primitives::encode_ctx_object_id(&mut buf, 0, &object_identifier());
    encode_context_value(&mut buf, 1, property);
    if let Some(index) = array_index {
        encode_context_value(&mut buf, 2, index);
    }
    buf
}

fn append_range(
    buf: &mut BytesMut,
    range: TestRange,
    reference_tag: u8,
    reference: &[u8],
    count_tag: u8,
    count: &[u8],
) {
    let context_tag = match range {
        TestRange::Position => 3,
        TestRange::Sequence => 6,
        TestRange::Time => 7,
    };
    tags::encode_opening_tag(buf, context_tag);
    match range {
        TestRange::Position | TestRange::Sequence => {
            encode_application_value(buf, reference_tag, reference);
        }
        TestRange::Time => {
            primitives::encode_app_date(
                buf,
                &Date {
                    year: 126,
                    month: 3,
                    day: 1,
                    day_of_week: 7,
                },
            );
            primitives::encode_app_time(
                buf,
                &Time {
                    hour: 14,
                    minute: 30,
                    second: 0,
                    hundredths: 0,
                },
            );
        }
    }
    encode_application_value(buf, count_tag, count);
    tags::encode_closing_tag(buf, context_tag);
}

fn encode_ack(
    property: &[u8],
    array_index: Option<&[u8]>,
    item_count: &[u8],
    first_sequence_number: Option<&[u8]>,
) -> BytesMut {
    let item_data = item_count
        .iter()
        .any(|octet| *octet != 0)
        .then_some(&[0x00][..])
        .unwrap_or_default();
    encode_ack_with_item_data(
        property,
        array_index,
        item_count,
        item_data,
        first_sequence_number,
    )
}

fn encode_ack_with_item_data(
    property: &[u8],
    array_index: Option<&[u8]>,
    item_count: &[u8],
    item_data: &[u8],
    first_sequence_number: Option<&[u8]>,
) -> BytesMut {
    let mut buf = BytesMut::new();
    primitives::encode_ctx_object_id(&mut buf, 0, &object_identifier());
    encode_context_value(&mut buf, 1, property);
    if let Some(index) = array_index {
        encode_context_value(&mut buf, 2, index);
    }
    primitives::encode_ctx_bit_string(&mut buf, 3, 5, &[0xa0]);
    encode_context_value(&mut buf, 4, item_count);
    tags::encode_opening_tag(&mut buf, 5);
    buf.extend_from_slice(item_data);
    tags::encode_closing_tag(&mut buf, 5);
    if let Some(sequence_number) = first_sequence_number {
        encode_context_value(&mut buf, 6, sequence_number);
    }
    buf
}

#[test]
fn request_unsigned_fields_accept_u32_max_with_leading_zero() {
    let max = [0, 0xff, 0xff, 0xff, 0xff];
    let mut by_position = request_prefix(&max, Some(&max));
    append_range(
        &mut by_position,
        TestRange::Position,
        tags::app_tag::UNSIGNED,
        &max,
        tags::app_tag::SIGNED,
        &[1],
    );
    let decoded = ReadRangeRequest::decode(&by_position).unwrap();
    assert_eq!(decoded.property_identifier.to_raw(), u32::MAX);
    assert_eq!(decoded.property_array_index, Some(u32::MAX));
    assert!(matches!(
        decoded.range,
        Some(RangeSpec::ByPosition {
            reference_index: u32::MAX,
            count: 1
        })
    ));

    let mut by_sequence = request_prefix(&[0x83], None);
    append_range(
        &mut by_sequence,
        TestRange::Sequence,
        tags::app_tag::UNSIGNED,
        &max,
        tags::app_tag::SIGNED,
        &[0xff],
    );
    assert!(matches!(
        ReadRangeRequest::decode(&by_sequence).unwrap().range,
        Some(RangeSpec::BySequenceNumber {
            reference_seq: u32::MAX,
            count: -1
        })
    ));
}

#[test]
fn request_unsigned_fields_reject_u32_overflow() {
    for value in [&[1, 0, 0, 0, 0][..], &[0xff; 8][..]] {
        assert!(ReadRangeRequest::decode(&request_prefix(value, None)).is_err());
        assert!(ReadRangeRequest::decode(&request_prefix(&[0x83], Some(value))).is_err());

        for range in [TestRange::Position, TestRange::Sequence] {
            let mut request = request_prefix(&[0x83], None);
            append_range(
                &mut request,
                range,
                tags::app_tag::UNSIGNED,
                value,
                tags::app_tag::SIGNED,
                &[1],
            );
            assert!(ReadRangeRequest::decode(&request).is_err());
        }
    }
}

#[test]
fn ack_unsigned_fields_enforce_u32_without_rejecting_leading_zero() {
    let max = [0, 0xff, 0xff, 0xff, 0xff];
    let decoded = ReadRangeAck::decode(&encode_ack(&max, Some(&max), &max, Some(&max))).unwrap();
    assert_eq!(decoded.property_identifier.to_raw(), u32::MAX);
    assert_eq!(decoded.property_array_index, Some(u32::MAX));
    assert_eq!(decoded.item_count, u32::MAX);
    assert_eq!(decoded.first_sequence_number, Some(u32::MAX));

    for value in [&[1, 0, 0, 0, 0][..], &[0xff; 8][..]] {
        assert!(ReadRangeAck::decode(&encode_ack(value, None, &[1], None)).is_err());
        assert!(ReadRangeAck::decode(&encode_ack(&[0x83], Some(value), &[1], None)).is_err());
        assert!(ReadRangeAck::decode(&encode_ack(&[0x83], None, value, None)).is_err());
        assert!(ReadRangeAck::decode(&encode_ack(&[0x83], None, &[1], Some(value))).is_err());
    }
}

#[test]
fn range_counts_require_nonzero_integer16_values() {
    for range in [TestRange::Position, TestRange::Sequence, TestRange::Time] {
        for count in [
            &[0][..],
            &[0, 0][..],
            &[0, 0x80, 0][..],
            &[0xff, 0x7f, 0xff][..],
        ] {
            let mut request = request_prefix(&[0x83], None);
            append_range(
                &mut request,
                range,
                tags::app_tag::UNSIGNED,
                &[1],
                tags::app_tag::SIGNED,
                count,
            );
            assert!(ReadRangeRequest::decode(&request).is_err());
        }

        for count in [&[0, 0x7f, 0xff][..], &[0xff, 0x80, 0][..]] {
            let mut request = request_prefix(&[0x83], None);
            append_range(
                &mut request,
                range,
                tags::app_tag::UNSIGNED,
                &[1],
                tags::app_tag::SIGNED,
                count,
            );
            assert!(ReadRangeRequest::decode(&request).is_ok());
        }
    }
}

#[test]
fn range_members_require_their_application_tags_and_complete_content() {
    let mut wrong_reference = request_prefix(&[0x83], None);
    append_range(
        &mut wrong_reference,
        TestRange::Position,
        tags::app_tag::ENUMERATED,
        &[1],
        tags::app_tag::SIGNED,
        &[1],
    );
    assert!(ReadRangeRequest::decode(&wrong_reference).is_err());

    let mut wrong_count = request_prefix(&[0x83], None);
    append_range(
        &mut wrong_count,
        TestRange::Sequence,
        tags::app_tag::UNSIGNED,
        &[1],
        tags::app_tag::UNSIGNED,
        &[1],
    );
    assert!(ReadRangeRequest::decode(&wrong_count).is_err());

    let mut wrong_date = request_prefix(&[0x83], None);
    let date_offset = wrong_date.len() + 1;
    append_range(
        &mut wrong_date,
        TestRange::Time,
        tags::app_tag::UNSIGNED,
        &[],
        tags::app_tag::SIGNED,
        &[1],
    );
    wrong_date[date_offset] = (tags::app_tag::TIME << 4) | 4;
    assert!(ReadRangeRequest::decode(&wrong_date).is_err());

    let mut wrong_time = request_prefix(&[0x83], None);
    let time_offset = wrong_time.len() + 6;
    append_range(
        &mut wrong_time,
        TestRange::Time,
        tags::app_tag::UNSIGNED,
        &[],
        tags::app_tag::SIGNED,
        &[1],
    );
    wrong_time[time_offset] = (tags::app_tag::DATE << 4) | 4;
    assert!(ReadRangeRequest::decode(&wrong_time).is_err());

    let mut trailing = request_prefix(&[0x83], None);
    append_range(
        &mut trailing,
        TestRange::Position,
        tags::app_tag::UNSIGNED,
        &[1],
        tags::app_tag::SIGNED,
        &[1],
    );
    let closing_tag = trailing[trailing.len() - 1];
    trailing.truncate(trailing.len() - 1);
    trailing.extend_from_slice(&[0x00, closing_tag]);
    assert!(ReadRangeRequest::decode(&trailing).is_err());
}

#[test]
fn requests_require_normative_context_tags_and_range_choice() {
    let mut wrong_object = request_prefix(&[0x83], None);
    wrong_object[0] = 0x1c;
    assert!(ReadRangeRequest::decode(&wrong_object).is_err());

    let mut wrong_property = request_prefix(&[0x83], None);
    wrong_property[5] = 0x09;
    assert!(ReadRangeRequest::decode(&wrong_property).is_err());

    assert!(ReadRangeRequest::decode(&request_prefix(&[0x83], Some(&[0]))).is_err());

    for property_identifier in [8, 80, 105] {
        assert!(ReadRangeRequest::decode(&request_prefix(&[property_identifier], None)).is_err());
    }

    let mut unknown_range = request_prefix(&[0x83], None);
    tags::encode_opening_tag(&mut unknown_range, 4);
    tags::encode_closing_tag(&mut unknown_range, 4);
    assert!(ReadRangeRequest::decode(&unknown_range).is_err());

    let mut trailing = request_prefix(&[0x83], None);
    append_range(
        &mut trailing,
        TestRange::Position,
        tags::app_tag::UNSIGNED,
        &[1],
        tags::app_tag::SIGNED,
        &[1],
    );
    primitives::encode_app_null(&mut trailing);
    assert!(ReadRangeRequest::decode(&trailing).is_err());
}

#[test]
fn by_time_requires_a_specific_datetime() {
    for (relative_offset, value) in [
        (2, 0xff),
        (3, 13),
        (4, 32),
        (5, 0xff),
        (7, 24),
        (8, 60),
        (9, 60),
        (10, 100),
    ] {
        let mut request = request_prefix(&[0x83], None);
        let range_offset = request.len();
        append_range(
            &mut request,
            TestRange::Time,
            tags::app_tag::UNSIGNED,
            &[],
            tags::app_tag::SIGNED,
            &[1],
        );
        request[range_offset + relative_offset] = value;
        assert!(ReadRangeRequest::decode(&request).is_err());
    }
}

#[test]
fn acknowledgments_validate_flags_item_data_and_suffix() {
    for (unused_bits, bits) in [
        (4, &[0xa0][..]),
        (5, &[][..]),
        (5, &[0xa0, 0][..]),
        (5, &[0xa1][..]),
    ] {
        let mut malformed_flags = BytesMut::new();
        primitives::encode_ctx_object_id(&mut malformed_flags, 0, &object_identifier());
        encode_context_value(&mut malformed_flags, 1, &[0x83]);
        primitives::encode_ctx_bit_string(&mut malformed_flags, 3, unused_bits, bits);
        encode_context_value(&mut malformed_flags, 4, &[1]);
        tags::encode_opening_tag(&mut malformed_flags, 5);
        tags::encode_closing_tag(&mut malformed_flags, 5);
        assert!(ReadRangeAck::decode(&malformed_flags).is_err());
    }

    let mut primitive_item_data = encode_ack(&[0x83], None, &[1], None);
    let opening = primitive_item_data.len() - 2;
    primitive_item_data[opening] = 0x59;
    assert!(ReadRangeAck::decode(&primitive_item_data).is_err());

    assert!(ReadRangeAck::decode(&encode_ack(&[0x83], None, &[0], Some(&[1]))).is_err());

    let mut wrong_first_sequence_tag = encode_ack(&[0x83], None, &[1], Some(&[1]));
    let first_sequence_tag = wrong_first_sequence_tag.len() - 2;
    wrong_first_sequence_tag[first_sequence_tag] = 0x21;
    assert!(ReadRangeAck::decode(&wrong_first_sequence_tag).is_err());

    assert!(
        ReadRangeAck::decode(&encode_ack_with_item_data(&[0x83], None, &[1], &[], None,)).is_err()
    );
    assert!(ReadRangeAck::decode(&encode_ack_with_item_data(
        &[0x83],
        None,
        &[0],
        &[0x00],
        None,
    ))
    .is_err());

    let mut trailing = encode_ack(&[0x83], None, &[1], None);
    encode_context_value(&mut trailing, 7, &[1]);
    assert!(ReadRangeAck::decode(&trailing).is_err());
}

#[test]
fn acknowledgments_require_mandatory_context_tags() {
    for (offset, header) in [(0, 0x1c), (5, 0x09), (7, 0x2a), (10, 0x39)] {
        let mut ack = encode_ack(&[0x83], None, &[1], None);
        ack[offset] = header;
        assert!(ReadRangeAck::decode(&ack).is_err());
    }
}
