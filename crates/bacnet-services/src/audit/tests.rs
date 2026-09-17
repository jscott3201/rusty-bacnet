use bacnet_encoding::{primitives, tags};
use bacnet_types::bitstring::AuditOperationFlags;
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bacnet_types::enums::{
    AuditOperation, BACnetSuccessFilter, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bacnet_types::MacAddr;
use bytes::BytesMut;

use super::{
    AuditLogQueryAck, AuditLogQueryRequest, AuditNotificationRequest, AuditPropertyReference,
    BACnetAuditLogDatum, BACnetAuditLogQueryParameters, BACnetAuditLogRecord,
    BACnetAuditLogRecordResult, BACnetAuditNotification,
};

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn minimal_by_target(success_filter: BACnetSuccessFilter) -> AuditLogQueryRequest {
    AuditLogQueryRequest {
        audit_log: oid(ObjectType::AUDIT_LOG, 1),
        query_parameters: BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier: oid(ObjectType::DEVICE, 2),
            target_device_address: None,
            target_object_identifier: None,
            target_property_identifier: None,
            target_array_index: None,
            target_priority: None,
            operations: None,
            successful_actions_only: success_filter,
        },
        start_at_sequence_number: None,
        requested_count: 5,
    }
}

fn minimal_by_source(
    success_filter: BACnetSuccessFilter,
    cursor: Option<u64>,
) -> AuditLogQueryRequest {
    AuditLogQueryRequest {
        audit_log: oid(ObjectType::AUDIT_LOG, 1),
        query_parameters: BACnetAuditLogQueryParameters::BySource {
            source_device_identifier: oid(ObjectType::DEVICE, 2),
            source_device_address: None,
            source_object_identifier: None,
            operations: None,
            successful_actions_only: success_filter,
        },
        start_at_sequence_number: cursor,
        requested_count: 513,
    }
}

#[test]
fn audit_notification_minimum_matches_clause_21_golden() {
    let request = AuditNotificationRequest {
        notifications: vec![BACnetAuditNotification {
            source_timestamp: None,
            target_timestamp: None,
            source_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 1)),
            source_object: None,
            operation: AuditOperation::READ,
            source_comment: None,
            target_comment: None,
            invoke_id: None,
            source_user_id: None,
            source_user_role: None,
            target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 2)),
            target_object: None,
            target_property: None,
            target_priority: None,
            target_value: None,
            current_value: None,
            result: None,
        }],
    };
    let mut encoded = BytesMut::new();
    request.try_encode(&mut encoded).unwrap();

    assert_eq!(
        encoded.as_ref(),
        &[
            0x0e, // notifications [0]
            0x2e, 0x0c, 0x02, 0x00, 0x00, 0x01, 0x2f, // source-device [2]
            0x49, 0x00, // operation [4]
            0xae, 0x0c, 0x02, 0x00, 0x00, 0x02, 0xaf, // target-device [10]
            0x0f, // end notifications
        ]
    );
    assert_eq!(AuditNotificationRequest::decode(&encoded).unwrap(), request);
}

/// Corrected-baseline known-answer vectors (Errata 2024-04-29 items 7/8):
/// both query choices with every `BACnetSuccessFilter` value, compared as
/// hand-written expected bytes against the selected production — not a
/// round-trip agreement check. The filter is a context-tagged ENUMERATED at
/// [7] (by-target) / [4] (by-source); the cursor is an Unsigned64 at [2].
#[test]
fn success_filter_goldens_cover_both_choices_and_all_three_values() {
    let filters = [
        (BACnetSuccessFilter::ALL, 0x00),
        (BACnetSuccessFilter::SUCCESSES_ONLY, 0x01),
        (BACnetSuccessFilter::FAILURES_ONLY, 0x02),
    ];
    for (filter, raw) in filters {
        let request = minimal_by_target(filter);
        let mut encoded = BytesMut::new();
        request.try_encode(&mut encoded).unwrap();
        assert_eq!(
            encoded.as_ref(),
            &[
                0x0c, 0x0f, 0x40, 0x00, 0x01, // audit-log [0]
                0x1e, // query-parameters [1]
                0x0e, // by-target [0]
                0x0c, 0x02, 0x00, 0x00, 0x02, // target-device-identifier [0]
                0x79, raw,  // success-filter [7] ENUMERATED
                0x0f, // end by-target
                0x1f, // end query-parameters
                0x39, 0x05, // requested-count [3]
            ]
        );
        assert_eq!(AuditLogQueryRequest::decode(&encoded).unwrap(), request);

        let request = minimal_by_source(filter, Some(0x0102_0304));
        let mut encoded = BytesMut::new();
        request.try_encode(&mut encoded).unwrap();
        assert_eq!(
            encoded.as_ref(),
            &[
                0x0c, 0x0f, 0x40, 0x00, 0x01, // audit-log [0]
                0x1e, // query-parameters [1]
                0x1e, // by-source [1]
                0x0c, 0x02, 0x00, 0x00, 0x02, // source-device-identifier [0]
                0x49, raw,  // success-filter [4] ENUMERATED
                0x1f, // end by-source
                0x1f, // end query-parameters
                0x2c, 0x01, 0x02, 0x03, 0x04, // start-at-sequence-number [2]
                0x3a, 0x02, 0x01, // requested-count [3]
            ]
        );
        assert_eq!(AuditLogQueryRequest::decode(&encoded).unwrap(), request);
    }
}

#[test]
fn by_target_all_optional_fields_match_corrected_golden() {
    let flags = AuditOperationFlags::from_bits((1 << 0) | (1 << 8) | (1u64 << 63)).unwrap();
    let request = AuditLogQueryRequest {
        audit_log: oid(ObjectType::AUDIT_LOG, 44),
        query_parameters: BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier: oid(ObjectType::DEVICE, 1001),
            target_device_address: Some(BACnetAddress {
                network_number: 0x1234,
                mac_address: MacAddr::from_slice(&[192, 0, 2, 10, 0xba, 0xc0]),
            }),
            target_object_identifier: Some(oid(ObjectType::ANALOG_VALUE, 7)),
            target_property_identifier: Some(PropertyIdentifier::PRESENT_VALUE),
            target_array_index: Some(u64::MAX),
            target_priority: Some(16),
            operations: Some(flags),
            successful_actions_only: BACnetSuccessFilter::FAILURES_ONLY,
        },
        start_at_sequence_number: Some(u64::MAX),
        requested_count: u16::MAX,
    };

    let mut encoded = BytesMut::new();
    request.try_encode(&mut encoded).unwrap();
    assert_eq!(AuditLogQueryRequest::decode(&encoded).unwrap(), request);

    // Corrected filter form: failures-only is ENUMERATED 2 at [7].
    assert!(encoded.windows(2).any(|window| window == [0x79, 0x02]));
    // Corrected Unsigned64 cursor: u64::MAX is the 8-octet extended form.
    assert!(encoded
        .windows(10)
        .any(|window| window == [0x2d, 0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]));
    // Requested-count stays Unsigned16.
    assert!(encoded
        .windows(3)
        .any(|window| window == [0x3a, 0xff, 0xff]));
    // Operations [6] holds unused-bits octet followed by the complete
    // bit-position-preserving 64-bit representation.
    assert!(encoded
        .windows(11)
        .any(|window| window == [0x6d, 0x09, 0x00, 0x80, 0x80, 0, 0, 0, 0, 0, 1]));
}

#[test]
fn by_source_address_object_and_flags_match_corrected_golden() {
    let request = AuditLogQueryRequest {
        audit_log: oid(ObjectType::AUDIT_LOG, 2),
        query_parameters: BACnetAuditLogQueryParameters::BySource {
            source_device_identifier: oid(ObjectType::DEVICE, 3),
            source_device_address: Some(BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::new(),
            }),
            source_object_identifier: Some(oid(ObjectType::BINARY_INPUT, 4)),
            operations: Some(AuditOperationFlags::from_bits((1 << 0) | (1 << 8)).unwrap()),
            successful_actions_only: BACnetSuccessFilter::ALL,
        },
        start_at_sequence_number: None,
        requested_count: 1,
    };

    let mut encoded = BytesMut::new();
    request.try_encode(&mut encoded).unwrap();
    assert_eq!(AuditLogQueryRequest::decode(&encoded).unwrap(), request);
    // Corrected filter form: all is ENUMERATED 0 at [4] with no cursor [2].
    assert!(encoded.windows(2).any(|window| window == [0x49, 0x00]));
    assert!(encoded
        .windows(4)
        .any(|window| window == [0x3b, 0x07, 0x80, 0x80]));
}

#[test]
fn audit_property_reference_shared_conversion_is_checked() {
    let shared = crate::common::PropertyReference {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: Some(u32::MAX),
    };
    let audit = AuditPropertyReference::from(shared.clone());
    assert_eq!(audit.property_array_index, Some(u64::from(u32::MAX)));
    assert_eq!(
        crate::common::PropertyReference::try_from(audit).unwrap(),
        shared
    );

    let too_wide = AuditPropertyReference {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: Some(u64::from(u32::MAX) + 1),
    };
    assert!(crate::common::PropertyReference::try_from(too_wide).is_err());
}

#[test]
fn success_filter_is_strict_enumerated_and_cannot_be_omitted() {
    // The corrected ENUMERATED 0..=2 range: value 2 was rejected as a
    // non-boolean by the uncorrected decoder and must now decode.
    for (filter, raw) in [
        (BACnetSuccessFilter::ALL, 0),
        (BACnetSuccessFilter::SUCCESSES_ONLY, 1),
        (BACnetSuccessFilter::FAILURES_ONLY, 2),
    ] {
        let mut encoded = BytesMut::new();
        minimal_by_target(filter).try_encode(&mut encoded).unwrap();
        let at = encoded
            .windows(2)
            .position(|bytes| bytes == [0x79, raw])
            .unwrap();
        assert_eq!(encoded[at], 0x79);
        let mut missing = encoded.to_vec();
        missing.drain(at..at + 2);
        assert!(AuditLogQueryRequest::decode(&missing).is_err());

        let mut encoded = BytesMut::new();
        minimal_by_source(filter, None)
            .try_encode(&mut encoded)
            .unwrap();
        let at = encoded
            .windows(2)
            .position(|bytes| bytes == [0x49, raw])
            .unwrap();
        let mut missing = encoded.to_vec();
        missing.drain(at..at + 2);
        assert!(AuditLogQueryRequest::decode(&missing).is_err());
    }

    // Out-of-range enumerated values fail on both choices.
    let mut encoded = BytesMut::new();
    minimal_by_target(BACnetSuccessFilter::ALL)
        .try_encode(&mut encoded)
        .unwrap();
    let filter_pos = encoded
        .windows(2)
        .position(|bytes| bytes == [0x79, 0x00])
        .unwrap();
    for raw in [3, 16, 255] {
        let mut out_of_range = encoded.to_vec();
        out_of_range[filter_pos + 1] = raw;
        assert!(AuditLogQueryRequest::decode(&out_of_range).is_err());
    }

    // Zero-length, non-canonical leading-zero, and over-wide (>8 octet)
    // filter encodings fail.
    let mut zero_length = encoded.to_vec();
    zero_length[filter_pos] = 0x78;
    zero_length.remove(filter_pos + 1);
    assert!(AuditLogQueryRequest::decode(&zero_length).is_err());

    let mut noncanonical = encoded.to_vec();
    noncanonical.splice(filter_pos..filter_pos + 2, [0x7a, 0x00, 0x01]);
    assert!(AuditLogQueryRequest::decode(&noncanonical).is_err());

    let mut over_wide = encoded.to_vec();
    let mut wide_field = vec![0x7d, 0x09];
    wide_field.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 1]);
    over_wide.splice(filter_pos..filter_pos + 2, wide_field);
    assert!(AuditLogQueryRequest::decode(&over_wide).is_err());

    // By-source [4] rejects an out-of-range enumerated value as well.
    let mut encoded = BytesMut::new();
    minimal_by_source(BACnetSuccessFilter::ALL, None)
        .try_encode(&mut encoded)
        .unwrap();
    let filter_pos = encoded
        .windows(2)
        .position(|bytes| bytes == [0x49, 0x00])
        .unwrap();
    let mut out_of_range = encoded.to_vec();
    out_of_range[filter_pos + 1] = 3;
    assert!(AuditLogQueryRequest::decode(&out_of_range).is_err());
}

/// Corrected Unsigned64 cursor boundaries (Errata 2024-04-29 item 8):
/// hand-written field bytes against the selected production plus decode
/// round-trips. The 5-octet `u32::MAX + 1` cursor was rejected by the
/// uncorrected Unsigned32 decoder and must now encode and decode.
#[test]
fn corrected_cursor_boundaries_encode_as_unsigned64() {
    let boundaries: [(u64, &[u8]); 4] = [
        (0, &[0x29, 0x00]),
        (u64::from(u32::MAX), &[0x2c, 0xff, 0xff, 0xff, 0xff]),
        (
            u64::from(u32::MAX) + 1,
            &[0x2d, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            u64::MAX,
            &[0x2d, 0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        ),
    ];
    for (cursor, field) in boundaries {
        let request = minimal_by_source(BACnetSuccessFilter::ALL, Some(cursor));
        let mut encoded = BytesMut::new();
        request.try_encode(&mut encoded).unwrap();
        assert!(
            encoded.windows(field.len()).any(|window| window == field),
            "cursor {cursor} missing field bytes {field:02x?}"
        );
        assert_eq!(AuditLogQueryRequest::decode(&encoded).unwrap(), request);
    }

    // Absent versus present cursor: the minimal by-target request carries no
    // [2] field, while Some(0) encodes the one-octet zero form.
    let mut absent = BytesMut::new();
    minimal_by_target(BACnetSuccessFilter::ALL)
        .try_encode(&mut absent)
        .unwrap();
    assert!(!absent.windows(1).any(|window| window == [0x29]));
    let mut present = BytesMut::new();
    minimal_by_source(BACnetSuccessFilter::ALL, Some(0))
        .try_encode(&mut present)
        .unwrap();
    assert!(present.windows(2).any(|window| window == [0x29, 0x00]));
}

#[test]
fn corrected_cursor_rejects_noncanonical_overwide_and_count_overflow() {
    // Over-wide (>8 octet) and non-canonical leading-zero cursors fail.
    let mut over_wide = BytesMut::new();
    encode_request_prefix(&mut over_wide);
    tags::encode_tag(&mut over_wide, 2, tags::TagClass::Context, 9);
    over_wide.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 1]);
    primitives::encode_ctx_unsigned(&mut over_wide, 3, 1);
    assert!(AuditLogQueryRequest::decode(&over_wide).is_err());

    let mut noncanonical_start = BytesMut::new();
    encode_request_prefix(&mut noncanonical_start);
    tags::encode_tag(&mut noncanonical_start, 2, tags::TagClass::Context, 2);
    noncanonical_start.extend_from_slice(&[0, 1]);
    primitives::encode_ctx_unsigned(&mut noncanonical_start, 3, 1);
    assert!(AuditLogQueryRequest::decode(&noncanonical_start).is_err());

    // Requested-count stays Unsigned16: over-wide and non-canonical fail.
    let mut too_wide_count = BytesMut::new();
    encode_request_prefix(&mut too_wide_count);
    primitives::encode_ctx_unsigned(&mut too_wide_count, 3, 0x1_0000);
    assert!(AuditLogQueryRequest::decode(&too_wide_count).is_err());

    let mut noncanonical_count = BytesMut::new();
    encode_request_prefix(&mut noncanonical_count);
    tags::encode_tag(&mut noncanonical_count, 3, tags::TagClass::Context, 2);
    noncanonical_count.extend_from_slice(&[0, 1]);
    assert!(AuditLogQueryRequest::decode(&noncanonical_count).is_err());
}

#[test]
fn nested_field_widths_and_priority_range_are_enforced() {
    let mut too_wide_address = BytesMut::new();
    encode_outer_prefix(&mut too_wide_address, 0);
    primitives::encode_ctx_object_id(&mut too_wide_address, 0, &oid(ObjectType::DEVICE, 2));
    tags::encode_opening_tag(&mut too_wide_address, 1);
    primitives::encode_app_unsigned(&mut too_wide_address, 0x1_0000);
    primitives::encode_app_octet_string(&mut too_wide_address, &[]);
    tags::encode_closing_tag(&mut too_wide_address, 1);
    primitives::encode_ctx_enumerated(&mut too_wide_address, 7, 0);
    encode_outer_suffix(&mut too_wide_address, 0);
    assert!(AuditLogQueryRequest::decode(&too_wide_address).is_err());

    let mut too_wide_array = BytesMut::new();
    encode_outer_prefix(&mut too_wide_array, 0);
    primitives::encode_ctx_object_id(&mut too_wide_array, 0, &oid(ObjectType::DEVICE, 2));
    tags::encode_tag(&mut too_wide_array, 4, tags::TagClass::Context, 9);
    too_wide_array.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0]);
    primitives::encode_ctx_enumerated(&mut too_wide_array, 7, 0);
    encode_outer_suffix(&mut too_wide_array, 0);
    assert!(AuditLogQueryRequest::decode(&too_wide_array).is_err());

    let mut bad_priority = BytesMut::new();
    encode_outer_prefix(&mut bad_priority, 0);
    primitives::encode_ctx_object_id(&mut bad_priority, 0, &oid(ObjectType::DEVICE, 2));
    primitives::encode_ctx_unsigned(&mut bad_priority, 5, 17);
    primitives::encode_ctx_enumerated(&mut bad_priority, 7, 0);
    encode_outer_suffix(&mut bad_priority, 0);
    assert!(AuditLogQueryRequest::decode(&bad_priority).is_err());

    let mut noncanonical_address = BytesMut::new();
    encode_outer_prefix(&mut noncanonical_address, 0);
    primitives::encode_ctx_object_id(&mut noncanonical_address, 0, &oid(ObjectType::DEVICE, 2));
    tags::encode_opening_tag(&mut noncanonical_address, 1);
    noncanonical_address.extend_from_slice(&[0x22, 0, 1]);
    primitives::encode_app_octet_string(&mut noncanonical_address, &[]);
    tags::encode_closing_tag(&mut noncanonical_address, 1);
    primitives::encode_ctx_enumerated(&mut noncanonical_address, 7, 0);
    encode_outer_suffix(&mut noncanonical_address, 0);
    assert!(AuditLogQueryRequest::decode(&noncanonical_address).is_err());

    for (tag, field) in [(3, [0x00, 0x55]), (4, [0x00, 0x01]), (5, [0x00, 0x01])] {
        let mut malformed = BytesMut::new();
        encode_outer_prefix(&mut malformed, 0);
        primitives::encode_ctx_object_id(&mut malformed, 0, &oid(ObjectType::DEVICE, 2));
        tags::encode_tag(&mut malformed, tag, tags::TagClass::Context, 2);
        malformed.extend_from_slice(&field);
        primitives::encode_ctx_enumerated(&mut malformed, 7, 0);
        encode_outer_suffix(&mut malformed, 0);
        assert!(AuditLogQueryRequest::decode(&malformed).is_err());
    }
}

#[test]
fn operation_flags_reject_more_than_64_bits_and_nonzero_padding() {
    let mut too_wide = BytesMut::new();
    encode_outer_prefix(&mut too_wide, 1);
    primitives::encode_ctx_object_id(&mut too_wide, 0, &oid(ObjectType::DEVICE, 2));
    primitives::encode_ctx_bit_string(&mut too_wide, 3, 7, &[0; 9]);
    primitives::encode_ctx_enumerated(&mut too_wide, 4, 0);
    encode_outer_suffix(&mut too_wide, 1);
    assert!(AuditLogQueryRequest::decode(&too_wide).is_err());

    let mut bad_padding = BytesMut::new();
    encode_outer_prefix(&mut bad_padding, 1);
    primitives::encode_ctx_object_id(&mut bad_padding, 0, &oid(ObjectType::DEVICE, 2));
    primitives::encode_ctx_bit_string(&mut bad_padding, 3, 1, &[0x01]);
    primitives::encode_ctx_enumerated(&mut bad_padding, 4, 0);
    encode_outer_suffix(&mut bad_padding, 1);
    assert!(AuditLogQueryRequest::decode(&bad_padding).is_err());
}

#[test]
fn every_constructed_level_requires_full_consumption() {
    let mut top_level = BytesMut::new();
    minimal_by_target(BACnetSuccessFilter::ALL)
        .try_encode(&mut top_level)
        .unwrap();
    top_level.extend_from_slice(&[0x39, 0x01]);
    assert!(AuditLogQueryRequest::decode(&top_level).is_err());

    let mut trailing_choice = BytesMut::new();
    primitives::encode_ctx_object_id(&mut trailing_choice, 0, &oid(ObjectType::AUDIT_LOG, 1));
    tags::encode_opening_tag(&mut trailing_choice, 1);
    tags::encode_opening_tag(&mut trailing_choice, 0);
    primitives::encode_ctx_object_id(&mut trailing_choice, 0, &oid(ObjectType::DEVICE, 2));
    primitives::encode_ctx_enumerated(&mut trailing_choice, 7, 0);
    tags::encode_closing_tag(&mut trailing_choice, 0);
    primitives::encode_ctx_unsigned(&mut trailing_choice, 9, 1);
    tags::encode_closing_tag(&mut trailing_choice, 1);
    primitives::encode_ctx_unsigned(&mut trailing_choice, 3, 1);
    assert!(AuditLogQueryRequest::decode(&trailing_choice).is_err());

    // Trailing field inside by-target after the required [7] filter.
    let mut trailing_by_target = BytesMut::new();
    encode_outer_prefix(&mut trailing_by_target, 0);
    primitives::encode_ctx_object_id(&mut trailing_by_target, 0, &oid(ObjectType::DEVICE, 2));
    primitives::encode_ctx_enumerated(&mut trailing_by_target, 7, 1);
    primitives::encode_ctx_unsigned(&mut trailing_by_target, 9, 1);
    encode_outer_suffix(&mut trailing_by_target, 0);
    assert!(AuditLogQueryRequest::decode(&trailing_by_target).is_err());

    // Trailing field inside by-source after the required [4] filter.
    let mut trailing_by_source = BytesMut::new();
    encode_outer_prefix(&mut trailing_by_source, 1);
    primitives::encode_ctx_object_id(&mut trailing_by_source, 0, &oid(ObjectType::DEVICE, 2));
    primitives::encode_ctx_enumerated(&mut trailing_by_source, 4, 2);
    primitives::encode_ctx_unsigned(&mut trailing_by_source, 9, 1);
    encode_outer_suffix(&mut trailing_by_source, 1);
    assert!(AuditLogQueryRequest::decode(&trailing_by_source).is_err());

    // Trailing application value inside the by-target device address.
    let mut trailing_address = BytesMut::new();
    encode_outer_prefix(&mut trailing_address, 0);
    primitives::encode_ctx_object_id(&mut trailing_address, 0, &oid(ObjectType::DEVICE, 2));
    tags::encode_opening_tag(&mut trailing_address, 1);
    primitives::encode_app_unsigned(&mut trailing_address, 1);
    primitives::encode_app_octet_string(&mut trailing_address, &[]);
    primitives::encode_app_unsigned(&mut trailing_address, 2);
    tags::encode_closing_tag(&mut trailing_address, 1);
    primitives::encode_ctx_enumerated(&mut trailing_address, 7, 0);
    encode_outer_suffix(&mut trailing_address, 0);
    assert!(AuditLogQueryRequest::decode(&trailing_address).is_err());
}

#[test]
fn invalid_choice_discriminator_is_rejected() {
    let mut wrong_choice = BytesMut::new();
    primitives::encode_ctx_object_id(&mut wrong_choice, 0, &oid(ObjectType::AUDIT_LOG, 1));
    tags::encode_opening_tag(&mut wrong_choice, 1);
    tags::encode_opening_tag(&mut wrong_choice, 2);
    primitives::encode_ctx_object_id(&mut wrong_choice, 0, &oid(ObjectType::DEVICE, 2));
    primitives::encode_ctx_enumerated(&mut wrong_choice, 7, 0);
    tags::encode_closing_tag(&mut wrong_choice, 2);
    tags::encode_closing_tag(&mut wrong_choice, 1);
    primitives::encode_ctx_unsigned(&mut wrong_choice, 3, 1);
    assert!(AuditLogQueryRequest::decode(&wrong_choice).is_err());

    // Wrong audit-log tag ([1] instead of [0]) is rejected.
    let mut wrong_audit_log = BytesMut::new();
    minimal_by_target(BACnetSuccessFilter::ALL)
        .try_encode(&mut wrong_audit_log)
        .unwrap();
    wrong_audit_log[0] = 0x1c;
    assert!(AuditLogQueryRequest::decode(&wrong_audit_log).is_err());
}

#[test]
fn invalid_priority_encode_is_atomic() {
    let mut request = minimal_by_target(BACnetSuccessFilter::SUCCESSES_ONLY);
    let BACnetAuditLogQueryParameters::ByTarget {
        target_priority, ..
    } = &mut request.query_parameters
    else {
        unreachable!();
    };
    *target_priority = Some(0);

    let mut output = BytesMut::from(&b"prefix"[..]);
    assert!(request.try_encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");
}

#[test]
fn invalid_filter_encode_is_atomic() {
    // Every u64 cursor value encodes, so cursor rejection is decode-side
    // (see corrected_cursor_rejects_noncanonical_overwide_and_count_overflow);
    // the atomic-encode failures are bad priority and bad filter.
    for raw in [3, 16, u32::MAX] {
        let mut request = minimal_by_source(BACnetSuccessFilter::ALL, None);
        let BACnetAuditLogQueryParameters::BySource {
            successful_actions_only,
            ..
        } = &mut request.query_parameters
        else {
            unreachable!();
        };
        *successful_actions_only = BACnetSuccessFilter::from_raw(raw);

        let mut output = BytesMut::from(&b"prefix"[..]);
        assert!(request.try_encode(&mut output).is_err());
        assert_eq!(output.as_ref(), b"prefix");
    }
}

#[test]
fn wide_cursor_pairs_with_wide_ack_sequence() {
    // The ACK sequence was already Unsigned64; the corrected query cursor
    // now pairs with it above 32 bits.
    let cursor = u64::from(u32::MAX) + 1;
    let request = minimal_by_source(BACnetSuccessFilter::FAILURES_ONLY, Some(cursor));
    let mut encoded = BytesMut::new();
    request.try_encode(&mut encoded).unwrap();
    assert!(encoded
        .windows(7)
        .any(|window| window == [0x2d, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00]));
    assert_eq!(AuditLogQueryRequest::decode(&encoded).unwrap(), request);

    let ack = AuditLogQueryAck {
        audit_log: oid(ObjectType::AUDIT_LOG, 1),
        records: vec![BACnetAuditLogRecordResult {
            sequence_number: cursor,
            record: audit_record(BACnetAuditLogDatum::LogStatus(0b010)),
        }],
        no_more_items: false,
    };
    let mut encoded = BytesMut::new();
    ack.try_encode(&mut encoded).unwrap();
    assert!(encoded
        .windows(7)
        .any(|window| window == [0x0d, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00]));
    assert_eq!(AuditLogQueryAck::decode(&encoded).unwrap(), ack);
}

fn encode_request_prefix(buf: &mut BytesMut) {
    encode_outer_prefix(buf, 0);
    primitives::encode_ctx_object_id(buf, 0, &oid(ObjectType::DEVICE, 2));
    primitives::encode_ctx_enumerated(buf, 7, 0);
    tags::encode_closing_tag(buf, 0);
    tags::encode_closing_tag(buf, 1);
}

fn encode_outer_prefix(buf: &mut BytesMut, choice: u8) {
    primitives::encode_ctx_object_id(buf, 0, &oid(ObjectType::AUDIT_LOG, 1));
    tags::encode_opening_tag(buf, 1);
    tags::encode_opening_tag(buf, choice);
}

fn encode_outer_suffix(buf: &mut BytesMut, choice: u8) {
    tags::encode_closing_tag(buf, choice);
    tags::encode_closing_tag(buf, 1);
    primitives::encode_ctx_unsigned(buf, 3, 1);
}

fn audit_record(datum: BACnetAuditLogDatum) -> BACnetAuditLogRecord {
    BACnetAuditLogRecord {
        timestamp: (
            Date {
                year: 126,
                month: 8,
                day: 29,
                day_of_week: 6,
            },
            Time {
                hour: 12,
                minute: 34,
                second: 56,
                hundredths: 78,
            },
        ),
        datum,
    }
}

fn ack_notification() -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 1)),
        source_object: None,
        operation: AuditOperation::WRITE,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 2)),
        target_object: None,
        target_property: None,
        target_priority: None,
        target_value: None,
        current_value: None,
        result: Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)),
    }
}

#[test]
fn audit_log_query_ack_empty_matches_clause_21_literal() {
    let ack = AuditLogQueryAck {
        audit_log: oid(ObjectType::AUDIT_LOG, 1),
        records: Vec::new(),
        no_more_items: true,
    };
    let mut encoded = BytesMut::new();
    ack.try_encode(&mut encoded).unwrap();

    assert_eq!(
        encoded.as_ref(),
        &[0x0c, 0x0f, 0x40, 0x00, 0x01, 0x1e, 0x1f, 0x29, 0x01]
    );
    assert_eq!(AuditLogQueryAck::decode(&encoded).unwrap(), ack);
}

#[test]
fn audit_log_query_ack_log_status_matches_clause_21_literal() {
    let ack = AuditLogQueryAck {
        audit_log: oid(ObjectType::AUDIT_LOG, 1),
        records: vec![BACnetAuditLogRecordResult {
            sequence_number: 1,
            record: audit_record(BACnetAuditLogDatum::LogStatus(0b010)),
        }],
        no_more_items: false,
    };
    let expected = [
        0x0c, 0x0f, 0x40, 0x00, 0x01, // audit-log [0]
        0x1e, // list-of-records [1]
        0x09, 0x01, // sequence-number [0]
        0x1e, // record [1]
        0x0e, 0xa4, 126, 8, 29, 6, 0xb4, 12, 34, 56, 78, 0x0f, // timestamp [0]
        0x1e, 0x0a, 5, 0x40, 0x1f, // datum [1], log-status [0]
        0x1f, // end record
        0x1f, // end list
        0x29, 0x00, // no-more-items [2]
    ];

    let mut encoded = BytesMut::new();
    ack.try_encode(&mut encoded).unwrap();
    assert_eq!(encoded.as_ref(), expected);
    assert_eq!(AuditLogQueryAck::decode(&expected).unwrap(), ack);
}

#[test]
fn audit_log_query_ack_nested_notification_with_error_matches_literal() {
    let notification = ack_notification();
    let ack = AuditLogQueryAck {
        audit_log: oid(ObjectType::AUDIT_LOG, 1),
        records: vec![BACnetAuditLogRecordResult {
            sequence_number: 2,
            record: audit_record(BACnetAuditLogDatum::AuditNotification(notification.clone())),
        }],
        no_more_items: true,
    };
    let expected = [
        0x0c, 0x0f, 0x40, 0x00, 0x01, 0x1e, 0x09, 0x02, 0x1e, 0x0e, 0xa4, 126, 8, 29, 6, 0xb4, 12,
        34, 56, 78, 0x0f, 0x1e, // datum [1]
        0x1e, // notification alternative [1]
        0x2e, 0x0c, 0x02, 0x00, 0x00, 0x01, 0x2f, // source-device
        0x49, 0x01, // WRITE operation
        0xae, 0x0c, 0x02, 0x00, 0x00, 0x02, 0xaf, // target-device
        0xfe, 16, 0x91, 0x02, 0x91, 0x28, 0xff, 16, // optional Error [16]
        0x1f, 0x1f, 0x1f, 0x1f, 0x29, 0x01,
    ];

    let mut encoded = BytesMut::new();
    ack.encode(&mut encoded).unwrap();
    assert_eq!(encoded.as_ref(), expected);
    assert_eq!(AuditLogQueryAck::decode(&expected).unwrap(), ack);
}

#[test]
fn audit_log_query_ack_time_change_and_adjacent_results_match_literal() {
    let ack = AuditLogQueryAck {
        audit_log: oid(ObjectType::AUDIT_LOG, 1),
        records: vec![
            BACnetAuditLogRecordResult {
                sequence_number: u64::MAX,
                record: audit_record(BACnetAuditLogDatum::TimeChange(1.5)),
            },
            BACnetAuditLogRecordResult {
                sequence_number: 0,
                record: audit_record(BACnetAuditLogDatum::LogStatus(0)),
            },
        ],
        no_more_items: false,
    };
    let expected = [
        0x0c, 0x0f, 0x40, 0x00, 0x01, 0x1e, 0x0d, 0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x1e, 0x0e, 0xa4, 126, 8, 29, 6, 0xb4, 12, 34, 56, 78, 0x0f, 0x1e, 0x2c, 0x3f, 0xc0,
        0x00, 0x00, 0x1f, 0x1f, 0x09, 0x00, 0x1e, 0x0e, 0xa4, 126, 8, 29, 6, 0xb4, 12, 34, 56, 78,
        0x0f, 0x1e, 0x0a, 5, 0x00, 0x1f, 0x1f, 0x1f, 0x29, 0x00,
    ];

    let mut encoded = BytesMut::new();
    ack.encode(&mut encoded).unwrap();
    assert_eq!(encoded.as_ref(), expected);
    assert_eq!(AuditLogQueryAck::decode(&expected).unwrap(), ack);
}

#[test]
fn audit_log_query_ack_encode_is_atomic() {
    let ack = AuditLogQueryAck {
        audit_log: oid(ObjectType::AUDIT_LOG, 1),
        records: vec![BACnetAuditLogRecordResult {
            sequence_number: 1,
            record: audit_record(BACnetAuditLogDatum::LogStatus(0b1000)),
        }],
        no_more_items: false,
    };
    let mut output = BytesMut::from(&b"prefix"[..]);
    assert!(ack.try_encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");
}
