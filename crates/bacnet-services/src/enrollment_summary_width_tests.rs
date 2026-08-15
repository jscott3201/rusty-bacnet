use super::*;
use bacnet_types::enums::ObjectType;

fn raw_request(fields: [&[u8]; 7], device: bool) -> BytesMut {
    let mut buf = BytesMut::new();
    primitives::encode_ctx_octet_string(&mut buf, 0, fields[0]);
    tags::encode_opening_tag(&mut buf, 1);
    if device {
        tags::encode_opening_tag(&mut buf, 0);
        let object = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        primitives::encode_ctx_object_id(&mut buf, 0, &object);
        tags::encode_closing_tag(&mut buf, 0);
    }
    primitives::encode_ctx_octet_string(&mut buf, 1, fields[1]);
    tags::encode_closing_tag(&mut buf, 1);
    primitives::encode_ctx_octet_string(&mut buf, 2, fields[2]);
    primitives::encode_ctx_octet_string(&mut buf, 3, fields[3]);
    tags::encode_opening_tag(&mut buf, 4);
    primitives::encode_ctx_octet_string(&mut buf, 0, fields[4]);
    primitives::encode_ctx_octet_string(&mut buf, 1, fields[5]);
    tags::encode_closing_tag(&mut buf, 4);
    primitives::encode_ctx_octet_string(&mut buf, 5, fields[6]);
    buf
}

fn raw_ack(fields: [&[u8]; 4]) -> BytesMut {
    let mut buf = BytesMut::new();
    let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    primitives::encode_app_object_id(&mut buf, &object);
    for (tag, value) in [
        (tags::app_tag::ENUMERATED, fields[0]),
        (tags::app_tag::ENUMERATED, fields[1]),
        (tags::app_tag::UNSIGNED, fields[2]),
        (tags::app_tag::UNSIGNED, fields[3]),
    ] {
        tags::encode_tag(
            &mut buf,
            tag,
            tags::TagClass::Application,
            value.len() as u32,
        );
        buf.extend_from_slice(value);
    }
    buf
}

fn field_offsets(data: &[u8], count: usize) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(count);
    let mut offset = 0;
    for _ in 0..count {
        offsets.push(offset);
        let (tag, pos) = tags::decode_tag(data, offset).unwrap();
        offset = pos + tag.length as usize;
    }
    offsets
}

#[test]
fn request_values_must_fit_public_field_widths() {
    let max_u32 = [0, 0xFF, 0xFF, 0xFF, 0xFF];
    let max_u8 = [0, 0xFF];
    let max_u16 = [0, 0xFF, 0xFF];
    let decoded = GetEnrollmentSummaryRequest::decode(&raw_request(
        [
            &max_u32, &max_u32, &max_u32, &max_u32, &max_u8, &max_u8, &max_u16,
        ],
        true,
    ))
    .unwrap();
    assert_eq!(decoded.acknowledgment_filter, u32::MAX);
    assert_eq!(
        decoded.enrollment_filter.unwrap().process_identifier,
        u32::MAX
    );
    assert_eq!(decoded.event_state_filter.unwrap().to_raw(), u32::MAX);
    assert_eq!(decoded.event_type_filter.unwrap().to_raw(), u32::MAX);
    assert_eq!(decoded.priority_filter.unwrap().min_priority, u8::MAX);
    assert_eq!(decoded.notification_class_filter, Some(u16::MAX));

    let zero = [0];
    let base = [&zero[..]; 7];
    let overflow_u32 = (u32::MAX as u64 + 1).to_be_bytes();
    let overflow_u8 = [1, 0];
    let overflow_u16 = [1, 0, 0];
    for (field, overflow) in [
        (0, &overflow_u32[..]),
        (1, &overflow_u32[..]),
        (2, &overflow_u32[..]),
        (3, &overflow_u32[..]),
        (4, &overflow_u8[..]),
        (5, &overflow_u8[..]),
        (6, &overflow_u16[..]),
    ] {
        let mut fields = base;
        fields[field] = overflow;
        assert!(GetEnrollmentSummaryRequest::decode(&raw_request(fields, true)).is_err());
    }
    let max_u64 = u64::MAX.to_be_bytes();
    for field in 0..7 {
        let mut fields = base;
        fields[field] = &max_u64;
        assert!(GetEnrollmentSummaryRequest::decode(&raw_request(fields, true)).is_err());
    }
}

#[test]
fn request_enrollment_filter_uses_standard_recipient_process_framing() {
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 7).unwrap();
    let request = GetEnrollmentSummaryRequest {
        acknowledgment_filter: 0,
        enrollment_filter: Some(RecipientProcess {
            device: Some(device),
            process_identifier: 7,
        }),
        event_state_filter: None,
        event_type_filter: None,
        priority_filter: None,
        notification_class_filter: None,
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded);
    assert_eq!(
        GetEnrollmentSummaryRequest::decode(&encoded).unwrap(),
        request
    );
    assert_eq!(
        encoded.as_ref(),
        &[0x09, 0, 0x1E, 0x0E, 0x0C, 0x02, 0, 0, 7, 0x0F, 0x19, 7, 0x1F,]
    );

    let (acknowledgment_tag, acknowledgment_pos) = tags::decode_tag(&encoded, 0).unwrap();
    let filter_offset = acknowledgment_pos + acknowledgment_tag.length as usize;
    let (filter_tag, filter_start) = tags::decode_tag(&encoded, filter_offset).unwrap();
    assert!(filter_tag.is_opening_tag(1));
    let (recipient_tag, recipient_pos) = tags::decode_tag(&encoded, filter_start).unwrap();
    assert!(recipient_tag.is_opening_tag(0));
    let (device_tag, device_pos) = tags::decode_tag(&encoded, recipient_pos).unwrap();
    assert!(device_tag.is_context(0));
    assert_eq!(
        ObjectIdentifier::decode(&encoded[device_pos..device_pos + 4]).unwrap(),
        device
    );
    let (process_tag, _) = tags::decode_tag(&encoded, device_pos + 5).unwrap();
    assert!(process_tag.is_context(1));
}

#[test]
fn request_rejects_malformed_nested_and_trailing_fields() {
    let zero = [0];
    let valid = raw_request([&zero; 7], true);

    let mut wrong_ack = valid.clone();
    wrong_ack[0] = 0x19;
    assert!(GetEnrollmentSummaryRequest::decode(&wrong_ack).is_err());

    assert!(GetEnrollmentSummaryRequest::decode(&raw_request([&zero[..]; 7], false)).is_err());

    let mut trailing = valid;
    primitives::encode_app_null(&mut trailing);
    assert!(GetEnrollmentSummaryRequest::decode(&trailing).is_err());

    let mut missing_process = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut missing_process, 0, 0);
    tags::encode_opening_tag(&mut missing_process, 1);
    tags::encode_closing_tag(&mut missing_process, 1);
    assert!(GetEnrollmentSummaryRequest::decode(&missing_process).is_err());
}

#[test]
fn request_try_encode_rejects_unrepresentable_filters_without_writing() {
    let request = GetEnrollmentSummaryRequest {
        acknowledgment_filter: 0,
        enrollment_filter: Some(RecipientProcess {
            device: None,
            process_identifier: 7,
        }),
        event_state_filter: None,
        event_type_filter: None,
        priority_filter: None,
        notification_class_filter: None,
    };
    let mut encoded = BytesMut::new();
    assert!(request.try_encode(&mut encoded).is_err());
    assert!(encoded.is_empty());
}

#[test]
fn ack_values_must_fit_public_field_widths() {
    let max_u32 = [0, 0xFF, 0xFF, 0xFF, 0xFF];
    let max_u8 = [0, 0xFF];
    let max_u16 = [0, 0xFF, 0xFF];
    let decoded =
        GetEnrollmentSummaryAck::decode(&raw_ack([&max_u32, &max_u32, &max_u8, &max_u16])).unwrap();
    assert_eq!(decoded.entries[0].event_type.to_raw(), u32::MAX);
    assert_eq!(decoded.entries[0].event_state.to_raw(), u32::MAX);
    assert_eq!(decoded.entries[0].priority, u8::MAX);
    assert_eq!(decoded.entries[0].notification_class, u16::MAX);

    let zero = [0];
    let base = [&zero[..]; 4];
    let overflow_u32 = (u32::MAX as u64 + 1).to_be_bytes();
    let overflow_u8 = [1, 0];
    let overflow_u16 = [1, 0, 0];
    for (field, overflow) in [
        (0, &overflow_u32[..]),
        (1, &overflow_u32[..]),
        (2, &overflow_u8[..]),
        (3, &overflow_u16[..]),
    ] {
        let mut fields = base;
        fields[field] = overflow;
        assert!(GetEnrollmentSummaryAck::decode(&raw_ack(fields)).is_err());
    }
    let max_u64 = u64::MAX.to_be_bytes();
    for field in 0..4 {
        let mut fields = base;
        fields[field] = &max_u64;
        assert!(GetEnrollmentSummaryAck::decode(&raw_ack(fields)).is_err());
    }
}

#[test]
fn ack_accepts_omitted_notification_class_between_entries() {
    let zero = [0];
    let encoded = raw_ack([&zero; 4]);
    let notification_offset = field_offsets(&encoded, 5)[4];
    let without_notification = &encoded[..notification_offset];

    let decoded = GetEnrollmentSummaryAck::decode(without_notification).unwrap();
    assert_eq!(decoded.entries[0].notification_class, 0);

    let mut repeated = BytesMut::from(without_notification);
    repeated.extend_from_slice(without_notification);
    let decoded = GetEnrollmentSummaryAck::decode(&repeated).unwrap();
    assert_eq!(decoded.entries.len(), 2);
    assert!(decoded
        .entries
        .iter()
        .all(|entry| entry.notification_class == 0));
}

#[test]
fn ack_requires_exact_application_field_tags() {
    let zero = [0];
    let valid = raw_ack([&zero; 4]);
    let offsets = field_offsets(&valid, 5);

    for offset in offsets.iter().copied() {
        let mut context = valid.clone();
        context[offset] |= 0x08;
        assert!(GetEnrollmentSummaryAck::decode(&context).is_err());

        for lvt in [6, 7] {
            let mut reserved = valid.to_vec();
            let declared_length = if offset == 0 { 4 } else { 1 };
            reserved[offset] = (reserved[offset] & 0xF8) | lvt;
            reserved.insert(offset + 1, declared_length);
            assert!(GetEnrollmentSummaryAck::decode(&reserved).is_err());
        }
    }

    let mut wrong_type = valid.clone();
    wrong_type[offsets[1]] = (tags::app_tag::UNSIGNED << 4) | (wrong_type[offsets[1]] & 0x0F);
    assert!(GetEnrollmentSummaryAck::decode(&wrong_type).is_err());

    let mut trailing = valid;
    primitives::encode_app_null(&mut trailing);
    assert!(GetEnrollmentSummaryAck::decode(&trailing).is_err());
}
