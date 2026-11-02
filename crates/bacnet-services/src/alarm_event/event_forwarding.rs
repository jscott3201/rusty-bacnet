use super::*;
use bacnet_encoding::constructed::tagged::{
    decode_ctx_boolean, decode_ctx_object_id, decode_ctx_primitive, decode_ctx_unsigned,
    expect_end, misplaced_tag, next_is_context,
};
use bacnet_encoding::constructed::{encode_event_notification, validate_tlv_sequence};
use bytes::Bytes;

// ---------------------------------------------------------------------------
// ForwardedEventNotification
// ---------------------------------------------------------------------------

/// An event notification as a Notification Forwarder handles it (Clause
/// 12.51): the members its filters and routing read, and the encoded request
/// it sends on.
///
/// A forwarded notification differs from the one received only in its process
/// identifier, which each destination supplies. Everything after that first
/// member is carried as received, octet for octet, so the message text keeps
/// whatever character set it arrived in and event values of a type this stack
/// does not model pass through unread.
///
/// [`decode`](Self::decode) checks the request's structure: each required
/// member present and in order, with the optional ones where the request
/// allows them. It does not decode the message text or the event values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardedEventNotification {
    /// Process identifier the notification was addressed to (`[0]`).
    pub process_identifier: u32,
    /// Device whose object generated the event (`[1]`).
    pub initiating_device_identifier: ObjectIdentifier,
    /// Object whose transition the notification reports (`[2]`).
    pub event_object_identifier: ObjectIdentifier,
    /// Notification class the originating object names (`[4]`).
    pub notification_class: u32,
    /// Event priority (`[5]`), which also sets the NPDU priority of each
    /// forwarded copy.
    pub priority: u8,
    /// Alarm, event or acknowledgment notification (`[8]`).
    pub notify_type: NotifyType,
    /// Event state after the transition (`[11]`); for an acknowledgment, the
    /// state of the transition acknowledged.
    pub to_state: EventState,
    /// The encoded request after the process identifier.
    tail: Bytes,
}

impl ForwardedEventNotification {
    /// Read a ConfirmedEventNotification or UnconfirmedEventNotification
    /// request. Errors when a required member is missing, a member is out of
    /// order or malformed, or anything follows the event values.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        const WHAT: &str = "EventNotification";
        validate_tlv_sequence(data, WHAT)?;
        let (process_identifier, tail_start) =
            decode_ctx_unsigned::<u32>(data, 0, 0, "EventNotification processIdentifier")?;
        let (initiating_device_identifier, offset) = decode_ctx_object_id(
            data,
            tail_start,
            1,
            "EventNotification initiatingDeviceIdentifier",
        )?;
        let (event_object_identifier, offset) =
            decode_ctx_object_id(data, offset, 2, "EventNotification eventObjectIdentifier")?;
        let (_, offset) = primitives::decode_timestamp(data, offset, 3)?;
        let (notification_class, offset) =
            decode_ctx_unsigned::<u32>(data, offset, 4, "EventNotification notificationClass")?;
        let (priority, offset) =
            decode_ctx_unsigned::<u8>(data, offset, 5, "EventNotification priority")?;
        let (_, mut offset) =
            decode_ctx_unsigned::<u32>(data, offset, 6, "EventNotification eventType")?;
        if next_is_context(data, offset, 7)? {
            (_, offset) = decode_ctx_primitive(data, offset, 7, "EventNotification messageText")?;
        }
        let (notify_type, mut offset) =
            decode_ctx_unsigned::<u32>(data, offset, 8, "EventNotification notifyType")?;
        let notify_type = NotifyType::from_raw(notify_type);
        // ackRequired, fromState and eventValues belong to an ALARM or EVENT
        // only, as in `decode_event_notification` (#2247): for an
        // ACK_NOTIFICATION, To State must follow the notify type, and nothing
        // may follow To State.
        let transition = notify_type != NotifyType::ACK_NOTIFICATION;
        if transition && next_is_context(data, offset, 9)? {
            (_, offset) = decode_ctx_boolean(data, offset, 9, "EventNotification ackRequired")?;
        }
        if transition && next_is_context(data, offset, 10)? {
            (_, offset) =
                decode_ctx_unsigned::<u32>(data, offset, 10, "EventNotification fromState")?;
        } else if transition {
            if offset < data.len() {
                let (found, _) = tags::decode_tag(data, offset)?;
                return Err(misplaced_tag(
                    data,
                    &found,
                    Some(10),
                    offset,
                    "EventNotification expected fromState",
                ));
            }
            return Err(Error::missing(
                offset,
                "EventNotification missing fromState",
            ));
        }
        let (to_state, offset) =
            decode_ctx_unsigned::<u32>(data, offset, 11, "EventNotification toState")?;
        let to_state = EventState::from_raw(to_state);
        if !transition {
            expect_end(data, offset, offset, "EventNotification")?;
        } else if offset < data.len() {
            // The event values are one non-empty constructed [12] member, and
            // the request ends at the closing tag that matches its opening one.
            let (opening, inner) = tags::decode_tag(data, offset)?;
            if !opening.is_opening_tag(12) {
                return Err(misplaced_tag(
                    data,
                    &opening,
                    Some(12),
                    offset,
                    "EventNotification expected eventValues after toState",
                ));
            }
            let (values, end) = tags::extract_context_value(data, inner, 12)?;
            if values.is_empty() {
                // The frame holds none of the CHOICE's alternatives.
                return Err(Error::missing(
                    offset,
                    "EventNotification eventValues are empty",
                ));
            }
            expect_end(data, end, end, "EventNotification")?;
        }
        Ok(Self {
            process_identifier,
            initiating_device_identifier,
            event_object_identifier,
            notification_class,
            priority,
            notify_type,
            to_state,
            tail: Bytes::copy_from_slice(&data[tail_start..]),
        })
    }

    /// The forwarding view of a notification this device built itself.
    pub fn from_request(request: &EventNotificationRequest) -> Result<Self, Error> {
        let mut buf = BytesMut::new();
        encode_event_notification(request, &mut buf)?;
        Self::decode(&buf)
    }

    /// The same notification addressed to `process_identifier`, as one
    /// forwarder hands it to the next within a device.
    pub fn retargeted(&self, process_identifier: u32) -> Self {
        Self {
            process_identifier,
            ..self.clone()
        }
    }

    /// The request to send on to a destination: the notification as
    /// received, its process identifier replaced by `process_identifier`.
    pub fn encode_for(&self, process_identifier: u32) -> Bytes {
        let mut buf = BytesMut::with_capacity(self.tail.len() + 6);
        primitives::encode_ctx_unsigned(&mut buf, 0, u64::from(process_identifier));
        buf.extend_from_slice(&self.tail);
        buf.freeze()
    }
}
