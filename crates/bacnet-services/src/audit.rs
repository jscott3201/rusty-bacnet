//! Audit services per ASHRAE 135-2020 Clauses 15.2.8 / 15.2.9.
//!
//! Complex nested types (BACnetRecipient, BACnetPropertyReference inside
//! AuditNotification, query options in AuditLogQuery) are stored as raw
//! bytes to keep the codec minimal.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier};
use bytes::BytesMut;

use crate::common::PropertyReference;

// ---------------------------------------------------------------------------
// AuditNotificationRequest
// ---------------------------------------------------------------------------

/// AuditNotification-Request service parameters.
///
/// BACnetRecipient fields (sourceDevice, targetDevice) are stored as raw bytes
/// between opening/closing tags since BACnetRecipient encode/decode is not
/// available in the encoding crate.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditNotificationRequest {
    /// [0] sourceTimestamp
    pub source_timestamp: BACnetTimeStamp,
    /// [1] targetTimestamp OPTIONAL
    pub target_timestamp: Option<BACnetTimeStamp>,
    /// [2] sourceDevice — raw BACnetRecipient bytes
    pub source_device: Vec<u8>,
    /// [3] sourceObject OPTIONAL
    pub source_object: Option<ObjectIdentifier>,
    /// [4] operation
    pub operation: u32,
    /// [5] sourceComment OPTIONAL
    pub source_comment: Option<String>,
    /// [6] targetComment OPTIONAL
    pub target_comment: Option<String>,
    /// [7] invokeId OPTIONAL
    pub invoke_id: Option<u8>,
    /// [8] sourceUserInfo OPTIONAL — raw bytes
    pub source_user_info: Option<Vec<u8>>,
    /// [9] targetDevice — raw BACnetRecipient bytes
    pub target_device: Vec<u8>,
    /// [10] targetObject OPTIONAL
    pub target_object: Option<ObjectIdentifier>,
    /// [11] targetProperty OPTIONAL
    pub target_property: Option<PropertyReference>,
    /// [12] targetPriority OPTIONAL
    pub target_priority: Option<u8>,
    /// [13] targetValue OPTIONAL — raw bytes
    pub target_value: Option<Vec<u8>>,
    /// [14] currentValue OPTIONAL — raw bytes
    pub current_value: Option<Vec<u8>>,
    /// [15] result OPTIONAL — raw error bytes
    pub result: Option<Vec<u8>>,
}

impl AuditNotificationRequest {
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] sourceTimestamp
        primitives::encode_timestamp(buf, 0, &self.source_timestamp);
        // [1] targetTimestamp OPTIONAL
        if let Some(ref ts) = self.target_timestamp {
            primitives::encode_timestamp(buf, 1, ts);
        }
        // [2] sourceDevice (raw BACnetRecipient)
        tags::encode_opening_tag(buf, 2);
        buf.extend_from_slice(&self.source_device);
        tags::encode_closing_tag(buf, 2);
        // [3] sourceObject OPTIONAL
        if let Some(ref oid) = self.source_object {
            primitives::encode_ctx_object_id(buf, 3, oid);
        }
        // [4] operation
        primitives::encode_ctx_enumerated(buf, 4, self.operation);
        // [5] sourceComment OPTIONAL
        if let Some(ref s) = self.source_comment {
            primitives::encode_ctx_character_string(buf, 5, s)?;
        }
        // [6] targetComment OPTIONAL
        if let Some(ref s) = self.target_comment {
            primitives::encode_ctx_character_string(buf, 6, s)?;
        }
        // [7] invokeId OPTIONAL
        if let Some(id) = self.invoke_id {
            primitives::encode_ctx_unsigned(buf, 7, id as u64);
        }
        // [8] sourceUserInfo OPTIONAL (raw)
        if let Some(ref raw) = self.source_user_info {
            tags::encode_opening_tag(buf, 8);
            buf.extend_from_slice(raw);
            tags::encode_closing_tag(buf, 8);
        }
        // [9] targetDevice (raw BACnetRecipient)
        tags::encode_opening_tag(buf, 9);
        buf.extend_from_slice(&self.target_device);
        tags::encode_closing_tag(buf, 9);
        // [10] targetObject OPTIONAL
        if let Some(ref oid) = self.target_object {
            primitives::encode_ctx_object_id(buf, 10, oid);
        }
        // [11] targetProperty OPTIONAL
        if let Some(ref pr) = self.target_property {
            tags::encode_opening_tag(buf, 11);
            pr.encode(buf);
            tags::encode_closing_tag(buf, 11);
        }
        // [12] targetPriority OPTIONAL
        if let Some(prio) = self.target_priority {
            primitives::encode_ctx_unsigned(buf, 12, prio as u64);
        }
        // [13] targetValue OPTIONAL (raw)
        if let Some(ref raw) = self.target_value {
            tags::encode_opening_tag(buf, 13);
            buf.extend_from_slice(raw);
            tags::encode_closing_tag(buf, 13);
        }
        // [14] currentValue OPTIONAL (raw)
        if let Some(ref raw) = self.current_value {
            tags::encode_opening_tag(buf, 14);
            buf.extend_from_slice(raw);
            tags::encode_closing_tag(buf, 14);
        }
        // [15] result OPTIONAL (raw)
        if let Some(ref raw) = self.result {
            tags::encode_opening_tag(buf, 15);
            buf.extend_from_slice(raw);
            tags::encode_closing_tag(buf, 15);
        }
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] sourceTimestamp
        let (source_timestamp, new_off) = primitives::decode_timestamp(data, offset, 0)?;
        offset = new_off;

        // [1] targetTimestamp OPTIONAL
        let mut target_timestamp = None;
        if offset < data.len() {
            let (peek, _) = tags::decode_tag(data, offset)?;
            if peek.is_opening_tag(1) {
                let (ts, new_off) = primitives::decode_timestamp(data, offset, 1)?;
                target_timestamp = Some(ts);
                offset = new_off;
            }
        }

        // [2] sourceDevice (raw)
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(2) {
            return Err(Error::decoding(
                offset,
                "AuditNotification expected opening tag 2 for source-device",
            ));
        }
        let (raw, new_off) = tags::extract_context_value(data, tag_end, 2)?;
        let source_device = raw.to_vec();
        offset = new_off;

        // [3] sourceObject OPTIONAL
        let mut source_object = None;
        if offset < data.len() {
            let (opt, new_off) = tags::decode_optional_context(data, offset, 3)?;
            if let Some(content) = opt {
                source_object = Some(ObjectIdentifier::decode(content)?);
                offset = new_off;
            }
        }

        // [4] operation
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "AuditNotification truncated at operation",
            ));
        }
        let operation = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [5] sourceComment OPTIONAL
        let mut source_comment = None;
        if offset < data.len() {
            let (opt, new_off) = tags::decode_optional_context(data, offset, 5)?;
            if let Some(content) = opt {
                source_comment = Some(primitives::decode_character_string(content)?);
                offset = new_off;
            }
        }

        // [6] targetComment OPTIONAL
        let mut target_comment = None;
        if offset < data.len() {
            let (opt, new_off) = tags::decode_optional_context(data, offset, 6)?;
            if let Some(content) = opt {
                target_comment = Some(primitives::decode_character_string(content)?);
                offset = new_off;
            }
        }

        // [7] invokeId OPTIONAL
        let mut invoke_id = None;
        if offset < data.len() {
            let (opt, new_off) = tags::decode_optional_context(data, offset, 7)?;
            if let Some(content) = opt {
                invoke_id = Some(primitives::decode_unsigned(content)? as u8);
                offset = new_off;
            }
        }

        // [8] sourceUserInfo OPTIONAL (raw)
        let mut source_user_info = None;
        if offset < data.len() {
            let (peek, _) = tags::decode_tag(data, offset)?;
            if peek.is_opening_tag(8) {
                let (_, inner_start) = tags::decode_tag(data, offset)?;
                let (raw, new_off) = tags::extract_context_value(data, inner_start, 8)?;
                source_user_info = Some(raw.to_vec());
                offset = new_off;
            }
        }

        // [9] targetDevice (raw)
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(9) {
            return Err(Error::decoding(
                offset,
                "AuditNotification expected opening tag 9 for target-device",
            ));
        }
        let (raw, new_off) = tags::extract_context_value(data, tag_end, 9)?;
        let target_device = raw.to_vec();
        offset = new_off;

        // [10] targetObject OPTIONAL
        let mut target_object = None;
        if offset < data.len() {
            let (opt, new_off) = tags::decode_optional_context(data, offset, 10)?;
            if let Some(content) = opt {
                target_object = Some(ObjectIdentifier::decode(content)?);
                offset = new_off;
            }
        }

        // [11] targetProperty OPTIONAL
        let mut target_property = None;
        if offset < data.len() {
            let (peek, _) = tags::decode_tag(data, offset)?;
            if peek.is_opening_tag(11) {
                let (_, inner_start) = tags::decode_tag(data, offset)?;
                let (pr, pr_end) = PropertyReference::decode(data, inner_start)?;
                target_property = Some(pr);
                let (_tag, tag_end) = tags::decode_tag(data, pr_end)?;
                offset = tag_end;
            }
        }

        // [12] targetPriority OPTIONAL
        let mut target_priority = None;
        if offset < data.len() {
            let (opt, new_off) = tags::decode_optional_context(data, offset, 12)?;
            if let Some(content) = opt {
                target_priority = Some(primitives::decode_unsigned(content)? as u8);
                offset = new_off;
            }
        }

        // [13] targetValue OPTIONAL (raw)
        let mut target_value = None;
        if offset < data.len() {
            let (peek, _) = tags::decode_tag(data, offset)?;
            if peek.is_opening_tag(13) {
                let (_, inner_start) = tags::decode_tag(data, offset)?;
                let (raw, new_off) = tags::extract_context_value(data, inner_start, 13)?;
                target_value = Some(raw.to_vec());
                offset = new_off;
            }
        }

        // [14] currentValue OPTIONAL (raw)
        let mut current_value = None;
        if offset < data.len() {
            let (peek, _) = tags::decode_tag(data, offset)?;
            if peek.is_opening_tag(14) {
                let (_, inner_start) = tags::decode_tag(data, offset)?;
                let (raw, new_off) = tags::extract_context_value(data, inner_start, 14)?;
                current_value = Some(raw.to_vec());
                offset = new_off;
            }
        }

        // [15] result OPTIONAL (raw)
        let mut result = None;
        if offset < data.len() {
            let (peek, _) = tags::decode_tag(data, offset)?;
            if peek.is_opening_tag(15) {
                let (_, inner_start) = tags::decode_tag(data, offset)?;
                let (raw, new_off) = tags::extract_context_value(data, inner_start, 15)?;
                result = Some(raw.to_vec());
                offset = new_off;
            }
        }
        let _ = offset;

        Ok(Self {
            source_timestamp,
            target_timestamp,
            source_device,
            source_object,
            operation,
            source_comment,
            target_comment,
            invoke_id,
            source_user_info,
            target_device,
            target_object,
            target_property,
            target_priority,
            target_value,
            current_value,
            result,
        })
    }
}

// ---------------------------------------------------------------------------
// AuditLogQueryRequest
// ---------------------------------------------------------------------------

/// AuditLogQuery-Request storing query options as raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditLogQueryRequest {
    /// [0] acknowledgmentFilter
    pub acknowledgment_filter: u32,
    /// Remaining query options as raw bytes (context tags 1+).
    pub query_options_raw: Vec<u8>,
}

impl AuditLogQueryRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] acknowledgmentFilter
        primitives::encode_ctx_enumerated(buf, 0, self.acknowledgment_filter);
        buf.extend_from_slice(&self.query_options_raw);
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] acknowledgmentFilter
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "AuditLogQuery truncated at acknowledgment-filter",
            ));
        }
        let acknowledgment_filter = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        let query_options_raw = data[offset..].to_vec();

        Ok(Self {
            acknowledgment_filter,
            query_options_raw,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};
    use bacnet_types::primitives::Time;

    #[test]
    fn audit_notification_round_trip() {
        let req = AuditNotificationRequest {
            source_timestamp: BACnetTimeStamp::SequenceNumber(100),
            target_timestamp: None,
            source_device: vec![0x09, 0x01], // raw recipient
            source_object: Some(ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap()),
            operation: 3,
            source_comment: Some("test audit".to_string()),
            target_comment: None,
            invoke_id: Some(5),
            source_user_info: None,
            target_device: vec![0x09, 0x02], // raw recipient
            target_object: Some(ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap()),
            target_property: Some(PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            }),
            target_priority: Some(8),
            target_value: Some(vec![0x44, 0x42, 0x90, 0x00, 0x00]),
            current_value: Some(vec![0x44, 0x00, 0x00, 0x00, 0x00]),
            result: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = AuditNotificationRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn audit_notification_minimal() {
        let req = AuditNotificationRequest {
            source_timestamp: BACnetTimeStamp::Time(Time {
                hour: 10,
                minute: 0,
                second: 0,
                hundredths: 0,
            }),
            target_timestamp: None,
            source_device: vec![0x09, 0x01],
            source_object: None,
            operation: 0,
            source_comment: None,
            target_comment: None,
            invoke_id: None,
            source_user_info: None,
            target_device: vec![0x09, 0x02],
            target_object: None,
            target_property: None,
            target_priority: None,
            target_value: None,
            current_value: None,
            result: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = AuditNotificationRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn audit_notification_empty_input() {
        assert!(AuditNotificationRequest::decode(&[]).is_err());
    }

    #[test]
    fn audit_log_query_round_trip() {
        let req = AuditLogQueryRequest {
            acknowledgment_filter: 1,
            query_options_raw: vec![0x19, 0x05, 0x29, 0x0A],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = AuditLogQueryRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn audit_log_query_no_options() {
        let req = AuditLogQueryRequest {
            acknowledgment_filter: 0,
            query_options_raw: vec![],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = AuditLogQueryRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn audit_log_query_empty_input() {
        assert!(AuditLogQueryRequest::decode(&[]).is_err());
    }
}
