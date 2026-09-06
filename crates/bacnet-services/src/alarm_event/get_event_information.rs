use super::*;
use crate::common::{decode_context, decode_context_bool, decode_context_u32};

fn decode_event_transition_bits(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(u8, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    if content.len() != 2 || content[0] != 5 || content[1] & 0x1f != 0 {
        return Err(Error::decoding(
            offset,
            format!("{field} must contain three bits with zero padding"),
        ));
    }
    Ok((bacnet_types::bitstring::unpack_octet(&content[1..], 3), end))
}

fn decode_application_u32(data: &[u8], offset: usize, field: &str) -> Result<(u32, usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != tags::TagClass::Application || tag.number != tags::app_tag::UNSIGNED {
        return Err(Error::decoding(
            offset,
            format!("{field} expected application Unsigned"),
        ));
    }
    let end = pos
        .checked_add(tag.length as usize)
        .ok_or_else(|| Error::decoding(pos, format!("{field} length overflow")))?;
    if end > data.len() {
        return Err(Error::decoding(pos, format!("{field} truncated")));
    }
    let value = primitives::decode_unsigned(&data[pos..end])?;
    let value = u32::try_from(value)
        .map_err(|_| Error::decoding(offset, format!("{field} exceeds u32")))?;
    Ok((value, end))
}

// GetEventInformation
// ---------------------------------------------------------------------------

/// GetEventInformation-Request — optional last_received_object_identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetEventInformationRequest {
    pub last_received_object_identifier: Option<ObjectIdentifier>,
}

impl GetEventInformationRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        if let Some(ref oid) = self.last_received_object_identifier {
            primitives::encode_ctx_object_id(buf, 0, oid);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.is_empty() {
            return Ok(Self {
                last_received_object_identifier: None,
            });
        }
        let (content, end) = decode_context(
            data,
            0,
            0,
            "GetEventInformation last-received-object-identifier",
        )?;
        if end != data.len() {
            return Err(Error::decoding(
                end,
                "GetEventInformation request contains trailing data",
            ));
        }
        let last_received_object_identifier = Some(ObjectIdentifier::decode(content)?);
        Ok(Self {
            last_received_object_identifier,
        })
    }
}

/// GetEventInformation-ACK service parameters (simplified).
#[derive(Debug, Clone)]
pub struct GetEventInformationAck {
    pub list_of_event_summaries: Vec<EventSummary>,
    pub more_events: bool,
}

/// Event summary for GetEventInformation-ACK.
#[derive(Debug, Clone)]
pub struct EventSummary {
    pub object_identifier: ObjectIdentifier,
    pub event_state: u32,
    /// 3-bit bitstring: TO_OFFNORMAL, TO_FAULT, TO_NORMAL
    pub acknowledged_transitions: u8,
    /// Timestamps for TO_OFFNORMAL, TO_FAULT, TO_NORMAL
    pub event_timestamps: [BACnetTimeStamp; 3],
    /// Notify type: ALARM(0), EVENT(1), ACK_NOTIFICATION(2)
    pub notify_type: u32,
    /// 3-bit bitstring: TO_OFFNORMAL, TO_FAULT, TO_NORMAL
    pub event_enable: u8,
    /// Priorities for TO_OFFNORMAL, TO_FAULT, TO_NORMAL
    pub event_priorities: [u32; 3],
    pub notification_class: u32,
}

impl GetEventInformationAck {
    /// Decode a GetEventInformationAck from wire bytes.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (tag, mut offset) = tags::decode_tag(data, 0)?;
        if !tag.is_opening_tag(0) {
            return Err(Error::decoding(
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
                return Err(Error::decoding(
                    offset,
                    format!("GetEventInformation ACK exceeds {MAX_DECODED_ITEMS} event summaries"),
                ));
            }

            let (content, end) =
                decode_context(data, offset, 0, "GetEventInformation ACK object-identifier")?;
            let object_identifier = ObjectIdentifier::decode(content)?;
            offset = end;

            let (event_state, end) =
                decode_context_u32(data, offset, 1, "GetEventInformation ACK event-state")?;
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
                return Err(Error::decoding(
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
                return Err(Error::decoding(
                    offset,
                    "GetEventInformation ACK expected closing tag 3 for event-timestamps",
                ));
            }
            offset = next;

            let (notify_type, end) =
                decode_context_u32(data, offset, 4, "GetEventInformation ACK notify-type")?;
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
                return Err(Error::decoding(
                    offset,
                    "GetEventInformation ACK expected opening tag 6 for event-priorities",
                ));
            }
            offset = next;
            let mut event_priorities = [0u32; 3];
            for pri in &mut event_priorities {
                let (value, end) =
                    decode_application_u32(data, offset, "GetEventInformation ACK event-priority")?;
                *pri = value;
                offset = end;
            }
            let (tag, next) = tags::decode_tag(data, offset)?;
            if !tag.is_closing_tag(6) {
                return Err(Error::decoding(
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
                notification_class: 0, // not present in the wire format
            });
        }

        let (more_events, end) =
            decode_context_bool(data, offset, 1, "GetEventInformation ACK more-events")?;
        if end != data.len() {
            return Err(Error::decoding(
                end,
                "GetEventInformation ACK contains trailing data",
            ));
        }

        Ok(Self {
            list_of_event_summaries,
            more_events,
        })
    }

    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] listOfEventSummaries
        tags::encode_opening_tag(buf, 0);
        for summary in &self.list_of_event_summaries {
            // [0] objectIdentifier
            primitives::encode_ctx_object_id(buf, 0, &summary.object_identifier);
            // [1] eventState
            primitives::encode_ctx_enumerated(buf, 1, summary.event_state);
            // [2] acknowledgedTransitions (3-bit bitstring)
            primitives::encode_ctx_bit_string(
                buf,
                2,
                5,
                &[bacnet_types::bitstring::pack_octet(
                    summary.acknowledged_transitions,
                )],
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
            primitives::encode_ctx_enumerated(buf, 4, summary.notify_type);
            // [5] eventEnable (3-bit bitstring)
            primitives::encode_ctx_bit_string(
                buf,
                5,
                5,
                &[bacnet_types::bitstring::pack_octet(summary.event_enable)],
            );
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
