#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use super::recipient::{check_encoded_recipient, write_recipient};
use super::tagged::{
    decode_app_canonical_enumerated, decode_ctx_canonical_unsigned, decode_ctx_character_string,
    decode_ctx_constructed, decode_ctx_object_id, decode_optional_ctx, expect_end, next_is_opening,
};
use super::{decode_recipient, validate_tlv_sequence};
use crate::{primitives, tags};
use bacnet_types::constructed::{AuditPropertyReference, BACnetAuditNotification};
use bacnet_types::enums::{AuditOperation, ErrorClass, ErrorCode};
use bacnet_types::error::{DecodeError, DecodingKind, Error, RangeError};
use bytes::BytesMut;

/// Encode one bare `BACnetAuditNotification` field sequence.
pub fn encode_audit_notification(
    notification: &BACnetAuditNotification,
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let operation = notification.operation.to_raw();
    if !valid_operation(operation) {
        return Err(RangeError::new("AuditNotification operation {} is reserved")
            .with_value(operation.into())
            .into());
    }
    if let Some(priority) = notification
        .target_priority
        .filter(|priority| !(1..=16).contains(priority))
    {
        return Err(
            RangeError::new("AuditNotification target priority {} is outside 1..=16")
                .with_value(priority.into())
                .into(),
        );
    }
    validate_raw_value(
        notification.target_value.as_deref(),
        "AuditNotification target-value",
    )?;
    validate_raw_value(
        notification.current_value.as_deref(),
        "AuditNotification current-value",
    )?;
    check_encoded_recipient(&notification.source_device)?;
    check_encoded_recipient(&notification.target_device)?;

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
    write_recipient(buf, recipient);
    tags::encode_closing_tag(buf, field_tag);
}

fn encode_raw_value(buf: &mut BytesMut, field_tag: u8, value: &[u8]) {
    tags::encode_opening_tag(buf, field_tag);
    buf.extend_from_slice(value);
    tags::encode_closing_tag(buf, field_tag);
}

/// Check a raw value before encoding it. `what` names it in the error, written
/// out whole, so a value that passes builds no label (#1645).
fn validate_raw_value(value: Option<&[u8]>, what: &'static str) -> Result<(), Error> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_tlv_sequence(value, what).map_err(Error::into_encode_refusal)
}

/// Decode one bare `BACnetAuditNotification` starting at `offset`.
pub fn decode_audit_notification_at(
    data: &[u8],
    mut offset: usize,
) -> Result<(BACnetAuditNotification, usize), Error> {
    let source_timestamp = if next_is_opening(data, offset, 0)? {
        let (timestamp, next) =
            decode_canonical_timestamp(data, offset, 0, "AuditNotification source-timestamp")?;
        offset = next;
        Some(timestamp)
    } else {
        None
    };
    let target_timestamp = if next_is_opening(data, offset, 1)? {
        let (timestamp, next) =
            decode_canonical_timestamp(data, offset, 1, "AuditNotification target-timestamp")?;
        offset = next;
        Some(timestamp)
    } else {
        None
    };

    let (source_device, next) =
        decode_wrapped_recipient(data, offset, 2, "AuditNotification source-device")?;
    offset = next;

    let (source_object, next) = decode_optional_ctx(
        data,
        offset,
        3,
        "AuditNotification source-object",
        decode_ctx_object_id,
    )?;
    offset = next;

    let operation_offset = offset;
    let (operation_raw, next) =
        decode_ctx_canonical_unsigned::<u32>(data, offset, 4, "AuditNotification operation")?;
    if !valid_operation(operation_raw) {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            operation_offset,
            "AuditNotification operation {} is reserved",
        )
        .with_value(operation_raw.into())
        .into());
    }
    let operation = AuditOperation::from_raw(operation_raw);
    offset = next;

    let (source_comment, next) = decode_optional_ctx(
        data,
        offset,
        5,
        "AuditNotification source-comment",
        decode_ctx_character_string,
    )?;
    let (target_comment, next) = decode_optional_ctx(
        data,
        next,
        6,
        "AuditNotification target-comment",
        decode_ctx_character_string,
    )?;
    let (invoke_id, next) = decode_optional_ctx(
        data,
        next,
        7,
        "AuditNotification invoke-id",
        decode_ctx_canonical_unsigned::<u8>,
    )?;
    let (source_user_id, next) = decode_optional_ctx(
        data,
        next,
        8,
        "AuditNotification source-user-id",
        decode_ctx_canonical_unsigned::<u16>,
    )?;
    let (source_user_role, next) = decode_optional_ctx(
        data,
        next,
        9,
        "AuditNotification source-user-role",
        decode_ctx_canonical_unsigned::<u8>,
    )?;
    offset = next;

    let (target_device, next) =
        decode_wrapped_recipient(data, offset, 10, "AuditNotification target-device")?;
    offset = next;

    let (target_object, next) = decode_optional_ctx(
        data,
        offset,
        11,
        "AuditNotification target-object",
        decode_ctx_object_id,
    )?;
    offset = next;
    let target_property = if next_is_opening(data, offset, 12)? {
        const WHAT: &str = "AuditNotification target-property";
        let (body, next) = decode_ctx_constructed(data, offset, 12, WHAT)?;
        let (property, property_end) = decode_property_reference(body)?;
        expect_end(body, property_end, offset, WHAT)?;
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
    let priority_offset = offset;
    let (target_priority, next) = decode_optional_ctx(
        data,
        offset,
        13,
        "AuditNotification target-priority",
        decode_ctx_canonical_unsigned::<u8>,
    )?;
    if let Some(priority) = target_priority.filter(|priority| !(1..=16).contains(priority)) {
        return Err(DecodeError::new(
            DecodingKind::OutOfRange,
            priority_offset,
            "AuditNotification target-priority {} is outside 1..=16",
        )
        .with_value(priority.into())
        .into());
    }
    offset = next;
    let target_value = if next_is_opening(data, offset, 14)? {
        let (value, next) = decode_raw_value(data, offset, 14, "AuditNotification target-value")?;
        offset = next;
        Some(value)
    } else {
        None
    };
    let current_value = if next_is_opening(data, offset, 15)? {
        let (value, next) = decode_raw_value(data, offset, 15, "AuditNotification current-value")?;
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
    let (property, offset) = decode_ctx_canonical_unsigned::<u32>(
        data,
        0,
        0,
        "AuditNotification target-property identifier",
    )?;
    let (property_array_index, offset) = decode_optional_ctx(
        data,
        offset,
        1,
        "AuditNotification target-property array-index",
        decode_ctx_canonical_unsigned::<u64>,
    )?;
    Ok((
        AuditPropertyReference {
            property_identifier: bacnet_types::enums::PropertyIdentifier::from_raw(property),
            property_array_index,
        },
        offset,
    ))
}

fn decode_wrapped_recipient(
    data: &[u8],
    offset: usize,
    tag_number: u8,
    what: &'static str,
) -> Result<(bacnet_types::constructed::BACnetRecipient, usize), Error> {
    let (body, next) = decode_ctx_constructed(data, offset, tag_number, what)?;
    let (recipient, recipient_end) = decode_recipient(body, 0)?;
    expect_end(body, recipient_end, offset, what)?;
    let mut canonical = BytesMut::new();
    write_recipient(&mut canonical, &recipient);
    if canonical.as_ref() != body {
        return Err(not_canonical(offset, what));
    }
    Ok((recipient, next))
}

fn decode_canonical_timestamp(
    data: &[u8],
    offset: usize,
    tag_number: u8,
    what: &'static str,
) -> Result<(bacnet_types::primitives::BACnetTimeStamp, usize), Error> {
    let (timestamp, next) = primitives::decode_timestamp(data, offset, tag_number)?;
    let mut canonical = BytesMut::new();
    primitives::encode_timestamp(&mut canonical, tag_number, &timestamp)?;
    if canonical.as_ref() != &data[offset..next] {
        return Err(not_canonical(offset, what));
    }
    Ok((timestamp, next))
}

/// The refusal of a member `what` at `offset` whose encoding isn't the
/// canonical one.
#[cold]
fn not_canonical(offset: usize, what: &'static str) -> Error {
    DecodeError::new(DecodingKind::InvalidEncoding, offset, "not canonically encoded")
        .within(what)
        .into()
}

fn decode_raw_value(
    data: &[u8],
    offset: usize,
    tag_number: u8,
    what: &'static str,
) -> Result<(Vec<u8>, usize), Error> {
    let (value, next) = decode_ctx_constructed(data, offset, tag_number, what)?;
    validate_tlv_sequence(value, what)?;
    Ok((value.to_vec(), next))
}

fn decode_error(data: &[u8], offset: usize) -> Result<((ErrorClass, ErrorCode), usize), Error> {
    const WHAT: &str = "AuditNotification result";
    let (body, next) = decode_ctx_constructed(data, offset, 16, WHAT)?;
    let (class, body_offset) =
        decode_app_canonical_enumerated(body, 0, "AuditNotification result error-class")?;
    let (code, body_end) =
        decode_app_canonical_enumerated(body, body_offset, "AuditNotification result error-code")?;
    expect_end(body, body_end, offset, WHAT)?;
    Ok((
        (ErrorClass::from_raw(class), ErrorCode::from_raw(code)),
        next,
    ))
}
