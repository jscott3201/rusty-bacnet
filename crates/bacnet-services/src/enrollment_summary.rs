//! GetEnrollmentSummary service per ASHRAE 135-2020 Clause 13.8.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::{EventState, EventType};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

// ---------------------------------------------------------------------------
// GetEnrollmentSummaryRequest (Clause 13.8.1)
// ---------------------------------------------------------------------------

/// Priority filter sub-structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriorityFilter {
    pub min_priority: u8,
    pub max_priority: u8,
}

/// GetEnrollmentSummary-Request service parameters.
///
/// `enrollmentFilter` ([1] BACnetRecipientProcess) is omitted — it requires
/// the full Recipient/RecipientProcess types which are rarely used in
/// practice. A compliant implementation would extend this struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetEnrollmentSummaryRequest {
    /// [0] acknowledgmentFilter: all(0), acked(1), not-acked(2).
    pub acknowledgment_filter: u32,
    /// [2] eventStateFilter (optional).
    pub event_state_filter: Option<EventState>,
    /// [3] eventTypeFilter (optional).
    pub event_type_filter: Option<EventType>,
    /// [4] priorityFilter { [0] minPriority, [1] maxPriority } (optional).
    pub priority_filter: Option<PriorityFilter>,
    /// [5] notificationClassFilter (optional).
    pub notification_class_filter: Option<u16>,
}

impl GetEnrollmentSummaryRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] acknowledgmentFilter
        primitives::encode_ctx_enumerated(buf, 0, self.acknowledgment_filter);
        // [1] enrollmentFilter — not implemented (skip)
        // [2] eventStateFilter (optional)
        if let Some(es) = self.event_state_filter {
            primitives::encode_ctx_enumerated(buf, 2, es.to_raw());
        }
        // [3] eventTypeFilter (optional)
        if let Some(et) = self.event_type_filter {
            primitives::encode_ctx_enumerated(buf, 3, et.to_raw());
        }
        // [4] priorityFilter (optional, constructed)
        if let Some(pf) = self.priority_filter {
            tags::encode_opening_tag(buf, 4);
            primitives::encode_ctx_unsigned(buf, 0, pf.min_priority as u64);
            primitives::encode_ctx_unsigned(buf, 1, pf.max_priority as u64);
            tags::encode_closing_tag(buf, 4);
        }
        // [5] notificationClassFilter (optional)
        if let Some(nc) = self.notification_class_filter {
            primitives::encode_ctx_unsigned(buf, 5, nc as u64);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] acknowledgmentFilter
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "EnrollmentSummary truncated at acknowledgmentFilter",
            ));
        }
        let acknowledgment_filter = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [1] enrollmentFilter — skip if present
        if offset < data.len() {
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_opening_tag(1) {
                // Skip over the entire constructed value
                let (_, new_offset) = tags::extract_context_value(data, tag_end, 1)?;
                offset = new_offset;
            }
        }

        // [2] eventStateFilter (optional)
        let mut event_state_filter = None;
        let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 2)?;
        if let Some(content) = opt_data {
            event_state_filter = Some(EventState::from_raw(
                primitives::decode_unsigned(content)? as u32
            ));
            offset = new_offset;
        }

        // [3] eventTypeFilter (optional)
        let mut event_type_filter = None;
        let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 3)?;
        if let Some(content) = opt_data {
            event_type_filter = Some(EventType::from_raw(
                primitives::decode_unsigned(content)? as u32
            ));
            offset = new_offset;
        }

        // [4] priorityFilter (optional, constructed)
        let mut priority_filter = None;
        if offset < data.len() {
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_opening_tag(4) {
                // [0] minPriority
                let (inner_tag, inner_pos) = tags::decode_tag(data, tag_end)?;
                let inner_end = inner_pos + inner_tag.length as usize;
                if inner_end > data.len() {
                    return Err(Error::decoding(
                        inner_pos,
                        "EnrollmentSummary truncated at minPriority",
                    ));
                }
                let min_priority = primitives::decode_unsigned(&data[inner_pos..inner_end])? as u8;

                // [1] maxPriority
                let (inner_tag, inner_pos) = tags::decode_tag(data, inner_end)?;
                let inner_end = inner_pos + inner_tag.length as usize;
                if inner_end > data.len() {
                    return Err(Error::decoding(
                        inner_pos,
                        "EnrollmentSummary truncated at maxPriority",
                    ));
                }
                let max_priority = primitives::decode_unsigned(&data[inner_pos..inner_end])? as u8;

                // closing tag 4
                let (close_tag, close_end) = tags::decode_tag(data, inner_end)?;
                if !close_tag.is_closing_tag(4) {
                    return Err(Error::decoding(
                        inner_end,
                        "EnrollmentSummary expected closing tag 4",
                    ));
                }
                priority_filter = Some(PriorityFilter {
                    min_priority,
                    max_priority,
                });
                offset = close_end;
            }
        }

        // [5] notificationClassFilter (optional)
        let mut notification_class_filter = None;
        if offset < data.len() {
            let (opt_data, _new_offset) = tags::decode_optional_context(data, offset, 5)?;
            if let Some(content) = opt_data {
                notification_class_filter = Some(primitives::decode_unsigned(content)? as u16);
            }
        }

        Ok(Self {
            acknowledgment_filter,
            event_state_filter,
            event_type_filter,
            priority_filter,
            notification_class_filter,
        })
    }
}

// ---------------------------------------------------------------------------
// GetEnrollmentSummaryAck (Clause 13.8.2)
// ---------------------------------------------------------------------------

/// One entry in the GetEnrollmentSummary-ACK sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentSummaryEntry {
    pub object_identifier: ObjectIdentifier,
    pub event_type: EventType,
    pub event_state: EventState,
    pub priority: u8,
    pub notification_class: u16,
}

/// GetEnrollmentSummary-ACK: a sequence of summary entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetEnrollmentSummaryAck {
    pub entries: Vec<EnrollmentSummaryEntry>,
}

impl GetEnrollmentSummaryAck {
    pub fn encode(&self, buf: &mut BytesMut) {
        for entry in &self.entries {
            primitives::encode_app_object_id(buf, &entry.object_identifier);
            primitives::encode_app_enumerated(buf, entry.event_type.to_raw());
            primitives::encode_app_enumerated(buf, entry.event_state.to_raw());
            primitives::encode_app_unsigned(buf, entry.priority as u64);
            primitives::encode_app_unsigned(buf, entry.notification_class as u64);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut entries = Vec::new();
        let mut offset = 0;

        while offset < data.len() {
            if entries.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(
                    offset,
                    "EnrollmentSummaryAck too many entries",
                ));
            }

            // objectIdentifier (app)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "EnrollmentSummaryAck truncated at object-id",
                ));
            }
            let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
            offset = end;

            // eventType (app enumerated)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "EnrollmentSummaryAck truncated at eventType",
                ));
            }
            let event_type =
                EventType::from_raw(primitives::decode_unsigned(&data[pos..end])? as u32);
            offset = end;

            // eventState (app enumerated)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "EnrollmentSummaryAck truncated at eventState",
                ));
            }
            let event_state =
                EventState::from_raw(primitives::decode_unsigned(&data[pos..end])? as u32);
            offset = end;

            // priority (app unsigned)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "EnrollmentSummaryAck truncated at priority",
                ));
            }
            let priority = primitives::decode_unsigned(&data[pos..end])? as u8;
            offset = end;

            // notificationClass (app unsigned)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "EnrollmentSummaryAck truncated at notificationClass",
                ));
            }
            let notification_class = primitives::decode_unsigned(&data[pos..end])? as u16;
            offset = end;

            entries.push(EnrollmentSummaryEntry {
                object_identifier,
                event_type,
                event_state,
                priority,
                notification_class,
            });
        }

        Ok(Self { entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    #[test]
    fn request_round_trip() {
        let req = GetEnrollmentSummaryRequest {
            acknowledgment_filter: 0, // all
            event_state_filter: Some(EventState::OFFNORMAL),
            event_type_filter: None,
            priority_filter: Some(PriorityFilter {
                min_priority: 1,
                max_priority: 10,
            }),
            notification_class_filter: Some(5),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = GetEnrollmentSummaryRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn request_minimal_round_trip() {
        let req = GetEnrollmentSummaryRequest {
            acknowledgment_filter: 2, // not-acked
            event_state_filter: None,
            event_type_filter: None,
            priority_filter: None,
            notification_class_filter: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = GetEnrollmentSummaryRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn ack_round_trip() {
        let ack = GetEnrollmentSummaryAck {
            entries: vec![
                EnrollmentSummaryEntry {
                    object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                    event_type: EventType::OUT_OF_RANGE,
                    event_state: EventState::HIGH_LIMIT,
                    priority: 3,
                    notification_class: 10,
                },
                EnrollmentSummaryEntry {
                    object_identifier: ObjectIdentifier::new(ObjectType::BINARY_INPUT, 5).unwrap(),
                    event_type: EventType::CHANGE_OF_STATE,
                    event_state: EventState::NORMAL,
                    priority: 7,
                    notification_class: 20,
                },
            ],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = GetEnrollmentSummaryAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn ack_empty_round_trip() {
        let ack = GetEnrollmentSummaryAck { entries: vec![] };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = GetEnrollmentSummaryAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_request_empty_input() {
        assert!(GetEnrollmentSummaryRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_request_truncated_1_byte() {
        let req = GetEnrollmentSummaryRequest {
            acknowledgment_filter: 0,
            event_state_filter: Some(EventState::FAULT),
            event_type_filter: None,
            priority_filter: None,
            notification_class_filter: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(GetEnrollmentSummaryRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_request_invalid_tag() {
        assert!(GetEnrollmentSummaryRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_ack_truncated_1_byte() {
        let ack = GetEnrollmentSummaryAck {
            entries: vec![EnrollmentSummaryEntry {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                event_type: EventType::OUT_OF_RANGE,
                event_state: EventState::HIGH_LIMIT,
                priority: 3,
                notification_class: 10,
            }],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        assert!(GetEnrollmentSummaryAck::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_ack_truncated_half() {
        let ack = GetEnrollmentSummaryAck {
            entries: vec![EnrollmentSummaryEntry {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                event_type: EventType::OUT_OF_RANGE,
                event_state: EventState::HIGH_LIMIT,
                priority: 3,
                notification_class: 10,
            }],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(GetEnrollmentSummaryAck::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_ack_invalid_tag() {
        assert!(GetEnrollmentSummaryAck::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
