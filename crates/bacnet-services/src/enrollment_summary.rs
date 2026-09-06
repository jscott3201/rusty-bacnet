//! GetEnrollmentSummary service per ASHRAE 135-2020 Clause 13.11.

use bacnet_encoding::constructed::{decode_recipient, encode_recipient};
use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::{EnrollmentSummaryEventStateFilter, EventState, EventType};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

fn is_application_tag(tag: &tags::Tag, header: u8, number: u8) -> bool {
    tag.class == tags::TagClass::Application && tag.number == number && header & 0x07 <= 5
}

// ---------------------------------------------------------------------------
// GetEnrollmentSummaryRequest
// ---------------------------------------------------------------------------

/// Priority filter sub-structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriorityFilter {
    pub min_priority: u8,
    pub max_priority: u8,
}

/// BACnetRecipientProcess — identifies a notification recipient.
///
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipientProcess {
    /// Device or address BACnetRecipient CHOICE.
    pub recipient: BACnetRecipient,
    /// Process identifier.
    pub process_identifier: u32,
}

/// GetEnrollmentSummary-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetEnrollmentSummaryRequest {
    /// [0] acknowledgmentFilter: all(0), acked(1), not-acked(2).
    pub acknowledgment_filter: u32,
    /// [1] enrollmentFilter (optional) — BACnetRecipientProcess.
    pub enrollment_filter: Option<RecipientProcess>,
    /// [2] eventStateFilter (optional).
    pub event_state_filter: Option<EnrollmentSummaryEventStateFilter>,
    /// [3] eventTypeFilter (optional).
    pub event_type_filter: Option<EventType>,
    /// [4] priorityFilter { [0] minPriority, [1] maxPriority } (optional).
    pub priority_filter: Option<PriorityFilter>,
    /// [5] notificationClassFilter (optional).
    pub notification_class_filter: Option<u32>,
}

impl GetEnrollmentSummaryRequest {
    /// Encode this request.
    ///
    /// # Panics
    ///
    /// Panics if a filter contains a value outside its service-defined range.
    pub fn encode(&self, buf: &mut BytesMut) {
        self.try_encode(buf)
            .expect("invalid GetEnrollmentSummary request");
    }

    /// Encode this request after validating representable filter invariants.
    pub fn try_encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        if self.acknowledgment_filter > 2 {
            return Err(Error::Encoding(
                "EnrollmentSummary acknowledgmentFilter is an undefined enumeration".into(),
            ));
        }
        if self.event_state_filter.is_some_and(|filter| {
            filter.to_raw() > EnrollmentSummaryEventStateFilter::ACTIVE.to_raw()
        }) {
            return Err(Error::Encoding(
                "EnrollmentSummary eventStateFilter is an undefined enumeration".into(),
            ));
        }
        if self
            .priority_filter
            .is_some_and(|filter| filter.min_priority > filter.max_priority)
        {
            return Err(Error::Encoding(
                "EnrollmentSummary priorityFilter minimum exceeds maximum".into(),
            ));
        }
        // [0] acknowledgmentFilter
        primitives::encode_ctx_enumerated(buf, 0, self.acknowledgment_filter);
        // [1] enrollmentFilter (optional, constructed)
        if let Some(ref ef) = self.enrollment_filter {
            tags::encode_opening_tag(buf, 1);
            tags::encode_opening_tag(buf, 0);
            encode_recipient(buf, &ef.recipient);
            tags::encode_closing_tag(buf, 0);
            primitives::encode_ctx_unsigned(buf, 1, ef.process_identifier as u64);
            tags::encode_closing_tag(buf, 1);
        }
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
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] acknowledgmentFilter
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(0) {
            return Err(Error::decoding(
                offset,
                "EnrollmentSummary expected context tag 0",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "EnrollmentSummary truncated at acknowledgmentFilter",
            ));
        }
        let acknowledgment_filter = decode_closed_enumeration(&data[pos..end], 2)?;
        offset = end;

        // [1] enrollmentFilter (optional, constructed)
        let mut enrollment_filter = None;
        if offset < data.len() {
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_opening_tag(1) {
                let (content, new_offset) = tags::extract_context_value(data, tag_end, 1)?;
                let (recipient_tag, recipient_pos) = tags::decode_tag(content, 0)?;
                if !recipient_tag.is_opening_tag(0) {
                    return Err(Error::decoding(
                        tag_end,
                        "EnrollmentSummary expected recipient tag 0",
                    ));
                }
                let (recipient_content, recipient_end) =
                    tags::extract_context_value(content, recipient_pos, 0)?;
                let (recipient, recipient_choice_end) = decode_recipient(recipient_content, 0)?;
                if recipient_choice_end != recipient_content.len() {
                    return Err(Error::decoding(
                        tag_end + recipient_pos + recipient_choice_end,
                        "EnrollmentSummary recipient has trailing data",
                    ));
                }

                let (process_tag, process_pos) = tags::decode_tag(content, recipient_end)?;
                if !process_tag.is_context(1) {
                    return Err(Error::decoding(
                        tag_end + recipient_end,
                        "EnrollmentSummary expected processIdentifier tag 1",
                    ));
                }
                let process_end = process_pos + process_tag.length as usize;
                if process_end > content.len() {
                    return Err(Error::decoding(
                        tag_end + process_pos,
                        "EnrollmentSummary truncated at processIdentifier",
                    ));
                }
                let process_id = primitives::decode_unsigned(&content[process_pos..process_end])?;
                let process_id = u32::try_from(process_id).map_err(|_| {
                    Error::decoding(
                        tag_end + process_pos,
                        "EnrollmentSummary processIdentifier exceeds u32",
                    )
                })?;
                if process_end != content.len() {
                    return Err(Error::decoding(
                        tag_end + process_end,
                        "EnrollmentSummary enrollmentFilter has trailing data",
                    ));
                }
                enrollment_filter = Some(RecipientProcess {
                    recipient,
                    process_identifier: process_id,
                });
                offset = new_offset;
            }
        }

        // [2] eventStateFilter (optional)
        let mut event_state_filter = None;
        let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 2)?;
        if let Some(content) = opt_data {
            let value = match content {
                [value] => u32::from(*value),
                [] | [0, ..] => {
                    return Err(Error::Reject {
                        reason: bacnet_types::enums::RejectReason::INVALID_DATA_ENCODING.to_raw(),
                    });
                }
                _ => {
                    return Err(Error::Reject {
                        reason: bacnet_types::enums::RejectReason::UNDEFINED_ENUMERATION.to_raw(),
                    });
                }
            };
            if value > EnrollmentSummaryEventStateFilter::ACTIVE.to_raw() {
                return Err(Error::Reject {
                    reason: bacnet_types::enums::RejectReason::UNDEFINED_ENUMERATION.to_raw(),
                });
            }
            event_state_filter = Some(EnrollmentSummaryEventStateFilter::from_raw(value));
            offset = new_offset;
        }

        // [3] eventTypeFilter (optional)
        let mut event_type_filter = None;
        let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 3)?;
        if let Some(content) = opt_data {
            let value = primitives::decode_unsigned(content)?;
            event_type_filter = Some(EventType::from_raw(u32::try_from(value).map_err(|_| {
                Error::decoding(offset, "EnrollmentSummary eventTypeFilter exceeds u32")
            })?));
            offset = new_offset;
        }

        // [4] priorityFilter (optional, constructed)
        let mut priority_filter = None;
        if offset < data.len() {
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_opening_tag(4) {
                let (content, new_offset) = tags::extract_context_value(data, tag_end, 4)?;

                // [0] minPriority
                let (inner_tag, inner_pos) = tags::decode_tag(content, 0)?;
                if !inner_tag.is_context(0) {
                    return Err(Error::decoding(
                        tag_end,
                        "EnrollmentSummary expected minPriority tag 0",
                    ));
                }
                let inner_end = inner_pos + inner_tag.length as usize;
                if inner_end > content.len() {
                    return Err(Error::decoding(
                        tag_end + inner_pos,
                        "EnrollmentSummary truncated at minPriority",
                    ));
                }
                let min_priority = primitives::decode_unsigned(&content[inner_pos..inner_end])?;
                let min_priority = u8::try_from(min_priority).map_err(|_| {
                    Error::decoding(
                        tag_end + inner_pos,
                        "EnrollmentSummary minPriority exceeds u8",
                    )
                })?;

                // [1] maxPriority
                let (inner_tag, inner_pos) = tags::decode_tag(content, inner_end)?;
                if !inner_tag.is_context(1) {
                    return Err(Error::decoding(
                        tag_end + inner_end,
                        "EnrollmentSummary expected maxPriority tag 1",
                    ));
                }
                let priority_end = inner_pos + inner_tag.length as usize;
                if priority_end > content.len() {
                    return Err(Error::decoding(
                        tag_end + inner_pos,
                        "EnrollmentSummary truncated at maxPriority",
                    ));
                }
                let max_priority = primitives::decode_unsigned(&content[inner_pos..priority_end])?;
                let max_priority = u8::try_from(max_priority).map_err(|_| {
                    Error::decoding(
                        tag_end + inner_pos,
                        "EnrollmentSummary maxPriority exceeds u8",
                    )
                })?;
                if priority_end != content.len() {
                    return Err(Error::decoding(
                        tag_end + priority_end,
                        "EnrollmentSummary priorityFilter has trailing data",
                    ));
                }
                if min_priority > max_priority {
                    return Err(Error::Reject {
                        reason: bacnet_types::enums::RejectReason::INVALID_DATA_ENCODING.to_raw(),
                    });
                }
                priority_filter = Some(PriorityFilter {
                    min_priority,
                    max_priority,
                });
                offset = new_offset;
            }
        }

        // [5] notificationClassFilter (optional)
        let mut notification_class_filter = None;
        if offset < data.len() {
            let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 5)?;
            if let Some(content) = opt_data {
                let value = primitives::decode_unsigned(content)?;
                notification_class_filter = Some(u32::try_from(value).map_err(|_| {
                    Error::decoding(
                        offset,
                        "EnrollmentSummary notificationClassFilter exceeds u32",
                    )
                })?);
                offset = new_offset;
            }
        }
        if offset != data.len() {
            return Err(Error::decoding(
                offset,
                "EnrollmentSummary has unexpected or trailing data",
            ));
        }

        Ok(Self {
            acknowledgment_filter,
            enrollment_filter,
            event_state_filter,
            event_type_filter,
            priority_filter,
            notification_class_filter,
        })
    }
}

fn decode_closed_enumeration(data: &[u8], maximum: u32) -> Result<u32, Error> {
    let value = match data {
        [value] => u32::from(*value),
        [] | [0, ..] => {
            return Err(Error::Reject {
                reason: bacnet_types::enums::RejectReason::INVALID_DATA_ENCODING.to_raw(),
            })
        }
        _ => {
            return Err(Error::Reject {
                reason: bacnet_types::enums::RejectReason::UNDEFINED_ENUMERATION.to_raw(),
            })
        }
    };
    if value > maximum {
        return Err(Error::Reject {
            reason: bacnet_types::enums::RejectReason::UNDEFINED_ENUMERATION.to_raw(),
        });
    }
    Ok(value)
}

// ---------------------------------------------------------------------------
// GetEnrollmentSummaryAck
// ---------------------------------------------------------------------------

/// One entry in the GetEnrollmentSummary-ACK sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentSummaryEntry {
    pub object_identifier: ObjectIdentifier,
    pub event_type: EventType,
    pub event_state: EventState,
    pub priority: u8,
    /// Optional notification-class member.
    pub notification_class: Option<u32>,
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
            if let Some(notification_class) = entry.notification_class {
                primitives::encode_app_unsigned(buf, notification_class as u64);
            }
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
            if !is_application_tag(&tag, data[offset], tags::app_tag::OBJECT_IDENTIFIER) {
                return Err(Error::decoding(
                    offset,
                    "EnrollmentSummaryAck expected object-id application tag",
                ));
            }
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
            if !is_application_tag(&tag, data[offset], tags::app_tag::ENUMERATED) {
                return Err(Error::decoding(
                    offset,
                    "EnrollmentSummaryAck expected eventType enumerated tag",
                ));
            }
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "EnrollmentSummaryAck truncated at eventType",
                ));
            }
            let event_type = primitives::decode_unsigned(&data[pos..end])?;
            let event_type = u32::try_from(event_type)
                .map(EventType::from_raw)
                .map_err(|_| Error::decoding(pos, "EnrollmentSummaryAck eventType exceeds u32"))?;
            offset = end;

            // eventState (app enumerated)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if !is_application_tag(&tag, data[offset], tags::app_tag::ENUMERATED) {
                return Err(Error::decoding(
                    offset,
                    "EnrollmentSummaryAck expected eventState enumerated tag",
                ));
            }
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "EnrollmentSummaryAck truncated at eventState",
                ));
            }
            let event_state = primitives::decode_unsigned(&data[pos..end])?;
            let event_state = u32::try_from(event_state)
                .map(EventState::from_raw)
                .map_err(|_| Error::decoding(pos, "EnrollmentSummaryAck eventState exceeds u32"))?;
            offset = end;

            // priority (app unsigned)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if !is_application_tag(&tag, data[offset], tags::app_tag::UNSIGNED) {
                return Err(Error::decoding(
                    offset,
                    "EnrollmentSummaryAck expected priority unsigned tag",
                ));
            }
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "EnrollmentSummaryAck truncated at priority",
                ));
            }
            let priority = primitives::decode_unsigned(&data[pos..end])?;
            let priority = u8::try_from(priority)
                .map_err(|_| Error::decoding(pos, "EnrollmentSummaryAck priority exceeds u8"))?;
            offset = end;

            // notificationClass (app unsigned, optional)
            let mut notification_class = None;
            if offset < data.len() {
                let (tag, pos) = tags::decode_tag(data, offset)?;
                if is_application_tag(&tag, data[offset], tags::app_tag::UNSIGNED) {
                    let end = pos + tag.length as usize;
                    if end > data.len() {
                        return Err(Error::decoding(
                            pos,
                            "EnrollmentSummaryAck truncated at notificationClass",
                        ));
                    }
                    let value = primitives::decode_unsigned(&data[pos..end])?;
                    notification_class = Some(u32::try_from(value).map_err(|_| {
                        Error::decoding(pos, "EnrollmentSummaryAck notificationClass exceeds u32")
                    })?);
                    offset = end;
                }
            }

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
#[path = "enrollment_summary_width_tests.rs"]
mod width_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    #[test]
    fn request_round_trip() {
        let req = GetEnrollmentSummaryRequest {
            acknowledgment_filter: 0, // all
            enrollment_filter: None,
            event_state_filter: Some(EnrollmentSummaryEventStateFilter::OFFNORMAL),
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
            enrollment_filter: None,
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
                    notification_class: Some(10),
                },
                EnrollmentSummaryEntry {
                    object_identifier: ObjectIdentifier::new(ObjectType::BINARY_INPUT, 5).unwrap(),
                    event_type: EventType::CHANGE_OF_STATE,
                    event_state: EventState::NORMAL,
                    priority: 7,
                    notification_class: Some(20),
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
            enrollment_filter: None,
            event_state_filter: Some(EnrollmentSummaryEventStateFilter::FAULT),
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
                notification_class: Some(10),
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
                notification_class: Some(10),
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
