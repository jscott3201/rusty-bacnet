use bacnet_encoding::constructed::{check_decoded_mac_len, check_encoded_mac_len};
use bacnet_encoding::{primitives, tags};
use bacnet_types::bitstring::AuditOperationFlags;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{BACnetSuccessFilter, PropertyIdentifier, RejectReason};
use bacnet_types::error::{DecodeError, DecodingKind, EncodeError, Error};
use bacnet_types::MacAddr;
use bytes::BytesMut;

use bacnet_encoding::constructed::tagged::{
    decode_app_primitive, decode_canonical_unsigned, decode_ctx_canonical_unsigned,
    decode_ctx_object_id, decode_ctx_primitive, expect_end, misplaced, misplaced_tag, next_is_context,
    next_is_opening,
};

use super::{AuditLogQueryRequest, BACnetAuditLogQueryParameters};

fn validate_filter(filter: BACnetSuccessFilter) -> Result<(), Error> {
    if filter != BACnetSuccessFilter::ALL
        && filter != BACnetSuccessFilter::SUCCESSES_ONLY
        && filter != BACnetSuccessFilter::FAILURES_ONLY
    {
        return Err(EncodeError::new("AuditLogQuery success-filter {} is outside 0..=2")
            .with_value(filter.to_raw())
            .into());
    }
    Ok(())
}

fn validate(request: &AuditLogQueryRequest) -> Result<(), Error> {
    if let BACnetAuditLogQueryParameters::ByTarget {
        target_priority: Some(priority),
        ..
    } = &request.query_parameters
        && !(1..=16).contains(priority) {
            return Err(Error::encode("AuditLogQuery target-priority must be in 1..=16"));
        }
    let (address, filter) = match &request.query_parameters {
        BACnetAuditLogQueryParameters::ByTarget {
            target_device_address: address,
            successful_actions_only,
            ..
        }
        | BACnetAuditLogQueryParameters::BySource {
            source_device_address: address,
            successful_actions_only,
            ..
        } => (address, *successful_actions_only),
    };
    if let Some(address) = address {
        check_encoded_mac_len(&address.mac_address, "AuditLogQuery device address")?;
    }
    validate_filter(filter)
}

pub(super) fn encode(request: &AuditLogQueryRequest, buf: &mut BytesMut) -> Result<(), Error> {
    validate(request)?;

    // Build the complete payload separately so validation failures cannot
    // leave a partial service request in the caller's buffer.
    let mut encoded = BytesMut::new();
    primitives::encode_ctx_object_id(&mut encoded, 0, &request.audit_log);

    tags::encode_opening_tag(&mut encoded, 1);
    match &request.query_parameters {
        BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier,
            target_device_address,
            target_object_identifier,
            target_property_identifier,
            target_array_index,
            target_priority,
            operations,
            successful_actions_only,
        } => {
            tags::encode_opening_tag(&mut encoded, 0);
            primitives::encode_ctx_object_id(&mut encoded, 0, target_device_identifier);
            if let Some(address) = target_device_address {
                encode_address(&mut encoded, 1, address);
            }
            if let Some(object) = target_object_identifier {
                primitives::encode_ctx_object_id(&mut encoded, 2, object);
            }
            if let Some(property) = target_property_identifier {
                primitives::encode_ctx_enumerated(&mut encoded, 3, property.to_raw());
            }
            if let Some(index) = target_array_index {
                primitives::encode_ctx_unsigned(&mut encoded, 4, *index);
            }
            if let Some(priority) = target_priority {
                primitives::encode_ctx_unsigned(&mut encoded, 5, u64::from(*priority));
            }
            if let Some(flags) = operations {
                let (unused_bits, data) = flags.to_bacnet();
                primitives::encode_ctx_bit_string(&mut encoded, 6, unused_bits, &data);
            }
            primitives::encode_ctx_enumerated(&mut encoded, 7, successful_actions_only.to_raw());
            tags::encode_closing_tag(&mut encoded, 0);
        }
        BACnetAuditLogQueryParameters::BySource {
            source_device_identifier,
            source_device_address,
            source_object_identifier,
            operations,
            successful_actions_only,
        } => {
            tags::encode_opening_tag(&mut encoded, 1);
            primitives::encode_ctx_object_id(&mut encoded, 0, source_device_identifier);
            if let Some(address) = source_device_address {
                encode_address(&mut encoded, 1, address);
            }
            if let Some(object) = source_object_identifier {
                primitives::encode_ctx_object_id(&mut encoded, 2, object);
            }
            if let Some(flags) = operations {
                let (unused_bits, data) = flags.to_bacnet();
                primitives::encode_ctx_bit_string(&mut encoded, 3, unused_bits, &data);
            }
            primitives::encode_ctx_enumerated(&mut encoded, 4, successful_actions_only.to_raw());
            tags::encode_closing_tag(&mut encoded, 1);
        }
    }
    tags::encode_closing_tag(&mut encoded, 1);

    if let Some(sequence) = request.start_at_sequence_number {
        primitives::encode_ctx_unsigned(&mut encoded, 2, sequence);
    }
    primitives::encode_ctx_unsigned(&mut encoded, 3, u64::from(request.requested_count));

    buf.extend_from_slice(&encoded);
    Ok(())
}

fn encode_address(buf: &mut BytesMut, tag: u8, address: &BACnetAddress) {
    tags::encode_opening_tag(buf, tag);
    primitives::encode_app_unsigned(buf, u64::from(address.network_number));
    primitives::encode_app_octet_string(buf, &address.mac_address);
    tags::encode_closing_tag(buf, tag);
}

pub(super) fn decode(data: &[u8]) -> Result<AuditLogQueryRequest, Error> {
    let (audit_log, mut offset) = decode_ctx_object_id(data, 0, 0, "AuditLogQuery audit-log")?;

    let (wrapper_tag, wrapper_start) = tags::decode_tag(data, offset)?;
    if !wrapper_tag.is_opening_tag(1) {
        return Err(misplaced_tag(
            data,
            &wrapper_tag,
            Some(1),
            offset,
            "AuditLogQuery query-parameters expected opening tag 1",
        ));
    }
    let (wrapper, wrapper_end) = tags::extract_context_value(data, wrapper_start, 1)?;
    let query_parameters = decode_query_parameters(wrapper)?;
    offset = wrapper_end;

    let mut start_at_sequence_number = None;
    if next_is_context(data, offset, 2)? {
        let (sequence, end) = decode_ctx_canonical_unsigned::<u64>(
            data,
            offset,
            2,
            "AuditLogQuery start-at-sequence-number",
        )?;
        start_at_sequence_number = Some(sequence);
        offset = end;
    }

    let (requested_count, end) =
        decode_ctx_canonical_unsigned::<u16>(data, offset, 3, "AuditLogQuery requested-count")?;
    offset = end;
    expect_end(data, offset, offset, "AuditLogQuery")?;

    Ok(AuditLogQueryRequest {
        audit_log,
        query_parameters,
        start_at_sequence_number,
        requested_count,
    })
}

fn decode_query_parameters(data: &[u8]) -> Result<BACnetAuditLogQueryParameters, Error> {
    let (choice, content_start) = tags::decode_tag(data, 0)?;
    let (parameters, choice_end) = if choice.is_opening_tag(0) {
        let (content, end) = tags::extract_context_value(data, content_start, 0)?;
        (decode_by_target(content)?, end)
    } else if choice.is_opening_tag(1) {
        let (content, end) = tags::extract_context_value(data, content_start, 1)?;
        (decode_by_source(content)?, end)
    } else {
        return Err(misplaced_tag(
            data,
            &choice,
            None,
            0,
            "AuditLogQuery query-parameters expected by-target [0] or by-source [1]",
        ));
    };
    expect_end(
        data,
        choice_end,
        choice_end,
        "AuditLogQuery query-parameters",
    )?;
    Ok(parameters)
}

fn decode_by_target(data: &[u8]) -> Result<BACnetAuditLogQueryParameters, Error> {
    let (target_device_identifier, mut offset) = decode_ctx_object_id(
        data,
        0,
        0,
        "AuditLogQuery by-target target-device-identifier",
    )?;

    let mut target_device_address = None;
    if next_is_opening(data, offset, 1)? {
        let (address, end) = decode_address(data, offset, 1, &BY_TARGET_ADDRESS)?;
        target_device_address = Some(address);
        offset = end;
    }

    let mut target_object_identifier = None;
    if next_is_context(data, offset, 2)? {
        let (object, end) = decode_ctx_object_id(
            data,
            offset,
            2,
            "AuditLogQuery by-target target-object-identifier",
        )?;
        target_object_identifier = Some(object);
        offset = end;
    }

    let mut target_property_identifier = None;
    if next_is_context(data, offset, 3)? {
        let (property, end) = decode_ctx_canonical_unsigned::<u32>(
            data,
            offset,
            3,
            "AuditLogQuery by-target target-property-identifier",
        )?;
        target_property_identifier = Some(PropertyIdentifier::from_raw(property));
        offset = end;
    }

    let mut target_array_index = None;
    if next_is_context(data, offset, 4)? {
        let (index, end) = decode_ctx_canonical_unsigned::<u64>(
            data,
            offset,
            4,
            "AuditLogQuery by-target target-array-index",
        )?;
        target_array_index = Some(index);
        offset = end;
    }

    let mut target_priority = None;
    if next_is_context(data, offset, 5)? {
        let (priority, end) = decode_ctx_canonical_unsigned::<u8>(
            data,
            offset,
            5,
            "AuditLogQuery by-target target-priority",
        )?;
        if !(1..=16).contains(&priority) {
            return Err(Error::out_of_range(
                offset,
                "AuditLogQuery target-priority must be in 1..=16",
            ));
        }
        target_priority = Some(priority);
        offset = end;
    }

    let mut operations = None;
    if next_is_context(data, offset, 6)? {
        let (flags, end) =
            decode_operation_flags(data, offset, 6, "AuditLogQuery by-target operations")?;
        operations = Some(flags);
        offset = end;
    }

    let (successful_actions_only, offset) = decode_success_filter(
        data,
        offset,
        7,
        "AuditLogQuery by-target successful-actions-only",
    )?;
    expect_end(data, offset, offset, "AuditLogQuery by-target")?;

    Ok(BACnetAuditLogQueryParameters::ByTarget {
        target_device_identifier,
        target_device_address,
        target_object_identifier,
        target_property_identifier,
        target_array_index,
        target_priority,
        operations,
        successful_actions_only,
    })
}

fn decode_by_source(data: &[u8]) -> Result<BACnetAuditLogQueryParameters, Error> {
    let (source_device_identifier, mut offset) = decode_ctx_object_id(
        data,
        0,
        0,
        "AuditLogQuery by-source source-device-identifier",
    )?;

    let mut source_device_address = None;
    if next_is_opening(data, offset, 1)? {
        let (address, end) = decode_address(data, offset, 1, &BY_SOURCE_ADDRESS)?;
        source_device_address = Some(address);
        offset = end;
    }

    let mut source_object_identifier = None;
    if next_is_context(data, offset, 2)? {
        let (object, end) = decode_ctx_object_id(
            data,
            offset,
            2,
            "AuditLogQuery by-source source-object-identifier",
        )?;
        source_object_identifier = Some(object);
        offset = end;
    }

    let mut operations = None;
    if next_is_context(data, offset, 3)? {
        let (flags, end) =
            decode_operation_flags(data, offset, 3, "AuditLogQuery by-source operations")?;
        operations = Some(flags);
        offset = end;
    }

    let (successful_actions_only, offset) = decode_success_filter(
        data,
        offset,
        4,
        "AuditLogQuery by-source successful-actions-only",
    )?;
    expect_end(data, offset, offset, "AuditLogQuery by-source")?;

    Ok(BACnetAuditLogQueryParameters::BySource {
        source_device_identifier,
        source_device_address,
        source_object_identifier,
        operations,
        successful_actions_only,
    })
}

/// What a query's device-address filter and its members are called in
/// errors, each written out whole, so a query that decodes builds no label
/// (#1645).
struct AddressLabels {
    field: &'static str,
    network_number: &'static str,
    mac_address: &'static str,
}

/// The by-target choice's.
const BY_TARGET_ADDRESS: AddressLabels = AddressLabels {
    field: "AuditLogQuery by-target address",
    network_number: "AuditLogQuery by-target address network-number",
    mac_address: "AuditLogQuery by-target address mac-address",
};

/// The by-source choice's.
const BY_SOURCE_ADDRESS: AddressLabels = AddressLabels {
    field: "AuditLogQuery by-source address",
    network_number: "AuditLogQuery by-source address network-number",
    mac_address: "AuditLogQuery by-source address mac-address",
};

fn decode_address(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    labels: &AddressLabels,
) -> Result<(BACnetAddress, usize), Error> {
    let field = labels.field;
    let (opening, content_start) = tags::decode_tag(data, offset)?;
    if !opening.is_opening_tag(expected_tag) {
        return Err(misplaced(
            data,
            &opening,
            Some(expected_tag),
            offset,
            "expected opening tag {}",
        )
        .within(field)
        .with_value(expected_tag.into())
        .into());
    }
    let (content, end) = tags::extract_context_value(data, content_start, expected_tag)?;

    let (network_bytes, inner_offset) =
        decode_app_primitive(content, 0, tags::app_tag::UNSIGNED, labels.network_number)?;
    let network_number = decode_canonical_unsigned(network_bytes, offset, field)?;
    let network_number = u16::try_from(network_number).map_err(|_| {
        DecodeError::new(
            DecodingKind::OutOfRange,
            offset,
            "network-number exceeds u16",
        )
        .within(field)
    })?;

    let mac_offset = inner_offset;
    let (mac_address, inner_offset) = decode_app_primitive(
        content,
        inner_offset,
        tags::app_tag::OCTET_STRING,
        labels.mac_address,
    )?;
    check_decoded_mac_len(mac_address.len(), content_start + mac_offset, field)?;
    expect_end(content, inner_offset, offset + inner_offset, field)?;

    Ok((
        BACnetAddress {
            network_number,
            mac_address: MacAddr::from_slice(mac_address),
        },
        end,
    ))
}

fn decode_operation_flags(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &'static str,
) -> Result<(AuditOperationFlags, usize), Error> {
    let (content, end) = decode_ctx_primitive(data, offset, expected_tag, field)?;
    let (unused_bits, bits) = primitives::decode_bit_string(content)?;
    let flags = AuditOperationFlags::from_bacnet(unused_bits, &bits)?;
    Ok((flags, end))
}

fn decode_success_filter(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &'static str,
) -> Result<(BACnetSuccessFilter, usize), Error> {
    // Strict corrected-baseline enumerated form (Errata 2024-04-29 item 7):
    // only 0 (all), 1 (successes-only), and 2 (failures-only) are accepted,
    // in shortest canonical form. The omitted field still fails via the
    // required-tag check in `decode_ctx_canonical_unsigned`. Note the 0/1
    // octet contents are byte-identical to the old BOOLEAN encoding, so
    // strictness here means the enumerated 0..=2 range plus canonical form —
    // not a distinct tag — while the previously rejected value 2 is now
    // required to decode.
    let (value, end) = decode_ctx_canonical_unsigned::<u64>(data, offset, expected_tag, field)?;
    let filter = match value {
        0 => BACnetSuccessFilter::ALL,
        1 => BACnetSuccessFilter::SUCCESSES_ONLY,
        2 => BACnetSuccessFilter::FAILURES_ONLY,
        // A value the enumeration doesn't define, as GetEnrollmentSummary
        // answers its own filters.
        _ => {
            return Err(Error::reject(RejectReason::UNDEFINED_ENUMERATION));
        }
    };
    Ok((filter, end))
}
