use super::decode_helpers::{decode_context_status_flags, finish_variant};
use super::*;
use crate::common::{decode_context, decode_context_u32};
use bacnet_encoding::constructed::validate_tlv_sequence;

pub(super) fn validate_authentication_factor(data: &[u8]) -> Result<(), Error> {
    let (_, pos) = decode_context_u32(data, 0, 0, "AuthenticationFactor format-type")?;
    let (format_class, pos) = decode_context(data, pos, 1, "AuthenticationFactor format-class")?;
    primitives::decode_unsigned(format_class)?;
    let (_, pos) = decode_context(data, pos, 2, "AuthenticationFactor value")?;
    if pos != data.len() {
        return Err(Error::decoding(
            pos,
            "AuthenticationFactor has unexpected fields",
        ));
    }
    Ok(())
}

pub(super) fn decode_complex_event_type(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    let mut property_values = Vec::new();
    let mut pos = inner_start;
    while pos < variant_body_end {
        if property_values.len() >= MAX_DECODED_ITEMS {
            return Err(Error::decoding(
                pos,
                format!("ComplexEventType exceeds {MAX_DECODED_ITEMS} property values"),
            ));
        }
        let (property_value, next) = BACnetPropertyValue::decode_in_list(data, pos, 6)?;
        if next <= pos || next > variant_body_end {
            return Err(Error::decoding(
                pos,
                "ComplexEventType property value made invalid progress",
            ));
        }
        validate_tlv_sequence(&property_value.value, "ComplexEventType property value")?;
        property_values.push(property_value);
        pos = next;
    }
    finish_variant(
        NotificationParameters::ComplexEventType { property_values },
        pos,
        variant_body_end,
        6,
    )
}

fn decode_object_identifier(
    data: &[u8],
    offset: usize,
    context_tag: u8,
    field: &str,
) -> Result<(ObjectIdentifier, usize), Error> {
    let (content, next) = decode_context(data, offset, context_tag, field)?;
    Ok((ObjectIdentifier::decode(content)?, next))
}

fn decode_access_credential(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetDeviceObjectReference, usize), Error> {
    let (opening, mut pos) = tags::decode_tag(data, offset)?;
    if !opening.is_opening_tag(4) {
        return Err(Error::decoding(
            offset,
            "AccessEvent expected opening [4] for access-credential",
        ));
    }

    let device_identifier = if tags::decode_tag(data, pos)?.0.is_context(0) {
        let (value, next) =
            decode_object_identifier(data, pos, 0, "AccessEvent credential device-identifier")?;
        pos = next;
        Some(value)
    } else {
        None
    };
    let (object_identifier, next) =
        decode_object_identifier(data, pos, 1, "AccessEvent credential object-identifier")?;
    pos = next;

    let (closing, next) = tags::decode_tag(data, pos)?;
    if !closing.is_closing_tag(4) {
        return Err(Error::decoding(
            pos,
            "AccessEvent credential has duplicate, out-of-order, or trailing fields",
        ));
    }
    Ok((
        BACnetDeviceObjectReference {
            device_identifier,
            object_identifier,
        },
        next,
    ))
}

pub(super) fn decode_access_event(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    let (access_event, pos) = decode_context_u32(data, inner_start, 0, "AccessEvent access-event")?;
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "AccessEvent status-flags")?;
    let (access_event_tag, pos) = decode_context_u32(data, pos, 2, "AccessEvent access-event-tag")?;
    let (timestamp, pos) = primitives::decode_timestamp(data, pos, 3)?;
    let access_event_time = match timestamp {
        BACnetTimeStamp::DateTime { date, time } => (date, time),
        _ => {
            return Err(Error::decoding(
                pos,
                "AccessEvent expected a DateTime timestamp",
            ));
        }
    };
    let (access_credential, pos) = decode_access_credential(data, pos)?;

    let (authentication_factor, pos) = if pos < variant_body_end {
        let (opening, content_start) = tags::decode_tag(data, pos)?;
        if !opening.is_opening_tag(5) {
            return Err(Error::decoding(
                pos,
                "AccessEvent expected opening [5] for authentication-factor",
            ));
        }
        let (value, next) = extract_raw_context(data, content_start, 5)?;
        validate_authentication_factor(&value)?;
        (Some(value), next)
    } else {
        (None, pos)
    };

    finish_variant(
        NotificationParameters::AccessEvent {
            access_event,
            status_flags,
            access_event_tag,
            access_event_time,
            access_credential,
            authentication_factor,
        },
        pos,
        variant_body_end,
        13,
    )
}
