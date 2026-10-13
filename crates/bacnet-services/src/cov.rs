//! COV (Change of Value) services per ASHRAE 135-2020 Clauses 13.6, 13.7, 13.14 and 13.15.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_encoding::constructed::tagged::{
    decode_ctx_boolean, decode_ctx_object_id, decode_ctx_real, decode_ctx_unsigned, expect_end,
    misplaced_tag, next_is_context, unclosed_kind,
};
use bacnet_encoding::constructed::{decode_property_reference, encode_bacnet_property_value};
use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::BACnetPropertyValue;

pub use crate::cov_decode::COVNotificationDecodeError;

// ---------------------------------------------------------------------------
// SubscribeCOVRequest
// ---------------------------------------------------------------------------

/// SubscribeCOV-Request service parameters.
///
/// Both `issue_confirmed_notifications` and `lifetime` absent = cancellation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribeCOVRequest {
    /// Subscriber-chosen handle echoed in every notification, matching it to this subscription.
    pub subscriber_process_identifier: u32,
    /// Object whose changes are being subscribed to.
    pub monitored_object_identifier: ObjectIdentifier,
    /// `true` confirmed, `false` unconfirmed notifications; `None` (with no lifetime) cancels.
    pub issue_confirmed_notifications: Option<bool>,
    /// Subscription lifetime in seconds, where `Some(0)` means no expiry. `None` together with
    /// no confirmed flag cancels; `None` alone also means no expiry.
    pub lifetime: Option<u32>,
}

impl SubscribeCOVRequest {
    /// Whether this is a cancellation (both optional fields absent).
    pub fn is_cancellation(&self) -> bool {
        self.issue_confirmed_notifications.is_none() && self.lifetime.is_none()
    }

    /// Lifetime requires an explicit notification mode; mode alone is indefinite.
    pub fn validate(&self) -> Result<(), Error> {
        if self.lifetime.is_some() && self.issue_confirmed_notifications.is_none() {
            return Err(Error::Encoding(
                "SubscribeCOV lifetime requires confirmed-notification mode".into(),
            ));
        }
        Ok(())
    }

    /// Append a valid request, leaving the buffer unchanged on validation failure.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
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
        Ok(())
    }

    /// Decode the request from `data`; errors on missing, malformed or truncated fields.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] subscriber-process-identifier
        let (subscriber_process_identifier, end) =
            decode_ctx_unsigned::<u32>(data, offset, 0, "SubscribeCOV process-id")?;
        offset = end;

        // [1] monitored-object-identifier
        let (monitored_object_identifier, end) =
            decode_ctx_object_id(data, offset, 1, "SubscribeCOV object-id")?;
        offset = end;

        // [2] issue-confirmed-notifications (optional)
        let mut issue_confirmed_notifications = None;
        if next_is_context(data, offset, 2)? {
            let (value, end) =
                decode_ctx_boolean(data, offset, 2, "SubscribeCOV confirmed-notifications")?;
            issue_confirmed_notifications = Some(value);
            offset = end;
        }

        // [3] lifetime (optional)
        let mut lifetime = None;
        if next_is_context(data, offset, 3)? {
            let (value, end) =
                decode_ctx_unsigned::<u32>(data, offset, 3, "SubscribeCOV lifetime")?;
            lifetime = Some(value);
            offset = end;
        }
        expect_end(data, offset, offset, "SubscribeCOV")?;

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
    /// Subscriber-chosen handle echoed in every notification for this subscription.
    pub subscriber_process_identifier: u32,
    /// Object that holds the monitored property.
    pub monitored_object_identifier: ObjectIdentifier,
    /// `true` confirmed, `false` unconfirmed notifications; `None` (with no lifetime) cancels.
    pub issue_confirmed_notifications: Option<bool>,
    /// Subscription lifetime in seconds, where `Some(0)` means no expiry. `None` together with
    /// no confirmed flag cancels; `None` alone also means no expiry.
    pub lifetime: Option<u32>,
    /// Property whose changes are reported.
    pub monitored_property_identifier: PropertyIdentifier,
    /// Array element of the monitored property to watch; `None` means the whole property.
    pub monitored_property_array_index: Option<u32>,
    /// Minimum change that triggers a notification, overriding the object's own COV increment.
    pub cov_increment: Option<f32>,
}

impl SubscribeCOVPropertyRequest {
    /// Whether this is a cancellation (both optional notification/lifetime absent).
    pub fn is_cancellation(&self) -> bool {
        self.issue_confirmed_notifications.is_none() && self.lifetime.is_none()
    }

    /// Validate the subscribe/cancel field pairing before any outbound mutation.
    pub fn validate(&self) -> Result<(), Error> {
        match (self.issue_confirmed_notifications, self.lifetime) {
            (None, None) => Ok(()),
            (Some(_), Some(lifetime)) if lifetime != 0 => Ok(()),
            (Some(_), Some(0)) => Err(Error::Encoding(
                "SubscribeCOVProperty requires a positive lifetime".into(),
            )),
            _ => Err(Error::Encoding(
                "SubscribeCOVProperty confirmed mode and lifetime must appear together".into(),
            )),
        }
    }

    /// Encode a validated request, leaving the buffer unchanged on failure.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
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
        Ok(())
    }

    /// Decode the request from `data`; errors on missing, malformed or truncated fields.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] subscriberProcessIdentifier
        let (subscriber_process_identifier, end) =
            decode_ctx_unsigned::<u32>(data, offset, 0, "SubscribeCOVProperty process-id")?;
        offset = end;

        // [1] monitoredObjectIdentifier
        let (monitored_object_identifier, end) =
            decode_ctx_object_id(data, offset, 1, "SubscribeCOVProperty object-id")?;
        offset = end;

        // [2] issueConfirmedNotifications (optional)
        let mut issue_confirmed_notifications = None;
        if next_is_context(data, offset, 2)? {
            let (value, end) = decode_ctx_boolean(
                data,
                offset,
                2,
                "SubscribeCOVProperty confirmed-notifications",
            )?;
            issue_confirmed_notifications = Some(value);
            offset = end;
        }

        // [3] lifetime (optional)
        let mut lifetime = None;
        if next_is_context(data, offset, 3)? {
            let (value, end) =
                decode_ctx_unsigned::<u32>(data, offset, 3, "SubscribeCOVProperty lifetime")?;
            lifetime = Some(value);
            offset = end;
        }

        // [4] monitoredPropertyIdentifier (BACnetPropertyReference)
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(4) {
            return Err(misplaced_tag(
                data,
                &tag,
                Some(4),
                offset,
                "SubscribeCOVProperty expected opening tag 4",
            ));
        }
        let (monitored_property, end) = decode_property_reference(data, pos)?;
        offset = end;
        let (tag, end) = tags::decode_tag(data, offset)?;
        if !tag.is_closing_tag(4) {
            return Err(Error::decoding_kind(
                unclosed_kind(&tag),
                offset,
                "SubscribeCOVProperty expected closing tag 4",
            ));
        }
        offset = end;

        // [5] covIncrement (optional)
        let mut cov_increment = None;
        if next_is_context(data, offset, 5)? {
            let (increment, end) =
                decode_ctx_real(data, offset, 5, "SubscribeCOVProperty COV increment")?;
            cov_increment = Some(increment);
            offset = end;
        }
        expect_end(data, offset, offset, "SubscribeCOVProperty")?;

        Ok(Self {
            subscriber_process_identifier,
            monitored_object_identifier,
            issue_confirmed_notifications,
            lifetime,
            monitored_property_identifier: monitored_property.property_identifier,
            monitored_property_array_index: monitored_property.property_array_index,
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
    /// Subscriber-chosen handle from the subscription, used to route the notification.
    pub subscriber_process_identifier: u32,
    /// Device that sent the notification.
    pub initiating_device_identifier: ObjectIdentifier,
    /// Object whose value changed.
    pub monitored_object_identifier: ObjectIdentifier,
    /// Seconds left before the subscription expires; 0 means it never expires (Clause 13.6.1).
    pub time_remaining: u32,
    /// Property values the notification reports for the monitored object (Table 13-1 lists which
    /// ones), each as raw encoded BACnet bytes; not only the ones that changed.
    pub list_of_values: Vec<BACnetPropertyValue>,
}

impl COVNotificationRequest {
    /// Append the ASN.1 encoding of the notification to `buf`.
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
            encode_bacnet_property_value(pv, buf);
        }
        tags::encode_closing_tag(buf, 4);
    }

    /// Decode the notification from `data`; errors on missing, malformed or truncated fields.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        Self::decode_detailed(data).map_err(COVNotificationDecodeError::into_error)
    }
}

#[cfg(test)]
#[path = "cov_width_tests.rs"]
mod width_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::MAX_DECODED_ITEMS;
    use bacnet_types::enums::{ObjectType, PropertyIdentifier, RejectReason};

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
        req.encode(&mut buf).unwrap();
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
        req.encode(&mut buf).unwrap();
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
        req.encode(&mut buf).unwrap();
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
        req.encode(&mut buf).unwrap();
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
        req.encode(&mut buf).unwrap();
        assert!(SubscribeCOVRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_subscribe_cov_invalid_tag() {
        assert!(SubscribeCOVRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_cov_notification_empty_input() {
        assert!(COVNotificationRequest::decode(&[]).is_err());
        assert_eq!(
            COVNotificationRequest::decode_detailed(&[])
                .unwrap_err()
                .reject_reason(),
            RejectReason::MISSING_REQUIRED_PARAMETER
        );
    }

    #[test]
    fn test_decode_cov_notification_classifies_missing_and_invalid_tags() {
        // A valid [0] process identifier followed by EOF is still missing a
        // later mandatory service argument.
        assert_eq!(
            COVNotificationRequest::decode_detailed(&[0x09, 0x01])
                .unwrap_err()
                .reject_reason(),
            RejectReason::MISSING_REQUIRED_PARAMETER
        );
        // Context [7] where the [0] process identifier is due: the members
        // before it are missing (#1446).
        assert_eq!(
            COVNotificationRequest::decode_detailed(&[0x79, 0x01])
                .unwrap_err()
                .reject_reason(),
            RejectReason::MISSING_REQUIRED_PARAMETER
        );
        // An application tag cannot begin this service request.
        assert_eq!(
            COVNotificationRequest::decode_detailed(&[0x21, 0x01])
                .unwrap_err()
                .reject_reason(),
            RejectReason::INVALID_TAG
        );
        // A present ObjectIdentifier with an invalid wire width has invalid
        // data encoding rather than a missing argument.
        assert_eq!(
            COVNotificationRequest::decode_detailed(&[0x09, 0x01, 0x1B, 0x01, 0x02, 0x03])
                .unwrap_err()
                .reject_reason(),
            RejectReason::INVALID_DATA_ENCODING
        );
        // Unsigned values use the smallest possible encoding.
        assert_eq!(
            COVNotificationRequest::decode_detailed(&[0x0A, 0x00, 0x01])
                .unwrap_err()
                .reject_reason(),
            RejectReason::INVALID_DATA_ENCODING
        );
    }

    #[test]
    fn test_decode_cov_notification_rejects_empty_list_of_values() {
        let req = COVNotificationRequest {
            subscriber_process_identifier: 1,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            time_remaining: 60,
            list_of_values: Vec::new(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);

        assert_eq!(
            COVNotificationRequest::decode_detailed(&buf)
                .unwrap_err()
                .reject_reason(),
            RejectReason::PARAMETER_OUT_OF_RANGE
        );
    }

    #[test]
    fn test_decode_cov_notification_classifies_nested_invalid_tag() {
        let req = COVNotificationRequest {
            subscriber_process_identifier: 1,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            time_remaining: 60,
            list_of_values: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x21, 0x01],
                priority: None,
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let list_start = buf.iter().position(|byte| *byte == 0x4E).unwrap();
        // An application Unsigned where the value's [0] property identifier
        // is due.
        buf[list_start + 1] = 0x21;

        assert_eq!(
            COVNotificationRequest::decode_detailed(&buf)
                .unwrap_err()
                .reject_reason(),
            RejectReason::INVALID_TAG
        );
    }

    #[test]
    fn test_decode_cov_notification_enforces_value_cap() {
        let value = BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x21, 0x01],
            priority: None,
        };
        let mut req = COVNotificationRequest {
            subscriber_process_identifier: 1,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            time_remaining: 60,
            list_of_values: vec![value.clone(); MAX_DECODED_ITEMS],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert_eq!(
            COVNotificationRequest::decode_detailed(&buf)
                .unwrap()
                .list_of_values
                .len(),
            MAX_DECODED_ITEMS
        );

        req.list_of_values.push(value);
        buf.clear();
        req.encode(&mut buf);
        assert_eq!(
            COVNotificationRequest::decode_detailed(&buf)
                .unwrap_err()
                .reject_reason(),
            RejectReason::BUFFER_OVERFLOW
        );
    }

    #[test]
    fn test_decode_cov_notification_classifies_trailing_arguments() {
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
        buf.extend_from_slice(&[0x59, 0x01]);
        assert_eq!(
            COVNotificationRequest::decode_detailed(&buf)
                .unwrap_err()
                .reject_reason(),
            RejectReason::TOO_MANY_ARGUMENTS
        );

        buf.truncate(buf.len() - 2);
        buf.extend_from_slice(&[0xFF]);
        assert_eq!(
            COVNotificationRequest::decode_detailed(&buf)
                .unwrap_err()
                .reject_reason(),
            RejectReason::INVALID_TAG
        );
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
}

#[cfg(test)]
#[path = "cov_property_validation_tests.rs"]
mod property_validation_tests;

#[cfg(test)]
#[path = "cov_request_validation_tests.rs"]
mod request_validation_tests;
