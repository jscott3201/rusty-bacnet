use super::*;
use crate::common::{decode_context, decode_context_bool, decode_context_u32};

// ---------------------------------------------------------------------------
// EventNotification
// ---------------------------------------------------------------------------

/// ConfirmedEventNotification / UnconfirmedEventNotification request parameters.
#[derive(Debug, Clone)]
pub struct EventNotificationRequest {
    /// Process identifier of the notification recipient.
    pub process_identifier: u32,
    /// Device that generated the event.
    pub initiating_device_identifier: ObjectIdentifier,
    /// Object that triggered the event.
    pub event_object_identifier: ObjectIdentifier,
    /// Timestamp of the event transition.
    pub timestamp: BACnetTimeStamp,
    /// Notification class for routing.
    pub notification_class: u32,
    /// Priority (0-255).
    pub priority: u8,
    /// Event type (e.g., OUT_OF_RANGE = 5).
    pub event_type: u32,
    /// Optional message text ([7]).
    pub message_text: Option<String>,
    /// Notify type: ALARM(0), EVENT(1), ACK_NOTIFICATION(2).
    pub notify_type: u32,
    /// Whether the recipient must acknowledge.
    pub ack_required: bool,
    /// Event state before this transition.
    pub from_state: u32,
    /// Event state after this transition.
    pub to_state: u32,
    /// Optional event values (tag [12]).
    pub event_values: Option<NotificationParameters>,
}

impl EventNotificationRequest {
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] processIdentifier
        primitives::encode_ctx_unsigned(buf, 0, self.process_identifier as u64);
        // [1] initiatingDeviceIdentifier
        primitives::encode_ctx_object_id(buf, 1, &self.initiating_device_identifier);
        // [2] eventObjectIdentifier
        primitives::encode_ctx_object_id(buf, 2, &self.event_object_identifier);
        // [3] timeStamp
        primitives::encode_timestamp(buf, 3, &self.timestamp)?;
        // [4] notificationClass
        primitives::encode_ctx_unsigned(buf, 4, self.notification_class as u64);
        // [5] priority
        primitives::encode_ctx_unsigned(buf, 5, self.priority as u64);
        // [6] eventType
        primitives::encode_ctx_enumerated(buf, 6, self.event_type);
        // [7] messageText (optional)
        if let Some(ref text) = self.message_text {
            primitives::encode_ctx_character_string(buf, 7, text)?;
        }
        // [8] notifyType
        primitives::encode_ctx_enumerated(buf, 8, self.notify_type);
        // [9] ackRequired (only for ALARM/EVENT)
        if self.notify_type != 2 {
            primitives::encode_ctx_boolean(buf, 9, self.ack_required);
        }
        // [10] fromState
        primitives::encode_ctx_enumerated(buf, 10, self.from_state);
        // [11] toState
        primitives::encode_ctx_enumerated(buf, 11, self.to_state);
        // [12] eventValues — optional
        if let Some(ref params) = self.event_values {
            tags::encode_opening_tag(buf, 12);
            params.encode(buf)?;
            tags::encode_closing_tag(buf, 12);
        }
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
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
            Error::decoding(priority_offset, "EventNotification priority exceeds u8")
        })?;
        offset = new_offset;

        // [6] eventType
        let (event_type, new_offset) =
            decode_context_u32(data, offset, 6, "EventNotification eventType")?;
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
        let (notify_type, new_offset) =
            decode_context_u32(data, offset, 8, "EventNotification notifyType")?;
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

        // [10] fromState
        let (from_state, new_offset) =
            decode_context_u32(data, offset, 10, "EventNotification fromState")?;
        offset = new_offset;

        // [11] toState
        let (to_state, new_offset) =
            decode_context_u32(data, offset, 11, "EventNotification toState")?;
        offset = new_offset;

        // [12] eventValues — optional
        let mut event_values = None;
        if offset < data.len() {
            let (peek, _) = tags::decode_tag(data, offset)?;
            if !peek.is_opening || peek.number != 12 {
                return Err(Error::decoding(
                    offset,
                    "EventNotification expected opening tag 12 for eventValues",
                ));
            }
            // Skip opening tag [12]
            let (_, inner_start) = tags::decode_tag(data, offset)?;
            event_values = Some(NotificationParameters::decode(data, inner_start)?);
            let (variant, variant_start) = tags::decode_tag(data, inner_start)?;
            if variant.number == 8 {
                let (_, variant_end) =
                    tags::extract_context_value(data, variant_start, variant.number)?;
                let (closing, next) = tags::decode_tag(data, variant_end)?;
                if !closing.is_closing_tag(12) {
                    return Err(Error::decoding(
                        variant_end,
                        "EventNotification expected closing tag 12 after eventValues",
                    ));
                }
                if next != data.len() {
                    return Err(Error::decoding(
                        next,
                        "EventNotification unexpected trailing data",
                    ));
                }
                offset = next;
            } else {
                // Legacy variants may contain opaque bytes that cannot be scanned as tags.
                let mut scan = inner_start;
                let mut depth: usize = 1;
                while depth > 0 && scan < data.len() {
                    let (tag, next) = tags::decode_tag(data, scan)?;
                    if tag.is_opening {
                        depth += 1;
                        scan = next;
                    } else if tag.is_closing {
                        depth -= 1;
                        if depth == 0 {
                            offset = next;
                        } else {
                            scan = next;
                        }
                    } else {
                        let end = next.saturating_add(tag.length as usize);
                        if end > data.len() {
                            return Err(Error::decoding(
                                next,
                                "EventNotification: truncated tag in eventValues",
                            ));
                        }
                        scan = end;
                    }
                }
            }
        }
        let _ = offset;

        Ok(Self {
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
}
