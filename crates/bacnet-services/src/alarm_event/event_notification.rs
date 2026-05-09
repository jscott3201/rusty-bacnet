use super::*;

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
        primitives::encode_timestamp(buf, 3, &self.timestamp);
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
        let mut offset = 0;

        // Helper: validate bounds after computing end from tag length
        macro_rules! check_bounds {
            ($pos:expr, $end:expr, $field:expr) => {
                if $end > data.len() {
                    return Err(Error::decoding(
                        $pos,
                        concat!("EventNotification truncated at ", $field),
                    ));
                }
            };
        }

        // [0] processIdentifier
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "processIdentifier");
        let process_identifier = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [1] initiatingDeviceIdentifier
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "initiatingDeviceIdentifier");
        let initiating_device_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [2] eventObjectIdentifier
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "eventObjectIdentifier");
        let event_object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [3] timeStamp
        let (timestamp, new_offset) = primitives::decode_timestamp(data, offset, 3)?;
        offset = new_offset;

        // [4] notificationClass
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "notificationClass");
        let notification_class = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [5] priority
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "priority");
        let priority = primitives::decode_unsigned(&data[pos..end])? as u8;
        offset = end;

        // [6] eventType
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "eventType");
        let event_type = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [7] messageText (optional)
        let mut message_text = None;
        if offset < data.len() {
            let (peek, peek_pos) = tags::decode_tag(data, offset)?;
            if peek.is_context(7) {
                let mt_end = peek_pos + peek.length as usize;
                if mt_end <= data.len() {
                    message_text = Some(primitives::decode_character_string(
                        &data[peek_pos..mt_end],
                    )?);
                }
                offset = mt_end;
            }
        }

        // Skip any remaining tags before [8] notifyType
        let mut skip_count = 0u32;
        while offset < data.len() {
            skip_count += 1;
            if skip_count > MAX_DECODED_ITEMS as u32 {
                return Err(Error::decoding(
                    offset,
                    "too many tags skipped looking for notification-parameters",
                ));
            }
            let (peek, peek_pos) = tags::decode_tag(data, offset)?;
            if peek.is_context(8) {
                break;
            }
            if peek.is_opening {
                let (_, new_offset) = tags::extract_context_value(data, peek_pos, peek.number)?;
                offset = new_offset;
            } else if peek.is_closing {
                return Err(Error::decoding(
                    offset,
                    "unexpected closing tag skipping to notification-parameters",
                ));
            } else {
                let skip_end = peek_pos + peek.length as usize;
                if skip_end > data.len() {
                    return Err(Error::decoding(
                        peek_pos,
                        "EventNotification truncated skipping messageText",
                    ));
                }
                offset = skip_end;
            }
        }

        // [8] notifyType
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "notifyType");
        let notify_type = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [9] ackRequired (optional — present for ALARM/EVENT)
        let mut ack_required = false;
        if offset < data.len() {
            let (peek, peek_pos) = tags::decode_tag(data, offset)?;
            if peek.is_context(9) {
                let end = peek_pos + peek.length as usize;
                check_bounds!(peek_pos, end, "ackRequired");
                ack_required = data[peek_pos] != 0;
                offset = end;
            }
        }

        // [10] fromState
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "fromState");
        let from_state = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [11] toState
        let (_tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + _tag.length as usize;
        check_bounds!(pos, end, "toState");
        let to_state = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [12] eventValues — optional
        let mut event_values = None;
        if offset < data.len() {
            let (peek, _) = tags::decode_tag(data, offset)?;
            if peek.is_opening && peek.number == 12 {
                // Skip opening tag [12]
                let (_, inner_start) = tags::decode_tag(data, offset)?;
                event_values = Some(NotificationParameters::decode(data, inner_start)?);
                // Find closing tag [12]
                let mut scan = inner_start;
                let mut depth: usize = 1;
                while depth > 0 && scan < data.len() {
                    let (t, next) = tags::decode_tag(data, scan)?;
                    if t.is_opening {
                        depth += 1;
                        scan = next;
                    } else if t.is_closing {
                        depth -= 1;
                        if depth == 0 {
                            offset = next;
                        } else {
                            scan = next;
                        }
                    } else {
                        let end = next.saturating_add(t.length as usize);
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
