use super::*;

fn file_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(bacnet_types::enums::ObjectType::FILE, 1).unwrap()
}

fn encode_count(buf: &mut BytesMut, tag_number: u8, value: u64, leading_zero: bool) {
    if leading_zero {
        tags::encode_tag(buf, tag_number, tags::TagClass::Application, 5);
        buf.extend_from_slice(&value.to_be_bytes()[3..]);
    } else {
        let offset = buf.len();
        primitives::encode_app_unsigned(buf, value);
        buf[offset] = (tag_number << 4) | (buf[offset] & 0x0f);
    }
}

fn encode_read_request(access_tag: u8, count_tag: u8, value: u64, leading_zero: bool) -> BytesMut {
    let mut buf = BytesMut::new();
    primitives::encode_app_object_id(&mut buf, &file_oid());
    tags::encode_opening_tag(&mut buf, access_tag);
    primitives::encode_app_signed(&mut buf, 0);
    encode_count(&mut buf, count_tag, value, leading_zero);
    tags::encode_closing_tag(&mut buf, access_tag);
    buf
}

fn encode_read_ack(count_tag: u8, value: u64, leading_zero: bool) -> BytesMut {
    let mut buf = BytesMut::new();
    primitives::encode_app_boolean(&mut buf, false);
    tags::encode_opening_tag(&mut buf, 1);
    primitives::encode_app_signed(&mut buf, 0);
    encode_count(&mut buf, count_tag, value, leading_zero);
    tags::encode_closing_tag(&mut buf, 1);
    buf
}

fn encode_write_record_request(record_count: u32, records: &[&[u8]]) -> BytesMut {
    let mut buf = BytesMut::new();
    primitives::encode_app_object_id(&mut buf, &file_oid());
    tags::encode_opening_tag(&mut buf, 1);
    primitives::encode_app_signed(&mut buf, 0);
    primitives::encode_app_unsigned(&mut buf, u64::from(record_count));
    for record in records {
        primitives::encode_app_octet_string(&mut buf, record);
    }
    tags::encode_closing_tag(&mut buf, 1);
    buf
}

fn append_empty_records(mut buf: BytesMut) -> BytesMut {
    buf.truncate(buf.len() - 1);
    for _ in 0..MAX_DECODED_ITEMS {
        primitives::encode_app_octet_string(&mut buf, &[]);
    }
    tags::encode_closing_tag(&mut buf, 1);
    buf
}

#[test]
fn atomic_write_record_short_list_is_missing_required_parameter() {
    let wire = encode_write_record_request(2, &[&[0x01]]);
    match AtomicWriteFileRequest::decode(&wire).unwrap_err() {
        Error::Reject { reason } => {
            assert_eq!(reason, RejectReason::MISSING_REQUIRED_PARAMETER.to_raw())
        }
        other => panic!("expected Reject/MISSING_REQUIRED_PARAMETER, got {other:?}"),
    }
}

#[test]
fn atomic_write_record_extra_element_is_too_many_arguments() {
    let wire = encode_write_record_request(1, &[&[0x01], &[0x02]]);
    match AtomicWriteFileRequest::decode(&wire).unwrap_err() {
        Error::Reject { reason } => {
            assert_eq!(reason, RejectReason::TOO_MANY_ARGUMENTS.to_raw())
        }
        other => panic!("expected Reject/TOO_MANY_ARGUMENTS, got {other:?}"),
    }
}

#[test]
fn atomic_write_record_zero_count_decodes() {
    let wire = encode_write_record_request(0, &[]);
    let decoded = AtomicWriteFileRequest::decode(&wire).unwrap();
    assert_eq!(
        decoded,
        AtomicWriteFileRequest {
            file_identifier: file_oid(),
            access: FileWriteAccessMethod::Record {
                file_start_record: 0,
                record_count: 0,
                file_record_data: vec![],
            },
        }
    );
}

#[test]
fn read_request_counts_accept_u32_max_with_leading_zero() {
    let stream = AtomicReadFileRequest::decode(&encode_read_request(
        0,
        tags::app_tag::UNSIGNED,
        u64::from(u32::MAX),
        true,
    ))
    .unwrap();
    assert!(matches!(
        stream.access,
        FileAccessMethod::Stream {
            requested_octet_count: u32::MAX,
            ..
        }
    ));

    let record = AtomicReadFileRequest::decode(&encode_read_request(
        1,
        tags::app_tag::UNSIGNED,
        u64::from(u32::MAX),
        true,
    ))
    .unwrap();
    assert!(matches!(
        record.access,
        FileAccessMethod::Record {
            requested_record_count: u32::MAX,
            ..
        }
    ));
}

#[test]
fn bounded_record_counts_accept_limit_with_leading_zero() {
    let value = MAX_DECODED_ITEMS as u64;
    let request =
        append_empty_records(encode_read_request(1, tags::app_tag::UNSIGNED, value, true));
    let write = AtomicWriteFileRequest::decode(&request).unwrap();
    assert!(matches!(
        write.access,
        FileWriteAccessMethod::Record {
            record_count,
            file_record_data,
            ..
        } if record_count == MAX_DECODED_ITEMS as u32
            && file_record_data.len() == MAX_DECODED_ITEMS
    ));

    let ack = AtomicReadFileAck::decode(&append_empty_records(encode_read_ack(
        tags::app_tag::UNSIGNED,
        value,
        true,
    )))
    .unwrap();
    assert!(matches!(
        ack.access,
        FileReadAckMethod::Record {
            returned_record_count,
            file_record_data,
            ..
        } if returned_record_count == MAX_DECODED_ITEMS as u32
            && file_record_data.len() == MAX_DECODED_ITEMS
    ));
}

#[test]
fn atomic_file_counts_reject_u32_overflow() {
    for value in [u64::from(u32::MAX) + 1, u64::MAX] {
        assert!(AtomicReadFileRequest::decode(&encode_read_request(
            0,
            tags::app_tag::UNSIGNED,
            value,
            false,
        ))
        .is_err());
        let record = encode_read_request(1, tags::app_tag::UNSIGNED, value, false);
        assert!(AtomicReadFileRequest::decode(&record).is_err());
        assert!(AtomicWriteFileRequest::decode(&record).is_err());
        assert!(AtomicReadFileAck::decode(
            &encode_read_ack(tags::app_tag::UNSIGNED, value, false,)
        )
        .is_err());
    }
}

#[test]
fn atomic_file_counts_require_application_unsigned_tags() {
    let stream = encode_read_request(0, tags::app_tag::ENUMERATED, 1, false);
    assert!(AtomicReadFileRequest::decode(&stream).is_err());

    let record = encode_read_request(1, tags::app_tag::ENUMERATED, 1, false);
    assert!(AtomicReadFileRequest::decode(&record).is_err());
    assert!(AtomicWriteFileRequest::decode(&record).is_err());

    let ack = encode_read_ack(tags::app_tag::ENUMERATED, 1, false);
    assert!(AtomicReadFileAck::decode(&ack).is_err());

    let mut context = BytesMut::new();
    primitives::encode_app_object_id(&mut context, &file_oid());
    tags::encode_opening_tag(&mut context, 0);
    primitives::encode_app_signed(&mut context, 0);
    primitives::encode_ctx_unsigned(&mut context, 2, 1);
    tags::encode_closing_tag(&mut context, 0);
    assert!(AtomicReadFileRequest::decode(&context).is_err());
}

#[test]
fn atomic_file_counts_reject_reserved_application_lvt() {
    for header in [0x26, 0x27] {
        let mut stream = BytesMut::new();
        primitives::encode_app_object_id(&mut stream, &file_oid());
        tags::encode_opening_tag(&mut stream, 0);
        primitives::encode_app_signed(&mut stream, 0);
        stream.extend_from_slice(&[header, 0x01, 0x00]);
        tags::encode_closing_tag(&mut stream, 0);
        assert!(AtomicReadFileRequest::decode(&stream).is_err());
    }
}
