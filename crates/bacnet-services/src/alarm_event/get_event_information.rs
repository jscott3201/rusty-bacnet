use super::*;

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
        let (opt_data, _) = tags::decode_optional_context(data, 0, 0)?;
        let last_received_object_identifier = if let Some(content) = opt_data {
            Some(ObjectIdentifier::decode(content)?)
        } else {
            None
        };
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
        let mut offset = 0;

        // [0] listOfEventSummaries — opening tag
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(0) {
            return Err(Error::decoding(offset, "expected opening tag [0]"));
        }
        offset = pos;

        let mut list_of_event_summaries = Vec::new();

        // Parse event summaries until closing tag [0]
        loop {
            let (tag, _) = tags::decode_tag(data, offset)?;
            if tag.is_closing_tag(0) {
                // advance past the closing tag byte(s)
                let (_, close_pos) = tags::decode_tag(data, offset)?;
                offset = close_pos;
                break;
            }

            // [0] objectIdentifier
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "GetEventInfoAck truncated at oid"));
            }
            let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
            offset = end;

            // [1] eventState
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "GetEventInfoAck truncated at eventState",
                ));
            }
            let event_state = primitives::decode_unsigned(&data[pos..end])? as u32;
            offset = end;

            // [2] acknowledgedTransitions (3-bit bitstring)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "truncated at ackedTransitions"));
            }
            // Content: [unused_bits_count, bit_data...]
            let acknowledged_transitions = if end > pos + 1 { data[pos + 1] >> 5 } else { 0 };
            offset = end;

            // [3] eventTimeStamps — opening tag
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(3) {
                return Err(Error::decoding(offset, "expected opening tag [3]"));
            }
            offset = pos;
            let mut event_timestamps = [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ];
            for ts in &mut event_timestamps {
                let (inner_tag, inner_pos) = tags::decode_tag(data, offset)?;
                if inner_tag.is_opening_tag(0) {
                    // Time choice [0] { application Time }
                    offset = inner_pos;
                    let (app_tag, app_pos) = tags::decode_tag(data, offset)?;
                    let end = app_pos + app_tag.length as usize;
                    if end > data.len() {
                        return Err(Error::decoding(app_pos, "truncated timestamp time"));
                    }
                    *ts = BACnetTimeStamp::Time(Time::decode(&data[app_pos..end])?);
                    offset = end;
                    // closing tag [0]
                    let (_, close_pos) = tags::decode_tag(data, offset)?;
                    offset = close_pos;
                } else if inner_tag.is_context(1) {
                    // SequenceNumber choice [1]
                    let end = inner_pos + inner_tag.length as usize;
                    if end > data.len() {
                        return Err(Error::decoding(inner_pos, "truncated timestamp seqnum"));
                    }
                    *ts = BACnetTimeStamp::SequenceNumber(primitives::decode_unsigned(
                        &data[inner_pos..end],
                    )?);
                    offset = end;
                } else if inner_tag.is_opening_tag(2) {
                    // DateTime choice [2] { Date, Time }
                    offset = inner_pos;
                    let (d_tag, d_pos) = tags::decode_tag(data, offset)?;
                    let d_end = d_pos + d_tag.length as usize;
                    if d_end > data.len() {
                        return Err(Error::decoding(d_pos, "truncated datetime date"));
                    }
                    let date = Date::decode(&data[d_pos..d_end])?;
                    offset = d_end;
                    let (t_tag, t_pos) = tags::decode_tag(data, offset)?;
                    let t_end = t_pos + t_tag.length as usize;
                    if t_end > data.len() {
                        return Err(Error::decoding(t_pos, "truncated datetime time"));
                    }
                    let time = Time::decode(&data[t_pos..t_end])?;
                    offset = t_end;
                    *ts = BACnetTimeStamp::DateTime { date, time };
                    // closing tag [2]
                    let (_, close_pos) = tags::decode_tag(data, offset)?;
                    offset = close_pos;
                } else {
                    return Err(Error::decoding(offset, "unexpected timestamp choice"));
                }
            }
            // closing tag [3]
            let (tag, _) = tags::decode_tag(data, offset)?;
            if !tag.is_closing_tag(3) {
                return Err(Error::decoding(offset, "expected closing tag [3]"));
            }
            let (_, close_pos) = tags::decode_tag(data, offset)?;
            offset = close_pos;

            // [4] notifyType
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "truncated at notifyType"));
            }
            let notify_type = primitives::decode_unsigned(&data[pos..end])? as u32;
            offset = end;

            // [5] eventEnable (3-bit bitstring)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "truncated at eventEnable"));
            }
            let event_enable = if end > pos + 1 { data[pos + 1] >> 5 } else { 0 };
            offset = end;

            // [6] eventPriorities — opening tag
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(6) {
                return Err(Error::decoding(offset, "expected opening tag [6]"));
            }
            offset = pos;
            let mut event_priorities = [0u32; 3];
            for pri in &mut event_priorities {
                let (tag, pos) = tags::decode_tag(data, offset)?;
                let end = pos + tag.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(pos, "truncated priority"));
                }
                *pri = primitives::decode_unsigned(&data[pos..end])? as u32;
                offset = end;
            }
            // closing tag [6]
            let (tag, _) = tags::decode_tag(data, offset)?;
            if !tag.is_closing_tag(6) {
                return Err(Error::decoding(offset, "expected closing tag [6]"));
            }
            let (_, close_pos) = tags::decode_tag(data, offset)?;
            offset = close_pos;

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

        // [1] moreEvents
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "truncated at moreEvents"));
        }
        let more_events = data[pos] != 0;

        Ok(Self {
            list_of_event_summaries,
            more_events,
        })
    }

    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] listOfEventSummaries
        tags::encode_opening_tag(buf, 0);
        for summary in &self.list_of_event_summaries {
            // [0] objectIdentifier
            primitives::encode_ctx_object_id(buf, 0, &summary.object_identifier);
            // [1] eventState
            primitives::encode_ctx_enumerated(buf, 1, summary.event_state);
            // [2] acknowledgedTransitions (3-bit bitstring)
            primitives::encode_ctx_bit_string(buf, 2, 5, &[summary.acknowledged_transitions << 5]);
            // [3] eventTimeStamps (SEQUENCE OF 3 BACnetTimeStamp)
            tags::encode_opening_tag(buf, 3);
            for ts in &summary.event_timestamps {
                // Each timestamp is encoded as a bare CHOICE (no extra wrapping)
                // within the SEQUENCE OF
                match ts {
                    BACnetTimeStamp::Time(t) => {
                        tags::encode_opening_tag(buf, 0);
                        primitives::encode_app_time(buf, t);
                        tags::encode_closing_tag(buf, 0);
                    }
                    BACnetTimeStamp::SequenceNumber(n) => {
                        primitives::encode_ctx_unsigned(buf, 1, *n);
                    }
                    BACnetTimeStamp::DateTime { date, time } => {
                        tags::encode_opening_tag(buf, 2);
                        primitives::encode_app_date(buf, date);
                        primitives::encode_app_time(buf, time);
                        tags::encode_closing_tag(buf, 2);
                    }
                }
            }
            tags::encode_closing_tag(buf, 3);
            // [4] notifyType
            primitives::encode_ctx_enumerated(buf, 4, summary.notify_type);
            // [5] eventEnable (3-bit bitstring)
            primitives::encode_ctx_bit_string(buf, 5, 5, &[summary.event_enable << 5]);
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
    }
}
