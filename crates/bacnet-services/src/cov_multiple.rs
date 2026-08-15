//! SubscribeCOVPropertyMultiple and COVNotificationMultiple services
//! per ASHRAE 135-2020 Clauses 13.16 / 13.18.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier};
use bytes::BytesMut;

use crate::common::{
    decode_context, decode_context_bool, decode_context_u32, PropertyReference, MAX_DECODED_ITEMS,
};

// ---------------------------------------------------------------------------
// SubscribeCOVPropertyMultipleRequest
// ---------------------------------------------------------------------------

/// A single COV reference within a subscription specification.
#[derive(Debug, Clone, PartialEq)]
pub struct COVReference {
    pub monitored_property: PropertyReference,
    pub cov_increment: Option<f32>,
    pub timestamped: bool,
}

/// A single subscription specification (object + list of property references).
#[derive(Debug, Clone, PartialEq)]
pub struct COVSubscriptionSpecification {
    pub monitored_object_identifier: ObjectIdentifier,
    pub list_of_cov_references: Vec<COVReference>,
}

/// SubscribeCOVPropertyMultiple-Request service parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscribeCOVPropertyMultipleRequest {
    pub subscriber_process_identifier: u32,
    pub issue_confirmed_notifications: Option<bool>,
    pub lifetime: Option<u32>,
    pub max_notification_delay: Option<u32>,
    pub list_of_cov_subscription_specifications: Vec<COVSubscriptionSpecification>,
}

impl SubscribeCOVPropertyMultipleRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] subscriberProcessIdentifier
        primitives::encode_ctx_unsigned(buf, 0, self.subscriber_process_identifier as u64);
        // [1] issueConfirmedNotifications
        if let Some(v) = self.issue_confirmed_notifications {
            primitives::encode_ctx_boolean(buf, 1, v);
        }
        // [2] lifetime OPTIONAL
        if let Some(v) = self.lifetime {
            primitives::encode_ctx_unsigned(buf, 2, v as u64);
        }
        // [3] maxNotificationDelay OPTIONAL
        if let Some(v) = self.max_notification_delay {
            primitives::encode_ctx_unsigned(buf, 3, v as u64);
        }
        // [4] listOfCovSubscriptionSpecifications
        tags::encode_opening_tag(buf, 4);
        for spec in &self.list_of_cov_subscription_specifications {
            // [0] monitoredObjectIdentifier
            primitives::encode_ctx_object_id(buf, 0, &spec.monitored_object_identifier);
            // [1] listOfCovReferences
            tags::encode_opening_tag(buf, 1);
            for cov_ref in &spec.list_of_cov_references {
                // [0] monitoredProperty (BACnetPropertyReference)
                tags::encode_opening_tag(buf, 0);
                cov_ref.monitored_property.encode(buf);
                tags::encode_closing_tag(buf, 0);
                // [1] covIncrement OPTIONAL
                if let Some(inc) = cov_ref.cov_increment {
                    primitives::encode_ctx_real(buf, 1, inc);
                }
                // [2] timestamped DEFAULT FALSE
                if cov_ref.timestamped {
                    primitives::encode_ctx_boolean(buf, 2, true);
                }
            }
            tags::encode_closing_tag(buf, 1);
        }
        tags::encode_closing_tag(buf, 4);
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] subscriberProcessIdentifier
        let (subscriber_process_identifier, end) =
            decode_context_u32(data, offset, 0, "SubscribeCOVPropertyMultiple process-id")?;
        offset = end;

        // [1] issueConfirmedNotifications
        let mut issue_confirmed_notifications = None;
        if offset < data.len() {
            let (tag, _) = tags::decode_tag(data, offset)?;
            if tag.is_context(1) {
                let (value, end) = decode_context_bool(
                    data,
                    offset,
                    1,
                    "SubscribeCOVPropertyMultiple confirmed-notifications",
                )?;
                issue_confirmed_notifications = Some(value);
                offset = end;
            }
        }

        // [2] lifetime OPTIONAL
        let mut lifetime = None;
        if offset < data.len() {
            let (tag, _) = tags::decode_tag(data, offset)?;
            if tag.is_context(2) {
                let (value, end) =
                    decode_context_u32(data, offset, 2, "SubscribeCOVPropertyMultiple lifetime")?;
                lifetime = Some(value);
                offset = end;
            }
        }

        // [3] maxNotificationDelay OPTIONAL
        let mut max_notification_delay = None;
        if offset < data.len() {
            let (tag, _) = tags::decode_tag(data, offset)?;
            if tag.is_context(3) {
                let (value, end) = decode_context_u32(
                    data,
                    offset,
                    3,
                    "SubscribeCOVPropertyMultiple max-notification-delay",
                )?;
                max_notification_delay = Some(value);
                offset = end;
            }
        }

        // [4] listOfCovSubscriptionSpecifications — opening tag 4
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(4) {
            return Err(Error::decoding(
                offset,
                "SubscribeCOVPropertyMultiple expected opening tag 4",
            ));
        }
        offset = tag_end;

        let mut specs = Vec::new();
        loop {
            if offset >= data.len() {
                return Err(Error::decoding(
                    offset,
                    "SubscribeCOVPropertyMultiple missing closing tag 4",
                ));
            }
            if specs.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(offset, "too many subscription specs"));
            }
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_closing_tag(4) {
                offset = tag_end;
                break;
            }

            // [0] monitoredObjectIdentifier
            let (content, end) =
                decode_context(data, offset, 0, "SubscribeCOVPropertyMultiple object-id")?;
            let oid = ObjectIdentifier::decode(content)?;
            offset = end;

            // [1] listOfCovReferences — opening tag 1
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(1) {
                return Err(Error::decoding(
                    offset,
                    "SubscribeCOVPropertyMultiple expected opening tag 1",
                ));
            }
            offset = tag_end;

            let mut refs = Vec::new();
            loop {
                if offset >= data.len() {
                    return Err(Error::decoding(
                        offset,
                        "SubscribeCOVPropertyMultiple missing closing tag 1",
                    ));
                }
                if refs.len() >= MAX_DECODED_ITEMS {
                    return Err(Error::decoding(offset, "too many COV references"));
                }
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if tag.is_closing_tag(1) {
                    offset = tag_end;
                    break;
                }

                // [0] monitoredProperty — opening tag 0
                if !tag.is_opening_tag(0) {
                    return Err(Error::decoding(
                        offset,
                        "SubscribeCOVPropertyMultiple expected opening tag 0 for property ref",
                    ));
                }
                let (prop_ref, new_off) = PropertyReference::decode(data, tag_end)?;
                offset = new_off;
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if !tag.is_closing_tag(0) {
                    return Err(Error::decoding(
                        offset,
                        "SubscribeCOVPropertyMultiple expected closing tag 0",
                    ));
                }
                offset = tag_end;

                // [1] covIncrement OPTIONAL
                let mut cov_increment = None;
                if offset < data.len() {
                    let (opt, new_off) = tags::decode_optional_context(data, offset, 1)?;
                    if let Some(content) = opt {
                        cov_increment = Some(primitives::decode_real(content)?);
                        offset = new_off;
                    }
                }

                // [2] timestamped DEFAULT FALSE
                let mut timestamped = false;
                if offset < data.len() {
                    let (tag, _) = tags::decode_tag(data, offset)?;
                    if tag.is_context(2) {
                        let (value, end) = decode_context_bool(
                            data,
                            offset,
                            2,
                            "SubscribeCOVPropertyMultiple timestamped",
                        )?;
                        timestamped = value;
                        offset = end;
                    }
                }

                refs.push(COVReference {
                    monitored_property: prop_ref,
                    cov_increment,
                    timestamped,
                });
            }

            specs.push(COVSubscriptionSpecification {
                monitored_object_identifier: oid,
                list_of_cov_references: refs,
            });
        }
        if offset != data.len() {
            return Err(Error::decoding(
                offset,
                "SubscribeCOVPropertyMultiple has trailing data",
            ));
        }

        Ok(Self {
            subscriber_process_identifier,
            issue_confirmed_notifications,
            lifetime,
            max_notification_delay,
            list_of_cov_subscription_specifications: specs,
        })
    }
}

// ---------------------------------------------------------------------------
// COVNotificationMultipleRequest
// ---------------------------------------------------------------------------

/// A single value entry in a COV notification list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct COVNotificationValue {
    pub property_identifier: PropertyIdentifier,
    pub property_array_index: Option<u32>,
    /// Raw application-tagged bytes for the value.
    pub value: Vec<u8>,
    pub time_of_change: Option<Vec<u8>>,
}

/// A single object notification within a COVNotificationMultiple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct COVNotificationItem {
    pub monitored_object_identifier: ObjectIdentifier,
    pub list_of_values: Vec<COVNotificationValue>,
}

/// COVNotificationMultiple-Request service parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct COVNotificationMultipleRequest {
    pub subscriber_process_identifier: u32,
    pub initiating_device_identifier: ObjectIdentifier,
    pub time_remaining: u32,
    pub timestamp: BACnetTimeStamp,
    pub list_of_cov_notifications: Vec<COVNotificationItem>,
}

impl COVNotificationMultipleRequest {
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] subscriberProcessIdentifier
        primitives::encode_ctx_unsigned(buf, 0, self.subscriber_process_identifier as u64);
        // [1] initiatingDeviceIdentifier
        primitives::encode_ctx_object_id(buf, 1, &self.initiating_device_identifier);
        // [2] timeRemaining
        primitives::encode_ctx_unsigned(buf, 2, self.time_remaining as u64);
        // [3] timestamp
        primitives::encode_timestamp(buf, 3, &self.timestamp)?;
        // [4] listOfCovNotifications
        tags::encode_opening_tag(buf, 4);
        for item in &self.list_of_cov_notifications {
            // [0] monitoredObjectIdentifier
            primitives::encode_ctx_object_id(buf, 0, &item.monitored_object_identifier);
            // [1] listOfValues
            tags::encode_opening_tag(buf, 1);
            for val in &item.list_of_values {
                // [0] propertyIdentifier
                primitives::encode_ctx_unsigned(buf, 0, val.property_identifier.to_raw() as u64);
                // [1] propertyArrayIndex OPTIONAL
                if let Some(idx) = val.property_array_index {
                    primitives::encode_ctx_unsigned(buf, 1, idx as u64);
                }
                // [2] value (opening/closing)
                tags::encode_opening_tag(buf, 2);
                buf.extend_from_slice(&val.value);
                tags::encode_closing_tag(buf, 2);
                // [3] timeOfChange OPTIONAL (opening/closing)
                if let Some(ref ts) = val.time_of_change {
                    tags::encode_opening_tag(buf, 3);
                    buf.extend_from_slice(ts);
                    tags::encode_closing_tag(buf, 3);
                }
            }
            tags::encode_closing_tag(buf, 1);
        }
        tags::encode_closing_tag(buf, 4);
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] subscriberProcessIdentifier
        let (subscriber_process_identifier, end) =
            decode_context_u32(data, offset, 0, "COVNotificationMultiple process-id")?;
        offset = end;

        // [1] initiatingDeviceIdentifier
        let (content, end) = decode_context(data, offset, 1, "COVNotificationMultiple device-id")?;
        let initiating_device_identifier = ObjectIdentifier::decode(content)?;
        offset = end;

        // [2] timeRemaining
        let (time_remaining, end) =
            decode_context_u32(data, offset, 2, "COVNotificationMultiple time-remaining")?;
        offset = end;

        // [3] timestamp
        let (timestamp, new_off) = primitives::decode_timestamp(data, offset, 3)?;
        offset = new_off;

        // [4] listOfCovNotifications — opening tag 4
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(4) {
            return Err(Error::decoding(
                offset,
                "COVNotificationMultiple expected opening tag 4",
            ));
        }
        offset = tag_end;

        let mut items = Vec::new();
        loop {
            if offset >= data.len() {
                return Err(Error::decoding(
                    offset,
                    "COVNotificationMultiple missing closing tag 4",
                ));
            }
            if items.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(offset, "too many notification items"));
            }
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_closing_tag(4) {
                offset = tag_end;
                break;
            }

            // [0] monitoredObjectIdentifier
            let (content, end) =
                decode_context(data, offset, 0, "COVNotificationMultiple monitored-id")?;
            let oid = ObjectIdentifier::decode(content)?;
            offset = end;

            // [1] listOfValues — opening tag 1
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(1) {
                return Err(Error::decoding(
                    offset,
                    "COVNotificationMultiple expected opening tag 1",
                ));
            }
            offset = tag_end;

            let mut values = Vec::new();
            loop {
                if offset >= data.len() {
                    return Err(Error::decoding(
                        offset,
                        "COVNotificationMultiple missing closing tag 1",
                    ));
                }
                if values.len() >= MAX_DECODED_ITEMS {
                    return Err(Error::decoding(offset, "too many notification values"));
                }
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if tag.is_closing_tag(1) {
                    offset = tag_end;
                    break;
                }

                // [0] propertyIdentifier
                let (prop_id, end) =
                    decode_context_u32(data, offset, 0, "COVNotificationMultiple property-id")?;
                offset = end;

                // [1] propertyArrayIndex OPTIONAL
                let mut array_index = None;
                if offset < data.len() {
                    let (tag, _) = tags::decode_tag(data, offset)?;
                    if tag.is_context(1) {
                        let (value, end) = decode_context_u32(
                            data,
                            offset,
                            1,
                            "COVNotificationMultiple array-index",
                        )?;
                        array_index = Some(value);
                        offset = end;
                    }
                }

                // [2] value (opening/closing)
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if !tag.is_opening_tag(2) {
                    return Err(Error::decoding(
                        offset,
                        "COVNotificationMultiple expected opening tag 2",
                    ));
                }
                let (value_bytes, new_off) = tags::extract_context_value(data, tag_end, 2)?;
                let value = value_bytes.to_vec();
                offset = new_off;

                // [3] timeOfChange OPTIONAL (opening/closing)
                let mut time_of_change = None;
                if offset < data.len() {
                    let (peek, _) = tags::decode_tag(data, offset)?;
                    if peek.is_opening_tag(3) {
                        let (_, inner_start) = tags::decode_tag(data, offset)?;
                        let (ts_bytes, new_off) =
                            tags::extract_context_value(data, inner_start, 3)?;
                        time_of_change = Some(ts_bytes.to_vec());
                        offset = new_off;
                    }
                }

                values.push(COVNotificationValue {
                    property_identifier: PropertyIdentifier::from_raw(prop_id),
                    property_array_index: array_index,
                    value,
                    time_of_change,
                });
            }

            items.push(COVNotificationItem {
                monitored_object_identifier: oid,
                list_of_values: values,
            });
        }
        if offset != data.len() {
            return Err(Error::decoding(
                offset,
                "COVNotificationMultiple has trailing data",
            ));
        }

        Ok(Self {
            subscriber_process_identifier,
            initiating_device_identifier,
            time_remaining,
            timestamp,
            list_of_cov_notifications: items,
        })
    }
}

#[cfg(test)]
#[path = "cov_multiple_width_tests.rs"]
mod width_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::Time;

    #[test]
    fn subscribe_cov_property_multiple_round_trip() {
        let req = SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier: 42,
            issue_confirmed_notifications: Some(true),
            lifetime: Some(60),
            max_notification_delay: Some(10),
            list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                    .unwrap(),
                list_of_cov_references: vec![
                    COVReference {
                        monitored_property: PropertyReference {
                            property_identifier: PropertyIdentifier::PRESENT_VALUE,
                            property_array_index: None,
                        },
                        cov_increment: Some(1.0),
                        timestamped: true,
                    },
                    COVReference {
                        monitored_property: PropertyReference {
                            property_identifier: PropertyIdentifier::STATUS_FLAGS,
                            property_array_index: None,
                        },
                        cov_increment: None,
                        timestamped: false,
                    },
                ],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = SubscribeCOVPropertyMultipleRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn subscribe_cov_property_multiple_uses_standard_tags() {
        let req = SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier: 42,
            issue_confirmed_notifications: Some(true),
            lifetime: Some(60),
            max_notification_delay: Some(5),
            list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 10)
                    .unwrap(),
                list_of_cov_references: vec![COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    cov_increment: Some(1.0),
                    timestamped: true,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);

        let mut offset = 0;
        for tag_number in [0, 1, 2, 3] {
            let (tag, pos) = tags::decode_tag(&buf, offset).unwrap();
            assert!(tag.is_context(tag_number));
            offset = pos + tag.length as usize;
        }
        let (tag, _) = tags::decode_tag(&buf, offset).unwrap();
        assert!(tag.is_opening_tag(4));
    }

    #[test]
    fn subscribe_cov_property_multiple_rejects_malformed_header_fields() {
        let mut malformed_bool = BytesMut::new();
        primitives::encode_ctx_unsigned(&mut malformed_bool, 0, 1);
        tags::encode_tag(&mut malformed_bool, 1, tags::TagClass::Context, 0);
        tags::encode_opening_tag(&mut malformed_bool, 4);
        tags::encode_closing_tag(&mut malformed_bool, 4);
        assert!(SubscribeCOVPropertyMultipleRequest::decode(&malformed_bool).is_err());

        let mut oversized_lifetime = BytesMut::new();
        primitives::encode_ctx_unsigned(&mut oversized_lifetime, 0, 1);
        primitives::encode_ctx_boolean(&mut oversized_lifetime, 1, true);
        tags::encode_tag(&mut oversized_lifetime, 2, tags::TagClass::Context, 5);
        oversized_lifetime.extend_from_slice(&[1, 0, 0, 0, 0]);
        tags::encode_opening_tag(&mut oversized_lifetime, 4);
        tags::encode_closing_tag(&mut oversized_lifetime, 4);
        assert!(SubscribeCOVPropertyMultipleRequest::decode(&oversized_lifetime).is_err());
    }

    #[test]
    fn subscribe_cov_property_multiple_minimal() {
        let req = SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier: 1,
            issue_confirmed_notifications: None,
            lifetime: None,
            max_notification_delay: None,
            list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::BINARY_INPUT, 5)
                    .unwrap(),
                list_of_cov_references: vec![COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    cov_increment: None,
                    timestamped: false,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = SubscribeCOVPropertyMultipleRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn cov_notification_multiple_round_trip() {
        let req = COVNotificationMultipleRequest {
            subscriber_process_identifier: 1,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
            time_remaining: 60,
            timestamp: BACnetTimeStamp::SequenceNumber(42),
            list_of_cov_notifications: vec![COVNotificationItem {
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                    .unwrap(),
                list_of_values: vec![
                    COVNotificationValue {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                        time_of_change: None,
                    },
                    COVNotificationValue {
                        property_identifier: PropertyIdentifier::STATUS_FLAGS,
                        property_array_index: None,
                        value: vec![0x82, 0x04, 0x00],
                        time_of_change: Some(vec![0x19, 0x2A]), // raw timestamp bytes
                    },
                ],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = COVNotificationMultipleRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn cov_notification_multiple_with_time_timestamp() {
        let req = COVNotificationMultipleRequest {
            subscriber_process_identifier: 5,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap(),
            time_remaining: 0,
            timestamp: BACnetTimeStamp::Time(Time {
                hour: 12,
                minute: 30,
                second: 0,
                hundredths: 0,
            }),
            list_of_cov_notifications: vec![COVNotificationItem {
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::BINARY_VALUE, 3)
                    .unwrap(),
                list_of_values: vec![COVNotificationValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x91, 0x01],
                    time_of_change: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = COVNotificationMultipleRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn subscribe_cov_property_multiple_empty_input() {
        assert!(SubscribeCOVPropertyMultipleRequest::decode(&[]).is_err());
    }

    #[test]
    fn cov_notification_multiple_empty_input() {
        assert!(COVNotificationMultipleRequest::decode(&[]).is_err());
    }
}
