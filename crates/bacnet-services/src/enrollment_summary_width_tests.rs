use super::*;
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bacnet_types::enums::ObjectType;
use bacnet_types::MacAddr;

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
    let max_filter = [4];
    let acknowledgment_filter = [2];
    let decoded = GetEnrollmentSummaryRequest::decode(&raw_request(
        [
            &acknowledgment_filter,
            &max_u32,
            &max_filter,
            &max_u32,
            &max_u8,
            &max_u8,
            &max_u32,
        ],
        true,
    ))
    .unwrap();
    assert_eq!(decoded.acknowledgment_filter, 2);
    assert_eq!(
        decoded.enrollment_filter.unwrap().process_identifier,
        u32::MAX
    );
    assert_eq!(
        decoded.event_state_filter,
        Some(EnrollmentSummaryEventStateFilter::ACTIVE)
    );
    assert_eq!(decoded.event_type_filter.unwrap().to_raw(), u32::MAX);
    assert_eq!(decoded.priority_filter.unwrap().min_priority, u8::MAX);
    assert_eq!(decoded.notification_class_filter, Some(u32::MAX));

    let zero = [0];
    let base = [&zero[..]; 7];
    let overflow_u32 = (u32::MAX as u64 + 1).to_be_bytes();
    let overflow_u8 = [1, 0];
    for (field, overflow) in [
        (1, &overflow_u32[..]),
        (3, &overflow_u32[..]),
        (4, &overflow_u8[..]),
        (5, &overflow_u8[..]),
        (6, &overflow_u32[..]),
    ] {
        let mut fields = base;
        fields[field] = overflow;
        assert!(GetEnrollmentSummaryRequest::decode(&raw_request(fields, true)).is_err());
    }
    let max_u64 = u64::MAX.to_be_bytes();
    for field in 1..7 {
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
            recipient: BACnetRecipient::Device(device),
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
fn request_enrollment_filter_address_choice_golden_and_round_trip() {
    let request = GetEnrollmentSummaryRequest {
        acknowledgment_filter: 1,
        enrollment_filter: Some(RecipientProcess {
            recipient: BACnetRecipient::Address(BACnetAddress {
                network_number: 5,
                mac_address: MacAddr::from_slice(&[192, 0, 2, 9, 0xba, 0xc0]),
            }),
            process_identifier: 9,
        }),
        event_state_filter: None,
        event_type_filter: None,
        priority_filter: None,
        notification_class_filter: None,
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded);

    assert_eq!(
        encoded.as_ref(),
        &[
            0x09, 1, 0x1e, 0x0e, 0x1e, 0x21, 5, 0x65, 6, 192, 0, 2, 9, 0xba, 0xc0, 0x1f, 0x0f,
            0x19, 9, 0x1f,
        ]
    );
    assert_eq!(
        GetEnrollmentSummaryRequest::decode(&encoded).unwrap(),
        request
    );
}

#[test]
fn request_rejects_malformed_nested_and_trailing_fields() {
    let zero = [0];
    let valid = raw_request([&zero; 7], true);

    let mut wrong_ack = valid.clone();
    wrong_ack[0] = 0x19;
    assert!(GetEnrollmentSummaryRequest::decode(&wrong_ack).is_err());

    assert!(GetEnrollmentSummaryRequest::decode(&raw_request([&zero[..]; 7], false)).is_err());

    let mut trailing = valid.clone();
    primitives::encode_app_null(&mut trailing);
    assert!(GetEnrollmentSummaryRequest::decode(&trailing).is_err());

    let mut trailing_choice = valid.to_vec();
    trailing_choice.insert(9, 0);
    assert!(GetEnrollmentSummaryRequest::decode(&trailing_choice).is_err());

    let mut malformed_choice = valid;
    malformed_choice[4] = 0x2c;
    assert!(GetEnrollmentSummaryRequest::decode(&malformed_choice).is_err());

    let mut missing_process = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut missing_process, 0, 0);
    tags::encode_opening_tag(&mut missing_process, 1);
    tags::encode_closing_tag(&mut missing_process, 1);
    assert!(GetEnrollmentSummaryRequest::decode(&missing_process).is_err());
}

#[test]
fn request_try_encode_rejects_unrepresentable_filters_without_writing() {
    let request = GetEnrollmentSummaryRequest {
        acknowledgment_filter: 3,
        enrollment_filter: None,
        event_state_filter: None,
        event_type_filter: None,
        priority_filter: None,
        notification_class_filter: None,
    };
    let mut encoded = BytesMut::new();
    assert!(request.try_encode(&mut encoded).is_err());
    assert!(encoded.is_empty());

    let request = GetEnrollmentSummaryRequest {
        acknowledgment_filter: 0,
        priority_filter: Some(PriorityFilter {
            min_priority: 10,
            max_priority: 9,
        }),
        ..request
    };
    assert!(request.try_encode(&mut encoded).is_err());
    assert!(encoded.is_empty());
}

#[test]
fn request_rejects_undefined_acknowledgment_filter_and_inverted_priority() {
    for raw in [3, u8::MAX] {
        let encoded = [0x09, raw];
        assert!(matches!(
            GetEnrollmentSummaryRequest::decode(&encoded),
            Err(Error::Reject { reason })
                if reason == bacnet_types::enums::RejectReason::UNDEFINED_ENUMERATION.to_raw()
        ));
    }

    let mut encoded = BytesMut::new();
    primitives::encode_ctx_enumerated(&mut encoded, 0, 0);
    tags::encode_opening_tag(&mut encoded, 4);
    primitives::encode_ctx_unsigned(&mut encoded, 0, 10);
    primitives::encode_ctx_unsigned(&mut encoded, 1, 9);
    tags::encode_closing_tag(&mut encoded, 4);
    assert!(matches!(
        GetEnrollmentSummaryRequest::decode(&encoded),
        Err(Error::Reject { reason })
            if reason == bacnet_types::enums::RejectReason::INVALID_DATA_ENCODING.to_raw()
    ));
}

#[test]
fn ack_values_must_fit_public_field_widths() {
    let max_u32 = [0, 0xFF, 0xFF, 0xFF, 0xFF];
    let max_u8 = [0, 0xFF];
    let decoded =
        GetEnrollmentSummaryAck::decode(&raw_ack([&max_u32, &max_u32, &max_u8, &max_u32])).unwrap();
    assert_eq!(decoded.entries[0].event_type.to_raw(), u32::MAX);
    assert_eq!(decoded.entries[0].event_state.to_raw(), u32::MAX);
    assert_eq!(decoded.entries[0].priority, u8::MAX);
    assert_eq!(decoded.entries[0].notification_class, Some(u32::MAX));

    let zero = [0];
    let base = [&zero[..]; 4];
    let overflow_u32 = (u32::MAX as u64 + 1).to_be_bytes();
    let overflow_u8 = [1, 0];
    for (field, overflow) in [
        (0, &overflow_u32[..]),
        (1, &overflow_u32[..]),
        (2, &overflow_u8[..]),
        (3, &overflow_u32[..]),
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
    assert_eq!(decoded.entries[0].notification_class, None);
    let mut reencoded = BytesMut::new();
    decoded.encode(&mut reencoded);
    assert_eq!(
        reencoded.as_ref(),
        without_notification,
        "decoding and re-encoding must preserve an omitted notification class"
    );

    let mut repeated = BytesMut::from(without_notification);
    repeated.extend_from_slice(without_notification);
    let decoded = GetEnrollmentSummaryAck::decode(&repeated).unwrap();
    assert_eq!(decoded.entries.len(), 2);
    assert!(decoded
        .entries
        .iter()
        .all(|entry| entry.notification_class.is_none()));
}

#[test]
fn ack_preserves_absent_present_zero_and_mixed_notification_classes() {
    let entries = vec![
        EnrollmentSummaryEntry {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            event_type: EventType::CHANGE_OF_STATE,
            event_state: EventState::NORMAL,
            priority: 1,
            notification_class: None,
        },
        EnrollmentSummaryEntry {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap(),
            event_type: EventType::CHANGE_OF_STATE,
            event_state: EventState::FAULT,
            priority: 2,
            notification_class: Some(0),
        },
        EnrollmentSummaryEntry {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
            event_type: EventType::CHANGE_OF_STATE,
            event_state: EventState::OFFNORMAL,
            priority: u8::MAX,
            notification_class: Some(u32::MAX),
        },
        EnrollmentSummaryEntry {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 4).unwrap(),
            event_type: EventType::CHANGE_OF_STATE,
            event_state: EventState::NORMAL,
            priority: 4,
            notification_class: None,
        },
    ];
    let ack = GetEnrollmentSummaryAck { entries };
    let mut encoded = BytesMut::new();
    ack.encode(&mut encoded);

    assert_eq!(GetEnrollmentSummaryAck::decode(&encoded).unwrap(), ack);
}

#[test]
fn ack_enforces_decoded_entry_cap() {
    let zero = [0];
    let entry_with_notification = raw_ack([&zero; 4]);
    let notification_offset = field_offsets(&entry_with_notification, 5)[4];
    let entry = &entry_with_notification[..notification_offset];
    let mut encoded = BytesMut::with_capacity(entry.len() * (MAX_DECODED_ITEMS + 1));
    for _ in 0..MAX_DECODED_ITEMS {
        encoded.extend_from_slice(entry);
    }
    assert_eq!(
        GetEnrollmentSummaryAck::decode(&encoded)
            .unwrap()
            .entries
            .len(),
        MAX_DECODED_ITEMS
    );
    encoded.extend_from_slice(entry);
    assert!(GetEnrollmentSummaryAck::decode(&encoded).is_err());
}

#[test]
fn request_event_state_filter_uses_service_defined_values() {
    for (raw, expected) in [
        (0, EnrollmentSummaryEventStateFilter::OFFNORMAL),
        (1, EnrollmentSummaryEventStateFilter::FAULT),
        (2, EnrollmentSummaryEventStateFilter::NORMAL),
        (3, EnrollmentSummaryEventStateFilter::ALL),
        (4, EnrollmentSummaryEventStateFilter::ACTIVE),
    ] {
        let mut encoded = BytesMut::new();
        primitives::encode_ctx_enumerated(&mut encoded, 0, 0);
        primitives::encode_ctx_enumerated(&mut encoded, 2, raw);
        assert_eq!(encoded.as_ref(), &[0x09, 0, 0x29, raw as u8]);
        assert_eq!(
            GetEnrollmentSummaryRequest::decode(&encoded)
                .unwrap()
                .event_state_filter,
            Some(expected)
        );
    }
}

#[test]
fn request_rejects_undefined_event_state_filter() {
    for raw in [5, u32::MAX] {
        let mut encoded = BytesMut::new();
        primitives::encode_ctx_enumerated(&mut encoded, 0, 0);
        primitives::encode_ctx_enumerated(&mut encoded, 2, raw);
        assert!(matches!(
            GetEnrollmentSummaryRequest::decode(&encoded),
            Err(Error::Reject { reason })
                if reason == bacnet_types::enums::RejectReason::UNDEFINED_ENUMERATION.to_raw()
        ));
    }
    let above_u32 = [0x09, 0, 0x2D, 0x05, 1, 0, 0, 0, 0];
    assert!(matches!(
        GetEnrollmentSummaryRequest::decode(&above_u32),
        Err(Error::Reject { reason })
            if reason == bacnet_types::enums::RejectReason::UNDEFINED_ENUMERATION.to_raw()
    ));
    let above_u64 = [0x09, 0, 0x2D, 0x09, 1, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(matches!(
        GetEnrollmentSummaryRequest::decode(&above_u64),
        Err(Error::Reject { reason })
            if reason == bacnet_types::enums::RejectReason::UNDEFINED_ENUMERATION.to_raw()
    ));

    for noncanonical in [[0x09, 0, 0x28, 0, 0], [0x09, 0, 0x2A, 0, 4]] {
        let encoded = if noncanonical[2] == 0x28 {
            &noncanonical[..3]
        } else {
            &noncanonical[..]
        };
        assert!(matches!(
            GetEnrollmentSummaryRequest::decode(encoded),
            Err(Error::Reject { reason })
                if reason == bacnet_types::enums::RejectReason::INVALID_DATA_ENCODING.to_raw()
        ));
    }

    let request = GetEnrollmentSummaryRequest {
        acknowledgment_filter: 0,
        enrollment_filter: None,
        event_state_filter: Some(EnrollmentSummaryEventStateFilter::from_raw(5)),
        event_type_filter: None,
        priority_filter: None,
        notification_class_filter: None,
    };
    let mut output = BytesMut::new();
    assert!(request.try_encode(&mut output).is_err());
    assert!(output.is_empty());

    let request = GetEnrollmentSummaryRequest {
        event_state_filter: Some(EnrollmentSummaryEventStateFilter::from_raw(u32::MAX)),
        ..request
    };
    assert!(request.try_encode(&mut output).is_err());
    assert!(output.is_empty());
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
