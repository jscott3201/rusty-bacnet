//! COV (Change of Value) services per ASHRAE 135-2020 Clause 13 & 16.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::{BACnetPropertyValue, MAX_DECODED_ITEMS};

// ---------------------------------------------------------------------------
// SubscribeCOVRequest
// ---------------------------------------------------------------------------

/// SubscribeCOV-Request service parameters.
///
/// Both `issue_confirmed_notifications` and `lifetime` absent = cancellation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribeCOVRequest {
    pub subscriber_process_identifier: u32,
    pub monitored_object_identifier: ObjectIdentifier,
    pub issue_confirmed_notifications: Option<bool>,
    pub lifetime: Option<u32>,
}

impl SubscribeCOVRequest {
    /// Whether this is a cancellation (both optional fields absent).
    pub fn is_cancellation(&self) -> bool {
        self.issue_confirmed_notifications.is_none() && self.lifetime.is_none()
    }

    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] subscriber-process-identifier
        primitives::encode_ctx_unsigned(buf, 0, self.subscriber_process_identifier as u64);
        // [1] monitored-object-identifier
        primitives::encode_ctx_object_id(buf, 1, &self.monitored_object_identifier);
        // [2] issue-confirmed-notifications (optional)
        if let Some(confirmed) = self.issue_confirmed_notifications {
            primitives::encode_ctx_boolean(buf, 2, confirmed);
        }
        // [3] lifetime (optional)
        if let Some(lifetime) = self.lifetime {
            primitives::encode_ctx_unsigned(buf, 3, lifetime as u64);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] subscriber-process-identifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "SubscribeCOV truncated at process-id"));
        }
        let subscriber_process_identifier = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [1] monitored-object-identifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "SubscribeCOV truncated at object-id"));
        }
        let monitored_object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [2] issue-confirmed-notifications (optional)
        let mut issue_confirmed_notifications = None;
        if offset < data.len() {
            let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 2)?;
            if let Some(content) = opt_data {
                if content.is_empty() {
                    return Err(Error::decoding(
                        offset,
                        "SubscribeCOV: empty confirmed-notifications field",
                    ));
                }
                issue_confirmed_notifications = Some(content[0] != 0);
                offset = new_offset;
            }
        }

        // [3] lifetime (optional)
        let mut lifetime = None;
        if offset < data.len() {
            let (opt_data, _new_offset) = tags::decode_optional_context(data, offset, 3)?;
            if let Some(content) = opt_data {
                lifetime = Some(primitives::decode_unsigned(content)? as u32);
            }
        }

        Ok(Self {
            subscriber_process_identifier,
            monitored_object_identifier,
            issue_confirmed_notifications,
            lifetime,
        })
    }
}

// ---------------------------------------------------------------------------
// SubscribeCOVPropertyRequest
// ---------------------------------------------------------------------------

/// SubscribeCOVProperty-Request service parameters.
///
/// Like SubscribeCOV but targets a specific property and optionally
/// overrides the COV increment.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscribeCOVPropertyRequest {
    pub subscriber_process_identifier: u32,
    pub monitored_object_identifier: ObjectIdentifier,
    pub issue_confirmed_notifications: Option<bool>,
    pub lifetime: Option<u32>,
    pub monitored_property_identifier: PropertyIdentifier,
    pub monitored_property_array_index: Option<u32>,
    pub cov_increment: Option<f32>,
}

impl SubscribeCOVPropertyRequest {
    /// Whether this is a cancellation (both optional notification/lifetime absent).
    pub fn is_cancellation(&self) -> bool {
        self.issue_confirmed_notifications.is_none() && self.lifetime.is_none()
    }

    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] subscriberProcessIdentifier
        primitives::encode_ctx_unsigned(buf, 0, self.subscriber_process_identifier as u64);
        // [1] monitoredObjectIdentifier
        primitives::encode_ctx_object_id(buf, 1, &self.monitored_object_identifier);
        // [2] issueConfirmedNotifications (optional)
        if let Some(v) = self.issue_confirmed_notifications {
            primitives::encode_ctx_boolean(buf, 2, v);
        }
        // [3] lifetime (optional)
        if let Some(v) = self.lifetime {
            primitives::encode_ctx_unsigned(buf, 3, v as u64);
        }
        // [4] monitoredPropertyIdentifier (BACnetPropertyReference)
        tags::encode_opening_tag(buf, 4);
        primitives::encode_ctx_unsigned(buf, 0, self.monitored_property_identifier.to_raw() as u64);
        if let Some(idx) = self.monitored_property_array_index {
            primitives::encode_ctx_unsigned(buf, 1, idx as u64);
        }
        tags::encode_closing_tag(buf, 4);
        // [5] covIncrement (optional)
        if let Some(v) = self.cov_increment {
            primitives::encode_ctx_real(buf, 5, v);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] subscriberProcessIdentifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if tag.number != 0 {
            return Err(Error::decoding(pos, "expected context 0 for process-id"));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::buffer_too_short(end, data.len()));
        }
        let subscriber_process_identifier = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [1] monitoredObjectIdentifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if tag.number != 1 {
            return Err(Error::decoding(pos, "expected context 1 for object-id"));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::buffer_too_short(end, data.len()));
        }
        let monitored_object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [2] issueConfirmedNotifications (optional)
        let mut issue_confirmed_notifications = None;
        if offset < data.len() {
            let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 2)?;
            if let Some(content) = opt_data {
                issue_confirmed_notifications = Some(!content.is_empty() && content[0] != 0);
                offset = new_offset;
            }
        }

        // [3] lifetime (optional)
        let mut lifetime = None;
        if offset < data.len() {
            let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 3)?;
            if let Some(content) = opt_data {
                lifetime = Some(primitives::decode_unsigned(content)? as u32);
                offset = new_offset;
            }
        }

        // [4] monitoredPropertyIdentifier (BACnetPropertyReference)
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if tag.number != 4 {
            return Err(Error::decoding(
                pos,
                "expected context 4 for monitored-property",
            ));
        }
        offset = pos;
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::buffer_too_short(end, data.len()));
        }
        let monitored_property_identifier =
            PropertyIdentifier::from_raw(primitives::decode_unsigned(&data[pos..end])? as u32);
        offset = end;

        // [1] propertyArrayIndex (optional)
        let mut monitored_property_array_index = None;
        if offset < data.len() {
            let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 1)?;
            if let Some(content) = opt_data {
                monitored_property_array_index = Some(primitives::decode_unsigned(content)? as u32);
                offset = new_offset;
            }
        }

        let (_tag, pos) = tags::decode_tag(data, offset)?;
        offset = pos;

        // [5] covIncrement (optional)
        let mut cov_increment = None;
        if offset < data.len() {
            let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 5)?;
            if let Some(content) = opt_data {
                cov_increment = Some(primitives::decode_real(content)?);
                offset = new_offset;
            }
        }
        let _ = offset; // suppress unused

        Ok(Self {
            subscriber_process_identifier,
            monitored_object_identifier,
            issue_confirmed_notifications,
            lifetime,
            monitored_property_identifier,
            monitored_property_array_index,
            cov_increment,
        })
    }
}

// ---------------------------------------------------------------------------
// COVNotificationRequest
// ---------------------------------------------------------------------------

/// COVNotification-Request service parameters.
///
/// Used for both ConfirmedCOVNotification and UnconfirmedCOVNotification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct COVNotificationRequest {
    pub subscriber_process_identifier: u32,
    pub initiating_device_identifier: ObjectIdentifier,
    pub monitored_object_identifier: ObjectIdentifier,
    pub time_remaining: u32,
    pub list_of_values: Vec<BACnetPropertyValue>,
}

impl COVNotificationRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] subscriber-process-identifier
        primitives::encode_ctx_unsigned(buf, 0, self.subscriber_process_identifier as u64);
        // [1] initiating-device-identifier
        primitives::encode_ctx_object_id(buf, 1, &self.initiating_device_identifier);
        // [2] monitored-object-identifier
        primitives::encode_ctx_object_id(buf, 2, &self.monitored_object_identifier);
        // [3] time-remaining
        primitives::encode_ctx_unsigned(buf, 3, self.time_remaining as u64);
        // [4] list-of-values (opening/closing)
        tags::encode_opening_tag(buf, 4);
        for pv in &self.list_of_values {
            pv.encode(buf);
        }
        tags::encode_closing_tag(buf, 4);
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] subscriber-process-identifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "COVNotification truncated at process-id",
            ));
        }
        let subscriber_process_identifier = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [1] initiating-device-identifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "COVNotification truncated at device-id",
            ));
        }
        let initiating_device_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [2] monitored-object-identifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "COVNotification truncated at monitored-id",
            ));
        }
        let monitored_object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [3] time-remaining
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "COVNotification truncated at time-remaining",
            ));
        }
        let time_remaining = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [4] list-of-values (opening tag 4)
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(4) {
            return Err(Error::decoding(
                offset,
                "COVNotification expected opening tag 4",
            ));
        }
        offset = tag_end;

        let mut values = Vec::new();
        loop {
            if offset >= data.len() {
                return Err(Error::decoding(
                    offset,
                    "COVNotification missing closing tag 4",
                ));
            }
            if values.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(
                    offset,
                    "COVNotification values exceeds max",
                ));
            }
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_closing_tag(4) {
                offset = tag_end;
                break;
            }
            let (pv, new_offset) = BACnetPropertyValue::decode(data, offset)?;
            values.push(pv);
            offset = new_offset;
        }
        let _ = offset;

        Ok(Self {
            subscriber_process_identifier,
            initiating_device_identifier,
            monitored_object_identifier,
            time_remaining,
            list_of_values: values,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};

    #[test]
    fn subscribe_cov_round_trip() {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            issue_confirmed_notifications: Some(true),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = SubscribeCOVRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
        assert!(!decoded.is_cancellation());
    }

    #[test]
    fn subscribe_cov_cancellation() {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            issue_confirmed_notifications: None,
            lifetime: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = SubscribeCOVRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
        assert!(decoded.is_cancellation());
    }

    #[test]
    fn cov_notification_round_trip() {
        let req = COVNotificationRequest {
            subscriber_process_identifier: 1,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            time_remaining: 60,
            list_of_values: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::STATUS_FLAGS,
                    property_array_index: None,
                    value: vec![0x82, 0x04, 0x00], // bit-string: 4 unused, 0x00
                    priority: None,
                },
            ],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = COVNotificationRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_subscribe_cov_empty_input() {
        assert!(SubscribeCOVRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_subscribe_cov_truncated_1_byte() {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            issue_confirmed_notifications: Some(true),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(SubscribeCOVRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_subscribe_cov_truncated_2_bytes() {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            issue_confirmed_notifications: Some(true),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(SubscribeCOVRequest::decode(&buf[..2]).is_err());
    }

    #[test]
    fn test_decode_subscribe_cov_truncated_3_bytes() {
        let req = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            issue_confirmed_notifications: Some(true),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(SubscribeCOVRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_subscribe_cov_invalid_tag() {
        assert!(SubscribeCOVRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_cov_notification_empty_input() {
        assert!(COVNotificationRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_cov_notification_truncated_1_byte() {
        let req = COVNotificationRequest {
            subscriber_process_identifier: 1,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            time_remaining: 60,
            list_of_values: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                priority: None,
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(COVNotificationRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_cov_notification_truncated_3_bytes() {
        let req = COVNotificationRequest {
            subscriber_process_identifier: 1,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            time_remaining: 60,
            list_of_values: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                priority: None,
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(COVNotificationRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_cov_notification_truncated_half() {
        let req = COVNotificationRequest {
            subscriber_process_identifier: 1,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            time_remaining: 60,
            list_of_values: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                priority: None,
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(COVNotificationRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_cov_notification_invalid_tag() {
        assert!(COVNotificationRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn subscribe_cov_property_round_trip() {
        let req = SubscribeCOVPropertyRequest {
            subscriber_process_identifier: 7,
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3)
                .unwrap(),
            issue_confirmed_notifications: Some(true),
            lifetime: Some(600),
            monitored_property_identifier: PropertyIdentifier::PRESENT_VALUE,
            monitored_property_array_index: None,
            cov_increment: Some(1.5),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = SubscribeCOVPropertyRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn subscribe_cov_property_round_trip_with_array_index() {
        let req = SubscribeCOVPropertyRequest {
            subscriber_process_identifier: 2,
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::BINARY_VALUE, 10)
                .unwrap(),
            issue_confirmed_notifications: None,
            lifetime: None,
            monitored_property_identifier: PropertyIdentifier::PRESENT_VALUE,
            monitored_property_array_index: Some(3),
            cov_increment: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = SubscribeCOVPropertyRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
        assert!(decoded.is_cancellation());
    }
}
