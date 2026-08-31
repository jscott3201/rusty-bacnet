use super::*;

fn encode_record_ack(
    returned_record_count: u32,
    records: &[&[u8]],
    include_closing_tag: bool,
) -> BytesMut {
    let mut buf = BytesMut::new();
    primitives::encode_app_boolean(&mut buf, false);
    tags::encode_opening_tag(&mut buf, 1);
    primitives::encode_app_signed(&mut buf, 0);
    primitives::encode_app_unsigned(&mut buf, u64::from(returned_record_count));
    for record in records {
        primitives::encode_app_octet_string(&mut buf, record);
    }
    if include_closing_tag {
        tags::encode_closing_tag(&mut buf, 1);
    }
    buf
}

fn encode_stream_access(buf: &mut BytesMut) {
    tags::encode_opening_tag(buf, 0);
    primitives::encode_app_signed(buf, 0);
    primitives::encode_app_octet_string(buf, &[0xAA]);
    tags::encode_closing_tag(buf, 0);
}

#[test]
fn atomic_read_file_ack_requires_application_boolean() {
    let mut wrong_tag = BytesMut::new();
    primitives::encode_app_unsigned(&mut wrong_tag, 0);
    encode_stream_access(&mut wrong_tag);
    assert!(AtomicReadFileAck::decode(&wrong_tag).is_err());

    let mut invalid_boolean_value = BytesMut::from(&[0x12][..]);
    encode_stream_access(&mut invalid_boolean_value);
    assert!(AtomicReadFileAck::decode(&invalid_boolean_value).is_err());
}

#[test]
fn atomic_read_file_ack_record_count_must_equal_payload_cardinality() {
    let missing = encode_record_ack(2, &[&[0x01]], true);
    assert!(AtomicReadFileAck::decode(&missing).is_err());

    let surplus = encode_record_ack(1, &[&[0x01], &[0x02]], true);
    assert!(AtomicReadFileAck::decode(&surplus).is_err());
}

#[test]
fn atomic_read_file_ack_requires_closure_and_complete_consumption() {
    let unclosed = encode_record_ack(1, &[&[0x01]], false);
    assert!(AtomicReadFileAck::decode(&unclosed).is_err());

    let mut trailing = encode_record_ack(1, &[&[0x01]], true);
    trailing.extend_from_slice(&[0x00]);
    assert!(AtomicReadFileAck::decode(&trailing).is_err());
}

#[test]
fn atomic_read_file_ack_preserves_zero_length_records() {
    let wire = encode_record_ack(3, &[&[], &[0x00, 0xFF], &[]], true);
    let ack = AtomicReadFileAck::decode(&wire).unwrap();
    assert_eq!(
        ack,
        AtomicReadFileAck {
            end_of_file: false,
            access: FileReadAckMethod::Record {
                file_start_record: 0,
                returned_record_count: 3,
                file_record_data: vec![vec![], vec![0x00, 0xFF], vec![]],
            },
        }
    );
}
