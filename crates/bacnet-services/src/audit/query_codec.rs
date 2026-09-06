use bacnet_encoding::{primitives, tags};
use bacnet_types::bitstring::AuditOperationFlags;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use bytes::BytesMut;

use crate::common::{decode_context, decode_context_bool};

use super::{decode_canonical_unsigned, AuditLogQueryRequest, BACnetAuditLogQueryParameters};

fn validate(request: &AuditLogQueryRequest) -> Result<(), Error> {
    if let BACnetAuditLogQueryParameters::ByTarget {
        target_priority: Some(priority),
        ..
    } = &request.query_parameters
    {
        if !(1..=16).contains(priority) {
            return Err(Error::Encoding(
                "AuditLogQuery target-priority must be in 1..=16".into(),
            ));
        }
    }
    Ok(())
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
            primitives::encode_ctx_boolean(&mut encoded, 7, *successful_actions_only);
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
            primitives::encode_ctx_boolean(&mut encoded, 4, *successful_actions_only);
            tags::encode_closing_tag(&mut encoded, 1);
        }
    }
    tags::encode_closing_tag(&mut encoded, 1);

    if let Some(sequence) = request.start_at_sequence_number {
        primitives::encode_ctx_unsigned(&mut encoded, 2, u64::from(sequence));
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
    let (audit_log, mut offset) = decode_object_id(data, 0, 0, "AuditLogQuery audit-log")?;

    let (wrapper_tag, wrapper_start) = tags::decode_tag(data, offset)?;
    if !wrapper_tag.is_opening_tag(1) {
        return Err(Error::decoding(
            offset,
            "AuditLogQuery query-parameters expected opening tag 1",
        ));
    }
    let (wrapper, wrapper_end) = tags::extract_context_value(data, wrapper_start, 1)?;
    let query_parameters = decode_query_parameters(wrapper)?;
    offset = wrapper_end;

    let mut start_at_sequence_number = None;
    if next_is_context(data, offset, 2)? {
        let (sequence, end) =
            decode_context_u32(data, offset, 2, "AuditLogQuery start-at-sequence-number")?;
        start_at_sequence_number = Some(sequence);
        offset = end;
    }

    let (requested_count, end) =
        decode_context_u16(data, offset, 3, "AuditLogQuery requested-count")?;
    offset = end;
    if offset != data.len() {
        return Err(Error::decoding(offset, "AuditLogQuery has trailing data"));
    }

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
        return Err(Error::decoding(
            0,
            "AuditLogQuery query-parameters expected by-target [0] or by-source [1]",
        ));
    };
    if choice_end != data.len() {
        return Err(Error::decoding(
            choice_end,
            "AuditLogQuery query-parameters has trailing data",
        ));
    }
    Ok(parameters)
}

fn decode_by_target(data: &[u8]) -> Result<BACnetAuditLogQueryParameters, Error> {
    let (target_device_identifier, mut offset) = decode_object_id(
        data,
        0,
        0,
        "AuditLogQuery by-target target-device-identifier",
    )?;

    let mut target_device_address = None;
    if next_is_opening(data, offset, 1)? {
        let (address, end) = decode_address(data, offset, 1, "AuditLogQuery by-target address")?;
        target_device_address = Some(address);
        offset = end;
    }

    let mut target_object_identifier = None;
    if next_is_context(data, offset, 2)? {
        let (object, end) = decode_object_id(
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
        let (property, end) = decode_context_u32(
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
        let (index, end) = decode_context_u64(
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
        let (priority, end) =
            decode_context_u8(data, offset, 5, "AuditLogQuery by-target target-priority")?;
        if !(1..=16).contains(&priority) {
            return Err(Error::decoding(
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

    let (successful_actions_only, offset) = decode_context_bool(
        data,
        offset,
        7,
        "AuditLogQuery by-target successful-actions-only",
    )?;
    if offset != data.len() {
        return Err(Error::decoding(
            offset,
            "AuditLogQuery by-target has trailing data",
        ));
    }

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
    let (source_device_identifier, mut offset) = decode_object_id(
        data,
        0,
        0,
        "AuditLogQuery by-source source-device-identifier",
    )?;

    let mut source_device_address = None;
    if next_is_opening(data, offset, 1)? {
        let (address, end) = decode_address(data, offset, 1, "AuditLogQuery by-source address")?;
        source_device_address = Some(address);
        offset = end;
    }

    let mut source_object_identifier = None;
    if next_is_context(data, offset, 2)? {
        let (object, end) = decode_object_id(
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

    let (successful_actions_only, offset) = decode_context_bool(
        data,
        offset,
        4,
        "AuditLogQuery by-source successful-actions-only",
    )?;
    if offset != data.len() {
        return Err(Error::decoding(
            offset,
            "AuditLogQuery by-source has trailing data",
        ));
    }

    Ok(BACnetAuditLogQueryParameters::BySource {
        source_device_identifier,
        source_device_address,
        source_object_identifier,
        operations,
        successful_actions_only,
    })
}

fn decode_address(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(BACnetAddress, usize), Error> {
    let (opening, content_start) = tags::decode_tag(data, offset)?;
    if !opening.is_opening_tag(expected_tag) {
        return Err(Error::decoding(
            offset,
            format!("{field} expected opening tag {expected_tag}"),
        ));
    }
    let (content, end) = tags::extract_context_value(data, content_start, expected_tag)?;

    let (network_bytes, inner_offset) = decode_application(
        content,
        0,
        tags::app_tag::UNSIGNED,
        &format!("{field} network-number"),
    )?;
    let network_number = decode_canonical_unsigned(network_bytes, offset, field)?;
    let network_number = u16::try_from(network_number)
        .map_err(|_| Error::decoding(offset, format!("{field} network-number exceeds u16")))?;

    let (mac_address, inner_offset) = decode_application(
        content,
        inner_offset,
        tags::app_tag::OCTET_STRING,
        &format!("{field} mac-address"),
    )?;
    if inner_offset != content.len() {
        return Err(Error::decoding(
            offset + inner_offset,
            format!("{field} has trailing data"),
        ));
    }

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
    field: &str,
) -> Result<(AuditOperationFlags, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    let (unused_bits, bits) = primitives::decode_bit_string(content)?;
    let flags = AuditOperationFlags::from_bacnet(unused_bits, &bits)?;
    Ok((flags, end))
}

fn decode_object_id(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(ObjectIdentifier, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    Ok((ObjectIdentifier::decode(content)?, end))
}

fn decode_context_u8(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(u8, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    let value = decode_canonical_unsigned(content, offset, field)?;
    let value =
        u8::try_from(value).map_err(|_| Error::decoding(offset, format!("{field} exceeds u8")))?;
    Ok((value, end))
}

fn decode_context_u16(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(u16, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    let value = decode_canonical_unsigned(content, offset, field)?;
    let value = u16::try_from(value)
        .map_err(|_| Error::decoding(offset, format!("{field} exceeds u16")))?;
    Ok((value, end))
}

fn decode_context_u32(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(u32, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    let value = decode_canonical_unsigned(content, offset, field)?;
    let value = u32::try_from(value)
        .map_err(|_| Error::decoding(offset, format!("{field} exceeds u32")))?;
    Ok((value, end))
}

fn decode_context_u64(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(u64, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    Ok((decode_canonical_unsigned(content, offset, field)?, end))
}

fn decode_application<'a>(
    data: &'a [u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(&'a [u8], usize), Error> {
    let (tag, content_start) = tags::decode_tag(data, offset)?;
    if tag.class != tags::TagClass::Application
        || tag.number != expected_tag
        || tag.is_opening
        || tag.is_closing
    {
        return Err(Error::decoding(
            offset,
            format!("{field} expected application tag {expected_tag}"),
        ));
    }
    let end = content_start
        .checked_add(tag.length as usize)
        .ok_or_else(|| Error::decoding(content_start, format!("{field} length overflow")))?;
    if end > data.len() {
        return Err(Error::decoding(content_start, format!("{field} truncated")));
    }
    Ok((&data[content_start..end], end))
}

fn next_is_context(data: &[u8], offset: usize, number: u8) -> Result<bool, Error> {
    if offset == data.len() {
        return Ok(false);
    }
    Ok(tags::decode_tag(data, offset)?.0.is_context(number))
}

fn next_is_opening(data: &[u8], offset: usize, number: u8) -> Result<bool, Error> {
    if offset == data.len() {
        return Ok(false);
    }
    Ok(tags::decode_tag(data, offset)?.0.is_opening_tag(number))
}
