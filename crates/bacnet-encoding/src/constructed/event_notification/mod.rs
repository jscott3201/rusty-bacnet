//! ConfirmedEventNotification / UnconfirmedEventNotification request
//! parameters (Clauses 13.8 and 13.9) and their event values, the
//! NotificationParameters choice of Clause 21.
//!
//! An Event Log record carries the same request parameters as its
//! notification datum (Clause 12.27.13), so the record codec reuses these
//! functions.

use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference, BACnetPropertyStates,
    ChangeOfValueChoice, EventNotificationRequest, NotificationParameters,
};
use bacnet_types::enums::{
    AccessEvent, EventState, EventType, LifeSafetyMode, LifeSafetyOperation, LifeSafetyState,
    NotifyType, Reliability, TimerState, TimerTransition,
};
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, StatusFlags, Time};
use bytes::BytesMut;

use super::tagged::{misplaced, misplaced_tag, unclosed_kind};
use super::{
    decode_bacnet_property_value_in_list, encode_bacnet_property_value, validate_tlv_sequence,
    MAX_FRAMED_ITEMS,
};
use crate::{primitives, tags};
use decode_helpers::{
    decode_context, decode_context_bool, decode_context_enum, decode_context_u32,
};
use property_states::{
    decode_device_obj_prop_ref, decode_property_states, encode_property_states, extract_raw_context,
};

mod decode_helpers;
mod decode_timer;
mod parameters_decode;
mod parameters_encode;
mod property_states;
mod structured;
mod tolerant;

pub use parameters_decode::decode_notification_parameters;
pub use parameters_encode::encode_notification_parameters;
pub use tolerant::decode_event_notification_tolerant;

/// Append the encoded request parameters to `buf`; `buf` is unchanged if validation fails.
pub fn encode_event_notification(
    request: &EventNotificationRequest,
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let mut encoded = BytesMut::new();
    encode_into(request, &mut encoded)?;
    validate_tlv_sequence(&encoded, "EventNotification")
        .map_err(Error::into_encode_refusal)?;
    buf.extend_from_slice(&encoded);
    Ok(())
}

fn encode_into(request: &EventNotificationRequest, buf: &mut BytesMut) -> Result<(), Error> {
    // [0] processIdentifier
    primitives::encode_ctx_unsigned(buf, 0, request.process_identifier as u64);
    // [1] initiatingDeviceIdentifier
    primitives::encode_ctx_object_id(buf, 1, &request.initiating_device_identifier);
    // [2] eventObjectIdentifier
    primitives::encode_ctx_object_id(buf, 2, &request.event_object_identifier);
    // [3] timeStamp
    primitives::encode_timestamp(buf, 3, &request.timestamp)?;
    // [4] notificationClass
    primitives::encode_ctx_unsigned(buf, 4, request.notification_class as u64);
    // [5] priority
    primitives::encode_ctx_unsigned(buf, 5, request.priority as u64);
    // [6] eventType
    primitives::encode_ctx_enumerated(buf, 6, request.event_type.to_raw());
    // [7] messageText (optional)
    if let Some(ref text) = request.message_text {
        primitives::encode_ctx_character_string(buf, 7, text)?;
    }
    // [8] notifyType
    primitives::encode_ctx_enumerated(buf, 8, request.notify_type.to_raw());
    // [9] ackRequired (only for ALARM/EVENT)
    if request.notify_type != NotifyType::ACK_NOTIFICATION {
        primitives::encode_ctx_boolean(buf, 9, request.ack_required);
    }
    // [10] fromState (only for ALARM/EVENT)
    if request.notify_type != NotifyType::ACK_NOTIFICATION {
        primitives::encode_ctx_enumerated(buf, 10, request.from_state.to_raw());
    }
    // [11] toState
    primitives::encode_ctx_enumerated(buf, 11, request.to_state.to_raw());
    // [12] eventValues — optional
    if request.notify_type != NotifyType::ACK_NOTIFICATION {
        if let Some(ref params) = request.event_values {
            tags::encode_opening_tag(buf, 12);
            encode_notification_parameters(params, buf)?;
            tags::encode_closing_tag(buf, 12);
        }
    }
    Ok(())
}

/// Decode request parameters from `data`; errors on missing, malformed or truncated fields.
pub fn decode_event_notification(data: &[u8]) -> Result<EventNotificationRequest, Error> {
    validate_tlv_sequence(data, "EventNotification")?;
    // [0] processIdentifier
    let (process_identifier, mut offset) =
        decode_context_u32(data, 0, 0, "EventNotification processIdentifier")?;

    // [1] initiatingDeviceIdentifier
    let (content, new_offset) = decode_context(
        data,
        offset,
        1,
        "EventNotification initiatingDeviceIdentifier",
    )?;
    let initiating_device_identifier = ObjectIdentifier::decode(content)?;
    offset = new_offset;

    // [2] eventObjectIdentifier
    let (content, new_offset) =
        decode_context(data, offset, 2, "EventNotification eventObjectIdentifier")?;
    let event_object_identifier = ObjectIdentifier::decode(content)?;
    offset = new_offset;

    // [3] timeStamp
    let (timestamp, new_offset) = primitives::decode_timestamp(data, offset, 3)?;
    offset = new_offset;

    // [4] notificationClass
    let (notification_class, new_offset) =
        decode_context_u32(data, offset, 4, "EventNotification notificationClass")?;
    offset = new_offset;

    // [5] priority
    let priority_offset = offset;
    let (content, new_offset) = decode_context(data, offset, 5, "EventNotification priority")?;
    let priority = primitives::decode_unsigned(content)?;
    let priority = u8::try_from(priority).map_err(|_| {
        Error::out_of_range(priority_offset, "EventNotification priority exceeds u8")
    })?;
    offset = new_offset;

    // [6] eventType
    let (event_type, new_offset) = decode_context_enum(
        data,
        offset,
        6,
        "EventNotification eventType",
        EventType::from_raw,
    )?;
    offset = new_offset;

    // [7] messageText (optional)
    let mut message_text = None;
    if offset < data.len() {
        let (peek, _) = tags::decode_tag(data, offset)?;
        if peek.is_context(7) {
            let (content, new_offset) =
                decode_context(data, offset, 7, "EventNotification messageText")?;
            message_text = Some(primitives::decode_character_string(content)?);
            offset = new_offset;
        }
    }

    // [8] notifyType
    let (notify_type, new_offset) = decode_context_enum(
        data,
        offset,
        8,
        "EventNotification notifyType",
        NotifyType::from_raw,
    )?;
    offset = new_offset;

    // [9] ackRequired (optional — present for ALARM/EVENT)
    let mut ack_required = false;
    if offset < data.len() {
        let (peek, _) = tags::decode_tag(data, offset)?;
        if peek.is_context(9) {
            (ack_required, offset) =
                decode_context_bool(data, offset, 9, "EventNotification ackRequired")?;
        }
    }

    // [10] fromState (absent for ACK_NOTIFICATION)
    let mut from_state = EventState::NORMAL;
    if offset < data.len() {
        let (peek, _) = tags::decode_tag(data, offset)?;
        if peek.is_context(10) {
            (from_state, offset) = decode_context_enum(
                data,
                offset,
                10,
                "EventNotification fromState",
                EventState::from_raw,
            )?;
        } else if notify_type != NotifyType::ACK_NOTIFICATION {
            return Err(misplaced_tag(
                data,
                &peek,
                Some(10),
                offset,
                "EventNotification expected fromState",
            ));
        }
    } else if notify_type != NotifyType::ACK_NOTIFICATION {
        return Err(Error::missing(
            offset,
            "EventNotification missing fromState",
        ));
    }

    // [11] toState
    let (to_state, new_offset) = decode_context_enum(
        data,
        offset,
        11,
        "EventNotification toState",
        EventState::from_raw,
    )?;
    offset = new_offset;

    // [12] eventValues — optional
    let mut event_values = None;
    if offset < data.len() {
        let (opening, inner_start) = tags::decode_tag(data, offset)?;
        if !opening.is_opening_tag(12) {
            return Err(misplaced_tag(
                data,
                &opening,
                Some(12),
                offset,
                "EventNotification expected opening tag 12 for eventValues",
            ));
        }
        let closing_offset = data
            .len()
            .checked_sub(1)
            .ok_or_else(|| Error::missing(offset, "EventNotification missing closing tag 12"))?;
        if closing_offset < inner_start {
            // The data ends right after the opening tag.
            return Err(Error::missing(
                inner_start,
                "EventNotification eventValues are never closed",
            ));
        }
        let (closing, next) = tags::decode_tag(data, closing_offset)?;
        if !closing.is_closing_tag(12) || next != data.len() {
            // Where the frame really ends: past the data (no closing tag) or
            // before its end (octets after the request).
            return Err(match tags::extract_context_value(data, inner_start, 12) {
                Err(error) => error,
                Ok((_, end)) if end < data.len() => {
                    Error::trailing(end, "EventNotification has octets after eventValues")
                }
                Ok(_) => Error::invalid_tag(
                    closing_offset,
                    "EventNotification expected closing tag 12 after eventValues",
                ),
            });
        }
        event_values = Some(parameters_decode::decode_bounded(
            data,
            inner_start,
            closing_offset,
        )?);
        offset = next;
    }
    let _ = offset;

    Ok(EventNotificationRequest {
        process_identifier,
        initiating_device_identifier,
        event_object_identifier,
        timestamp,
        notification_class,
        priority,
        event_type,
        message_text,
        notify_type,
        ack_required,
        from_state,
        to_state,
        event_values,
    })
}
