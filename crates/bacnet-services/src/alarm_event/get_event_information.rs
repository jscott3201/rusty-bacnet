#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use super::*;
use bacnet_encoding::constructed::tagged::{
    decode_app_unsigned, decode_ctx_boolean, decode_ctx_object_id, decode_ctx_primitive,
    decode_ctx_unsigned, expect_end, misplaced_tag, unclosed_kind,
};
use bacnet_types::bitstring::EventTransitionBits;

fn decode_event_transition_bits(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &'static str,
) -> Result<(EventTransitionBits, usize), Error> {
    let (content, end) = decode_ctx_primitive(data, offset, expected_tag, field)?;
    if content.len() != 2 || content[0] != 5 || content[1] & 0x1f != 0 {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "must contain three bits with zero padding",
        )
        .within(field)
        .into());
    }
    Ok((EventTransitionBits::from_bacnet(&content[1..]), end))
}

// GetEventInformation
// ---------------------------------------------------------------------------

/// GetEventInformation-Request — optional last_received_object_identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetEventInformationRequest {
    /// Continuation cursor: last object from the previous response; `None` requests the first page.
    pub last_received_object_identifier: Option<ObjectIdentifier>,
}

impl GetEventInformationRequest {
    /// Append the ASN.1 encoding of the request to `buf`.
    pub fn encode(&self, buf: &mut BytesMut) {
        if let Some(ref oid) = self.last_received_object_identifier {
            primitives::encode_ctx_object_id(buf, 0, oid);
        }
    }

    /// Decode the request from `data`; errors on malformed input or trailing bytes.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.is_empty() {
            return Ok(Self {
                last_received_object_identifier: None,
            });
        }
        let (object_identifier, end) = decode_ctx_object_id(
            data,
            0,
            0,
            "GetEventInformation last-received-object-identifier",
        )?;
        expect_end(data, end, end, "GetEventInformation")?;
        Ok(Self {
            last_received_object_identifier: Some(object_identifier),
        })
    }
}

/// GetEventInformation-ACK service parameters (Clause 13.12.1.2).
#[derive(Debug, Clone)]
pub struct GetEventInformationAck {
    /// Objects with a non-normal event state or unacknowledged transitions.
    pub list_of_event_summaries: Vec<EventSummary>,
    /// `true` when more summaries remain beyond this response.
    pub more_events: bool,
}

/// Event summary for GetEventInformation-ACK.
#[derive(Debug, Clone)]
pub struct EventSummary {
    /// Object these event details describe.
    pub object_identifier: ObjectIdentifier,
    /// The object's `Event_State`.
    pub event_state: EventState,
    /// The object's `Acked_Transitions`.
    pub acknowledged_transitions: EventTransitionBits,
    /// Timestamps for TO_OFFNORMAL, TO_FAULT, TO_NORMAL
    pub event_timestamps: [BACnetTimeStamp; 3],
    /// The object's `Notify_Type`.
    pub notify_type: NotifyType,
    /// The object's `Event_Enable`.
    pub event_enable: EventTransitionBits,
    /// Priorities for TO_OFFNORMAL, TO_FAULT, TO_NORMAL
    pub event_priorities: [u32; 3],
}

impl GetEventInformationAck {
    /// Decode a GetEventInformationAck from wire bytes.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (tag, mut offset) = tags::decode_tag(data, 0)?;
        if !tag.is_opening_tag(0) {
            return Err(misplaced_tag(
                data,
                &tag,
                Some(0),
                0,
                "GetEventInformation ACK expected opening tag 0",
            ));
        }

        let mut list_of_event_summaries = Vec::new();
        loop {
            let (tag, next) = tags::decode_tag(data, offset)?;
            if tag.is_closing_tag(0) {
                offset = next;
                break;
            }
            if list_of_event_summaries.len() >= MAX_DECODED_ITEMS {
                return Err(DecodeError::new(
                    DecodingKind::Overflow,
                    offset,
                    "GetEventInformation ACK exceeds {} event summaries",
                )
                .with_value(MAX_DECODED_ITEMS as u64)
                .into());
            }

            let (object_identifier, end) =
                decode_ctx_object_id(data, offset, 0, "GetEventInformation ACK object-identifier")?;
            offset = end;

            let (event_state, end) =
                decode_ctx_unsigned::<u32>(data, offset, 1, "GetEventInformation ACK event-state")?;
            let event_state = EventState::from_raw(event_state);
            offset = end;

            let (acknowledged_transitions, end) = decode_event_transition_bits(
                data,
                offset,
                2,
                "GetEventInformation ACK acknowledged-transitions",
            )?;
            offset = end;

            let (tag, next) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(3) {
                return Err(misplaced_tag(
                    data,
                    &tag,
                    Some(3),
                    offset,
                    "GetEventInformation ACK expected opening tag 3 for event-timestamps",
                ));
            }
            offset = next;
            let mut event_timestamps = [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ];
            for ts in &mut event_timestamps {
                let (decoded_ts, new_offset) = primitives::decode_timestamp_choice(data, offset)?;
                *ts = decoded_ts;
                offset = new_offset;
            }
            let (tag, next) = tags::decode_tag(data, offset)?;
            if !tag.is_closing_tag(3) {
                return Err(Error::decoding_kind(
                    unclosed_kind(&tag),
                    offset,
                    "GetEventInformation ACK expected closing tag 3 for event-timestamps",
                ));
            }
            offset = next;

            let (notify_type, end) =
                decode_ctx_unsigned::<u32>(data, offset, 4, "GetEventInformation ACK notify-type")?;
            let notify_type = NotifyType::from_raw(notify_type);
            offset = end;

            let (event_enable, end) = decode_event_transition_bits(
                data,
                offset,
                5,
                "GetEventInformation ACK event-enable",
            )?;
            offset = end;

            let (tag, next) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(6) {
                return Err(misplaced_tag(
                    data,
                    &tag,
                    Some(6),
                    offset,
                    "GetEventInformation ACK expected opening tag 6 for event-priorities",
                ));
            }
            offset = next;
            let mut event_priorities = [0u32; 3];
            for pri in &mut event_priorities {
                let (value, end) = decode_app_unsigned::<u32>(
                    data,
                    offset,
                    "GetEventInformation ACK event-priority",
                )?;
                *pri = value;
                offset = end;
            }
            let (tag, next) = tags::decode_tag(data, offset)?;
            if !tag.is_closing_tag(6) {
                return Err(Error::decoding_kind(
                    unclosed_kind(&tag),
                    offset,
                    "GetEventInformation ACK expected closing tag 6 for event-priorities",
                ));
            }
            offset = next;

            list_of_event_summaries.push(EventSummary {
                object_identifier,
                event_state,
                acknowledged_transitions,
                event_timestamps,
                notify_type,
                event_enable,
                event_priorities,
            });
        }

        let (more_events, end) =
            decode_ctx_boolean(data, offset, 1, "GetEventInformation ACK more-events")?;
        expect_end(data, end, end, "GetEventInformation ACK")?;

        Ok(Self {
            list_of_event_summaries,
            more_events,
        })
    }

    /// Append the ASN.1 encoding of the ACK to `buf`; fails if a timestamp is unencodable.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] listOfEventSummaries
        tags::encode_opening_tag(buf, 0);
        for summary in &self.list_of_event_summaries {
            // [0] objectIdentifier
            primitives::encode_ctx_object_id(buf, 0, &summary.object_identifier);
            // [1] eventState
            primitives::encode_ctx_enumerated(buf, 1, summary.event_state.to_raw());
            // [2] acknowledgedTransitions (3-bit bitstring)
            primitives::encode_ctx_bit_string(
                buf,
                2,
                5,
                &[summary.acknowledged_transitions.to_bacnet()],
            );
            // [3] eventTimeStamps (SEQUENCE OF 3 BACnetTimeStamp)
            tags::encode_opening_tag(buf, 3);
            for ts in &summary.event_timestamps {
                // Each timestamp is a bare CHOICE item of the SEQUENCE OF
                // (no extra wrapping) — encoded by the shared primitives
                // codec so this service and every other timestamp producer
                // agree on the wire bytes.
                primitives::encode_timestamp_choice(buf, ts)?;
            }
            tags::encode_closing_tag(buf, 3);
            // [4] notifyType
            primitives::encode_ctx_enumerated(buf, 4, summary.notify_type.to_raw());
            // [5] eventEnable (3-bit bitstring)
            primitives::encode_ctx_bit_string(buf, 5, 5, &[summary.event_enable.to_bacnet()]);
            // [6] eventPriorities (SEQUENCE OF 3 Unsigned)
            tags::encode_opening_tag(buf, 6);
            for &p in &summary.event_priorities {
                primitives::encode_app_unsigned(buf, p as u64);
            }
            tags::encode_closing_tag(buf, 6);
        }
        tags::encode_closing_tag(buf, 0);
        // [1] moreEvents
        primitives::encode_ctx_boolean(buf, 1, self.more_events);
        Ok(())
    }
}
