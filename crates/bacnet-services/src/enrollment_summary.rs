//! GetEnrollmentSummary service per ASHRAE 135-2020 Clause 13.11.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_encoding::constructed::tagged::{
    decode_app_enumerated, decode_app_object_id, decode_app_unsigned, decode_ctx_constructed,
    decode_ctx_primitive, decode_ctx_unsigned, decode_optional_ctx, expect_end,
    next_is_application, next_is_opening,
};
use bacnet_encoding::constructed::{check_encoded_mac_len, decode_recipient, encode_recipient};
use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::{
    AcknowledgmentFilter, EnrollmentSummaryEventStateFilter, EventState, EventType,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

// ---------------------------------------------------------------------------
// GetEnrollmentSummaryRequest
// ---------------------------------------------------------------------------

/// Priority filter sub-structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriorityFilter {
    /// Lowest event priority to include (0-255).
    pub min_priority: u8,
    /// Highest event priority to include (0-255).
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
    /// \[0\] acknowledgmentFilter: all enrollments, only acknowledged ones, or
    /// only those with an unacknowledged transition.
    pub acknowledgment_filter: AcknowledgmentFilter,
    /// \[1\] enrollmentFilter (optional) — BACnetRecipientProcess.
    pub enrollment_filter: Option<RecipientProcess>,
    /// \[2\] eventStateFilter (optional).
    pub event_state_filter: Option<EnrollmentSummaryEventStateFilter>,
    /// \[3\] eventTypeFilter (optional).
    pub event_type_filter: Option<EventType>,
    /// \[4\] priorityFilter { \[0\] minPriority, \[1\] maxPriority } (optional).
    pub priority_filter: Option<PriorityFilter>,
    /// \[5\] notificationClassFilter (optional).
    pub notification_class_filter: Option<u32>,
}

impl GetEnrollmentSummaryRequest {
    /// Encode this request.
    ///
    /// # Panics
    ///
    /// Panics if a filter contains a value outside its service-defined range,
    /// or an enrollment-filter address whose MAC is longer than
    /// `BACnetAddress::MAX_MAC_LEN` octets.
    pub fn encode(&self, buf: &mut BytesMut) {
        self.try_encode(buf)
            .expect("invalid GetEnrollmentSummary request");
    }

    /// Encode this request after validating representable filter invariants.
    pub fn try_encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        if self.acknowledgment_filter.to_raw() > AcknowledgmentFilter::NOT_ACKED.to_raw() {
            return Err(Error::encode("EnrollmentSummary acknowledgmentFilter is an undefined enumeration"));
        }
        if self.event_state_filter.is_some_and(|filter| {
            filter.to_raw() > EnrollmentSummaryEventStateFilter::ACTIVE.to_raw()
        }) {
            return Err(Error::encode("EnrollmentSummary eventStateFilter is an undefined enumeration"));
        }
        if self
            .priority_filter
            .is_some_and(|filter| filter.min_priority > filter.max_priority)
        {
            return Err(Error::encode("EnrollmentSummary priorityFilter minimum exceeds maximum"));
        }
        if let Some(RecipientProcess {
            recipient: BACnetRecipient::Address(address),
            ..
        }) = &self.enrollment_filter
        {
            check_encoded_mac_len(&address.mac_address, "EnrollmentSummary enrollmentFilter")?;
        }
        // [0] acknowledgmentFilter
        primitives::encode_ctx_enumerated(buf, 0, self.acknowledgment_filter.to_raw());
        // [1] enrollmentFilter (optional, constructed)
        if let Some(ref ef) = self.enrollment_filter {
            tags::encode_opening_tag(buf, 1);
            tags::encode_opening_tag(buf, 0);
            encode_recipient(buf, &ef.recipient)?;
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

    /// Decode the request from `data`; errors on malformed or truncated fields.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        // [0] acknowledgmentFilter
        let (content, mut offset) =
            decode_ctx_primitive(data, 0, 0, "EnrollmentSummary acknowledgmentFilter")?;
        let acknowledgment_filter = AcknowledgmentFilter::from_raw(decode_closed_enumeration(
            content,
            AcknowledgmentFilter::NOT_ACKED.to_raw(),
        )?);

        // [1] enrollmentFilter (optional, constructed): a [0] recipient
        // frame, then a [1] process identifier
        let mut enrollment_filter = None;
        if next_is_opening(data, offset, 1)? {
            let what = "EnrollmentSummary enrollmentFilter";
            let (content, new_offset) = decode_ctx_constructed(data, offset, 1, what)?;
            let (recipient_content, recipient_end) =
                decode_ctx_constructed(content, 0, 0, "EnrollmentSummary recipient")?;
            let (recipient, recipient_choice_end) = decode_recipient(recipient_content, 0)?;
            expect_end(
                recipient_content,
                recipient_choice_end,
                offset,
                "EnrollmentSummary recipient",
            )?;
            let (process_identifier, process_end) = decode_ctx_unsigned::<u32>(
                content,
                recipient_end,
                1,
                "EnrollmentSummary processIdentifier",
            )?;
            expect_end(content, process_end, offset, what)?;
            enrollment_filter = Some(RecipientProcess {
                recipient,
                process_identifier,
            });
            offset = new_offset;
        }

        // [2] eventStateFilter (optional)
        let (event_state, new_offset) = decode_optional_ctx(
            data,
            offset,
            2,
            "EnrollmentSummary eventStateFilter",
            decode_ctx_primitive,
        )?;
        let event_state_filter = match event_state {
            Some(content) => Some(EnrollmentSummaryEventStateFilter::from_raw(
                decode_closed_enumeration(
                    content,
                    EnrollmentSummaryEventStateFilter::ACTIVE.to_raw(),
                )?,
            )),
            None => None,
        };
        offset = new_offset;

        // [3] eventTypeFilter (optional)
        let (event_type, new_offset) = decode_optional_ctx(
            data,
            offset,
            3,
            "EnrollmentSummary eventTypeFilter",
            decode_ctx_unsigned::<u32>,
        )?;
        let event_type_filter = event_type.map(EventType::from_raw);
        offset = new_offset;

        // [4] priorityFilter (optional, constructed): [0] minPriority, then
        // [1] maxPriority
        let mut priority_filter = None;
        if next_is_opening(data, offset, 4)? {
            let what = "EnrollmentSummary priorityFilter";
            let (content, new_offset) = decode_ctx_constructed(data, offset, 4, what)?;
            let (min_priority, end) =
                decode_ctx_unsigned::<u8>(content, 0, 0, "EnrollmentSummary minPriority")?;
            let (max_priority, end) =
                decode_ctx_unsigned::<u8>(content, end, 1, "EnrollmentSummary maxPriority")?;
            expect_end(content, end, offset, what)?;
            if min_priority > max_priority {
                return Err(Error::out_of_range(
                    offset,
                    "EnrollmentSummary priorityFilter minimum exceeds maximum",
                ));
            }
            priority_filter = Some(PriorityFilter {
                min_priority,
                max_priority,
            });
            offset = new_offset;
        }

        // [5] notificationClassFilter (optional)
        let (notification_class_filter, new_offset) = decode_optional_ctx(
            data,
            offset,
            5,
            "EnrollmentSummary notificationClassFilter",
            decode_ctx_unsigned::<u32>,
        )?;
        offset = new_offset;
        expect_end(data, offset, offset, "EnrollmentSummary")?;

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
            return Err(Error::decoding(
                0,
                "EnrollmentSummary eventStateFilter is not a canonical ENUMERATED",
            ))
        }
        _ => {
            return Err(Error::reject(bacnet_types::enums::RejectReason::UNDEFINED_ENUMERATION))
        }
    };
    if value > maximum {
        return Err(Error::reject(bacnet_types::enums::RejectReason::UNDEFINED_ENUMERATION));
    }
    Ok(value)
}

// ---------------------------------------------------------------------------
// GetEnrollmentSummaryAck
// ---------------------------------------------------------------------------

/// One entry in the GetEnrollmentSummary-ACK sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentSummaryEntry {
    /// Event-initiating object being summarised.
    pub object_identifier: ObjectIdentifier,
    /// Kind of event algorithm the object uses.
    pub event_type: EventType,
    /// Event state the object currently holds.
    pub event_state: EventState,
    /// Priority of the object's event notifications (0-255).
    pub priority: u8,
    /// Optional notification-class member.
    pub notification_class: Option<u32>,
}

/// GetEnrollmentSummary-ACK: a sequence of summary entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetEnrollmentSummaryAck {
    /// Summary entries for matching objects.
    pub entries: Vec<EnrollmentSummaryEntry>,
}

impl GetEnrollmentSummaryAck {
    /// Append the ASN.1 encoding of the ACK to `buf`.
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

    /// Decode the ACK from `data`; errors on malformed input.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut entries = Vec::new();
        let mut offset = 0;

        while offset < data.len() {
            if entries.len() >= MAX_DECODED_ITEMS {
                return Err(Error::overflow(
                    offset,
                    "EnrollmentSummaryAck too many entries",
                ));
            }

            // objectIdentifier, eventType, eventState, priority and an
            // optional notificationClass (all application-tagged)
            let (object_identifier, end) =
                decode_app_object_id(data, offset, "EnrollmentSummaryAck object-id")?;
            let (event_type, end) =
                decode_app_enumerated::<u32>(data, end, "EnrollmentSummaryAck eventType")?;
            let event_type = EventType::from_raw(event_type);
            let (event_state, end) =
                decode_app_enumerated::<u32>(data, end, "EnrollmentSummaryAck eventState")?;
            let event_state = EventState::from_raw(event_state);
            let (priority, end) =
                decode_app_unsigned::<u8>(data, end, "EnrollmentSummaryAck priority")?;
            offset = end;
            let mut notification_class = None;
            if next_is_application(data, offset, tags::app_tag::UNSIGNED)? {
                let (value, end) = decode_app_unsigned::<u32>(
                    data,
                    offset,
                    "EnrollmentSummaryAck notificationClass",
                )?;
                notification_class = Some(value);
                offset = end;
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
            acknowledgment_filter: AcknowledgmentFilter::ALL,
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
            acknowledgment_filter: AcknowledgmentFilter::NOT_ACKED,
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
            acknowledgment_filter: AcknowledgmentFilter::ALL,
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
