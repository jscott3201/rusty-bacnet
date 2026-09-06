use super::{decode_recipient, encode_recipient, validate_tlv_sequence};
use crate::{primitives, tags};
use bacnet_types::constructed::{AuditPropertyReference, BACnetAuditNotification};
use bacnet_types::enums::{AuditOperation, ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

/// Encode one bare `BACnetAuditNotification` field sequence.
pub fn encode_audit_notification(
    notification: &BACnetAuditNotification,
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let operation = notification.operation.to_raw();
    if !valid_operation(operation) {
        return Err(Error::OutOfRange(format!(
            "AuditNotification operation {operation} is reserved"
        )));
    }
    if notification
        .target_priority
        .is_some_and(|priority| !(1..=16).contains(&priority))
    {
        return Err(Error::OutOfRange(format!(
            "AuditNotification target priority {} is outside 1..=16",
            notification.target_priority.unwrap()
        )));
    }
    validate_raw_value(notification.target_value.as_deref(), "target-value")?;
    validate_raw_value(notification.current_value.as_deref(), "current-value")?;

    if let Some(timestamp) = &notification.source_timestamp {
        primitives::encode_timestamp(buf, 0, timestamp)?;
    }
    if let Some(timestamp) = &notification.target_timestamp {
        primitives::encode_timestamp(buf, 1, timestamp)?;
    }

    encode_wrapped_recipient(buf, 2, &notification.source_device);
    if let Some(object) = &notification.source_object {
        primitives::encode_ctx_object_id(buf, 3, object);
    }
    primitives::encode_ctx_enumerated(buf, 4, operation);
    if let Some(comment) = &notification.source_comment {
        primitives::encode_ctx_character_string(buf, 5, comment)?;
    }
    if let Some(comment) = &notification.target_comment {
        primitives::encode_ctx_character_string(buf, 6, comment)?;
    }
    if let Some(invoke_id) = notification.invoke_id {
        primitives::encode_ctx_unsigned(buf, 7, u64::from(invoke_id));
    }
    if let Some(user_id) = notification.source_user_id {
        primitives::encode_ctx_unsigned(buf, 8, u64::from(user_id));
    }
    if let Some(user_role) = notification.source_user_role {
        primitives::encode_ctx_unsigned(buf, 9, u64::from(user_role));
    }

    encode_wrapped_recipient(buf, 10, &notification.target_device);
    if let Some(object) = &notification.target_object {
        primitives::encode_ctx_object_id(buf, 11, object);
    }
    if let Some(property) = &notification.target_property {
        tags::encode_opening_tag(buf, 12);
        encode_property_reference(buf, property);
        tags::encode_closing_tag(buf, 12);
    }
    if let Some(priority) = notification.target_priority {
        primitives::encode_ctx_unsigned(buf, 13, u64::from(priority));
    }
    if let Some(value) = &notification.target_value {
        encode_raw_value(buf, 14, value);
    }
    if let Some(value) = &notification.current_value {
        encode_raw_value(buf, 15, value);
    }
    if let Some((class, code)) = notification.result {
        tags::encode_opening_tag(buf, 16);
        primitives::encode_app_enumerated(buf, u32::from(class.to_raw()));
        primitives::encode_app_enumerated(buf, u32::from(code.to_raw()));
        tags::encode_closing_tag(buf, 16);
    }
    Ok(())
}

fn encode_wrapped_recipient(
    buf: &mut BytesMut,
    field_tag: u8,
    recipient: &bacnet_types::constructed::BACnetRecipient,
) {
    tags::encode_opening_tag(buf, field_tag);
    encode_recipient(buf, recipient);
    tags::encode_closing_tag(buf, field_tag);
}

fn encode_raw_value(buf: &mut BytesMut, field_tag: u8, value: &[u8]) {
    tags::encode_opening_tag(buf, field_tag);
    buf.extend_from_slice(value);
    tags::encode_closing_tag(buf, field_tag);
}

fn validate_raw_value(value: Option<&[u8]>, field: &str) -> Result<(), Error> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.is_empty() {
        return Err(Error::Encoding(format!(
            "AuditNotification {field} must contain a BACnet value"
        )));
    }
    validate_tlv_sequence(value, &format!("AuditNotification {field}"))
        .map_err(|error| Error::Encoding(error.to_string()))
}

/// Decode one bare `BACnetAuditNotification` starting at `offset`.
pub fn decode_audit_notification_at(
    data: &[u8],
    mut offset: usize,
) -> Result<(BACnetAuditNotification, usize), Error> {
    let source_timestamp = if next_is_opening(data, offset, 0)? {
        let (timestamp, next) = decode_canonical_timestamp(data, offset, 0, "source-timestamp")?;
        offset = next;
        Some(timestamp)
    } else {
        None
    };
    let target_timestamp = if next_is_opening(data, offset, 1)? {
        let (timestamp, next) = decode_canonical_timestamp(data, offset, 1, "target-timestamp")?;
        offset = next;
        Some(timestamp)
    } else {
        None
    };

    let (source_device, next) = decode_wrapped_recipient(data, offset, 2, "source-device")?;
    offset = next;

    let source_object = if next_is_context(data, offset, 3)? {
        let (object, next) = decode_object(data, offset, 3, "source-object")?;
        offset = next;
        Some(object)
    } else {
        None
    };

    let operation_offset = offset;
    let (operation_raw, next) = decode_context_u32(data, offset, 4, "operation")?;
    if !valid_operation(operation_raw) {
        return Err(Error::decoding(
            operation_offset,
            format!("AuditNotification operation {operation_raw} is reserved"),
        ));
    }
    let operation = AuditOperation::from_raw(operation_raw);
    offset = next;

    let source_comment = if next_is_context(data, offset, 5)? {
        let (comment, next) = decode_string(data, offset, 5, "source-comment")?;
        offset = next;
        Some(comment)
    } else {
        None
    };
    let target_comment = if next_is_context(data, offset, 6)? {
        let (comment, next) = decode_string(data, offset, 6, "target-comment")?;
        offset = next;
        Some(comment)
    } else {
        None
    };
    let invoke_id = if next_is_context(data, offset, 7)? {
        let (value, next) = decode_context_u8(data, offset, 7, "invoke-id")?;
        offset = next;
        Some(value)
    } else {
        None
    };
    let source_user_id = if next_is_context(data, offset, 8)? {
        let (value, next) = decode_context_u16(data, offset, 8, "source-user-id")?;
        offset = next;
        Some(value)
    } else {
        None
    };
    let source_user_role = if next_is_context(data, offset, 9)? {
        let (value, next) = decode_context_u8(data, offset, 9, "source-user-role")?;
        offset = next;
        Some(value)
    } else {
        None
    };

    let (target_device, next) = decode_wrapped_recipient(data, offset, 10, "target-device")?;
    offset = next;

    let target_object = if next_is_context(data, offset, 11)? {
        let (object, next) = decode_object(data, offset, 11, "target-object")?;
        offset = next;
        Some(object)
    } else {
        None
    };
    let target_property = if next_is_opening(data, offset, 12)? {
        let (body, next) = decode_constructed_body(data, offset, 12, "target-property")?;
        let (property, property_end) = decode_property_reference(body)?;
        if property_end != body.len() {
            return Err(Error::decoding(
                offset,
                "AuditNotification target-property has trailing fields",
            ));
        }
        let mut canonical = BytesMut::new();
        encode_property_reference(&mut canonical, &property);
        if canonical.as_ref() != body {
            return Err(Error::decoding(
                offset,
                "AuditNotification target-property is not canonically encoded",
            ));
        }
        offset = next;
        Some(property)
    } else {
        None
    };
    let target_priority = if next_is_context(data, offset, 13)? {
        let priority_offset = offset;
        let (priority, next) = decode_context_u8(data, offset, 13, "target-priority")?;
        if !(1..=16).contains(&priority) {
            return Err(Error::decoding(
                priority_offset,
                format!("AuditNotification target-priority {priority} is outside 1..=16"),
            ));
        }
        offset = next;
        Some(priority)
    } else {
        None
    };
    let target_value = if next_is_opening(data, offset, 14)? {
        let (value, next) = decode_raw_value(data, offset, 14, "target-value")?;
        offset = next;
        Some(value)
    } else {
        None
    };
    let current_value = if next_is_opening(data, offset, 15)? {
        let (value, next) = decode_raw_value(data, offset, 15, "current-value")?;
        offset = next;
        Some(value)
    } else {
        None
    };
    let result = if next_is_opening(data, offset, 16)? {
        let (result, next) = decode_error(data, offset)?;
        offset = next;
        Some(result)
    } else {
        None
    };

    Ok((
        BACnetAuditNotification {
            source_timestamp,
            target_timestamp,
            source_device,
            source_object,
            operation,
            source_comment,
            target_comment,
            invoke_id,
            source_user_id,
            source_user_role,
            target_device,
            target_object,
            target_property,
            target_priority,
            target_value,
            current_value,
            result,
        },
        offset,
    ))
}

fn valid_operation(value: u32) -> bool {
    (0..=15).contains(&value) || (32..=63).contains(&value)
}

fn encode_property_reference(buf: &mut BytesMut, property: &AuditPropertyReference) {
    primitives::encode_ctx_enumerated(buf, 0, property.property_identifier.to_raw());
    if let Some(index) = property.property_array_index {
        primitives::encode_ctx_unsigned(buf, 1, index);
    }
}

fn decode_property_reference(data: &[u8]) -> Result<(AuditPropertyReference, usize), Error> {
    let (property, mut offset) = decode_context_u32(data, 0, 0, "target-property identifier")?;
    let property_array_index = if next_is_context(data, offset, 1)? {
        let (index, next) = decode_context_u64(data, offset, 1, "target-property array-index")?;
        offset = next;
        Some(index)
    } else {
        None
    };
    Ok((
        AuditPropertyReference {
            property_identifier: bacnet_types::enums::PropertyIdentifier::from_raw(property),
            property_array_index,
        },
        offset,
    ))
}

fn next_is_context(data: &[u8], offset: usize, number: u8) -> Result<bool, Error> {
    if offset == data.len() {
        return Ok(false);
    }
    let (tag, _) = tags::decode_tag(data, offset)?;
    Ok(tag.is_context(number))
}

fn next_is_opening(data: &[u8], offset: usize, number: u8) -> Result<bool, Error> {
    if offset == data.len() {
        return Ok(false);
    }
    let (tag, _) = tags::decode_tag(data, offset)?;
    Ok(tag.is_opening_tag(number))
}

fn decode_context_u32(
    data: &[u8],
    offset: usize,
    tag: u8,
    field: &str,
) -> Result<(u32, usize), Error> {
    let (contents, next) =
        decode_context(data, offset, tag, &format!("AuditNotification {field}"))?;
    let raw = decode_canonical_unsigned(contents, offset, field)?;
    let value = u32::try_from(raw)
        .map_err(|_| Error::decoding(offset, format!("AuditNotification {field} exceeds u32")))?;
    Ok((value, next))
}

fn decode_context_u64(
    data: &[u8],
    offset: usize,
    tag: u8,
    field: &str,
) -> Result<(u64, usize), Error> {
    let (contents, next) =
        decode_context(data, offset, tag, &format!("AuditNotification {field}"))?;
    Ok((decode_canonical_unsigned(contents, offset, field)?, next))
}

fn decode_context_u16(
    data: &[u8],
    offset: usize,
    tag: u8,
    field: &str,
) -> Result<(u16, usize), Error> {
    let (contents, next) =
        decode_context(data, offset, tag, &format!("AuditNotification {field}"))?;
    let raw = decode_canonical_unsigned(contents, offset, field)?;
    let value = u16::try_from(raw)
        .map_err(|_| Error::decoding(offset, format!("AuditNotification {field} exceeds u16")))?;
    Ok((value, next))
}

fn decode_context_u8(
    data: &[u8],
    offset: usize,
    tag: u8,
    field: &str,
) -> Result<(u8, usize), Error> {
    let (contents, next) =
        decode_context(data, offset, tag, &format!("AuditNotification {field}"))?;
    let raw = decode_canonical_unsigned(contents, offset, field)?;
    let value = u8::try_from(raw)
        .map_err(|_| Error::decoding(offset, format!("AuditNotification {field} exceeds u8")))?;
    Ok((value, next))
}

fn decode_object(
    data: &[u8],
    offset: usize,
    tag: u8,
    field: &str,
) -> Result<(ObjectIdentifier, usize), Error> {
    let (contents, next) =
        decode_context(data, offset, tag, &format!("AuditNotification {field}"))?;
    Ok((ObjectIdentifier::decode(contents)?, next))
}

fn decode_string(
    data: &[u8],
    offset: usize,
    tag: u8,
    field: &str,
) -> Result<(String, usize), Error> {
    let (contents, next) =
        decode_context(data, offset, tag, &format!("AuditNotification {field}"))?;
    Ok((primitives::decode_character_string(contents)?, next))
}

fn decode_constructed_body<'a>(
    data: &'a [u8],
    offset: usize,
    tag_number: u8,
    field: &str,
) -> Result<(&'a [u8], usize), Error> {
    let (opening, body_start) = tags::decode_tag(data, offset)?;
    if !opening.is_opening_tag(tag_number) {
        return Err(Error::decoding(
            offset,
            format!("AuditNotification {field} expected opening tag [{tag_number}]"),
        ));
    }
    tags::extract_context_value(data, body_start, tag_number)
}

fn decode_wrapped_recipient(
    data: &[u8],
    offset: usize,
    tag_number: u8,
    field: &str,
) -> Result<(bacnet_types::constructed::BACnetRecipient, usize), Error> {
    let (body, next) = decode_constructed_body(data, offset, tag_number, field)?;
    let (recipient, recipient_end) = decode_recipient(body, 0)?;
    if recipient_end != body.len() {
        return Err(Error::decoding(
            offset,
            format!("AuditNotification {field} has trailing fields"),
        ));
    }
    let mut canonical = BytesMut::new();
    encode_recipient(&mut canonical, &recipient);
    if canonical.as_ref() != body {
        return Err(Error::decoding(
            offset,
            format!("AuditNotification {field} is not canonically encoded"),
        ));
    }
    Ok((recipient, next))
}

fn decode_canonical_timestamp(
    data: &[u8],
    offset: usize,
    tag_number: u8,
    field: &str,
) -> Result<(bacnet_types::primitives::BACnetTimeStamp, usize), Error> {
    let (timestamp, next) = primitives::decode_timestamp(data, offset, tag_number)?;
    let mut canonical = BytesMut::new();
    primitives::encode_timestamp(&mut canonical, tag_number, &timestamp)?;
    if canonical.as_ref() != &data[offset..next] {
        return Err(Error::decoding(
            offset,
            format!("AuditNotification {field} is not canonically encoded"),
        ));
    }
    Ok((timestamp, next))
}

fn decode_raw_value(
    data: &[u8],
    offset: usize,
    tag_number: u8,
    field: &str,
) -> Result<(Vec<u8>, usize), Error> {
    let (value, next) = decode_constructed_body(data, offset, tag_number, field)?;
    if value.is_empty() {
        return Err(Error::decoding(
            offset,
            format!("AuditNotification {field} must not be empty"),
        ));
    }
    validate_tlv_sequence(value, &format!("AuditNotification {field}"))?;
    Ok((value.to_vec(), next))
}

fn decode_error(data: &[u8], offset: usize) -> Result<((ErrorClass, ErrorCode), usize), Error> {
    let (body, next) = decode_constructed_body(data, offset, 16, "result")?;
    let (class, body_offset) = decode_app_enumerated_u16(body, 0, "result error-class")?;
    let (code, body_end) = decode_app_enumerated_u16(body, body_offset, "result error-code")?;
    if body_end != body.len() {
        return Err(Error::decoding(
            offset,
            "AuditNotification result has trailing fields",
        ));
    }
    Ok((
        (ErrorClass::from_raw(class), ErrorCode::from_raw(code)),
        next,
    ))
}

fn decode_app_enumerated_u16(
    data: &[u8],
    offset: usize,
    field: &str,
) -> Result<(u16, usize), Error> {
    let (tag, contents_start) = tags::decode_tag(data, offset)?;
    if tag.class != tags::TagClass::Application || tag.number != tags::app_tag::ENUMERATED {
        return Err(Error::decoding(
            offset,
            format!("AuditNotification {field} expected application Enumerated"),
        ));
    }
    let end = contents_start
        .checked_add(tag.length as usize)
        .ok_or_else(|| {
            Error::decoding(contents_start, "AuditNotification result length overflow")
        })?;
    if end > data.len() {
        return Err(Error::decoding(
            contents_start,
            format!("AuditNotification {field} is truncated"),
        ));
    }
    let raw = decode_canonical_unsigned(&data[contents_start..end], offset, field)?;
    let value = u16::try_from(raw)
        .map_err(|_| Error::decoding(offset, format!("AuditNotification {field} exceeds u16")))?;
    Ok((value, end))
}

fn decode_context<'a>(
    data: &'a [u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(&'a [u8], usize), Error> {
    let (tag, contents_start) = tags::decode_tag(data, offset)?;
    if !tag.is_context(expected_tag) {
        return Err(Error::decoding(
            offset,
            format!("{field} expected context tag [{expected_tag}]"),
        ));
    }
    let end = contents_start
        .checked_add(tag.length as usize)
        .ok_or_else(|| Error::decoding(contents_start, format!("{field} length overflow")))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    Ok((&data[contents_start..end], end))
}

fn decode_canonical_unsigned(data: &[u8], offset: usize, field: &str) -> Result<u64, Error> {
    if data.is_empty() {
        return Err(Error::decoding(
            offset,
            format!("{field} must contain at least one octet"),
        ));
    }
    if data.len() > 1 && data.first() == Some(&0) {
        return Err(Error::decoding(
            offset,
            format!("{field} must use the shortest Unsigned/Enumerated encoding"),
        ));
    }
    primitives::decode_unsigned(data)
}
