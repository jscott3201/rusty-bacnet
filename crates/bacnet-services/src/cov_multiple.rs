//! SubscribeCOVPropertyMultiple and COVNotificationMultiple services
//! per ASHRAE 135-2020 Clauses 13.16–13.18.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::{PropertyIdentifier, RejectReason};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::BytesMut;

use crate::common::{decode_context, decode_context_u32, PropertyReference, MAX_DECODED_ITEMS};

#[path = "cov_multiple_helpers.rs"]
mod helpers;
use helpers::{
    actual_time_is_valid, decode_date_time, decode_required_bool, reject,
    validate_actual_date_time, validate_actual_time, validate_decoded_property_identifier,
    validate_property_identifier, validate_raw_property_value,
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
    pub issue_confirmed_notifications: bool,
    pub lifetime: Option<u32>,
    pub max_notification_delay: Option<u32>,
    pub list_of_cov_subscription_specifications: Vec<COVSubscriptionSpecification>,
}

impl SubscribeCOVPropertyMultipleRequest {
    /// Encode a validated request without mutating `buf` on failure.
    pub fn try_encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
        self.encode_validated(buf);
        Ok(())
    }

    /// Encode a request known to satisfy the service model.
    ///
    /// Dynamic or untrusted models should use [`Self::try_encode`] instead.
    pub fn encode(&self, buf: &mut BytesMut) {
        self.try_encode(buf)
            .expect("valid SubscribeCOVPropertyMultiple request");
    }

    fn validate(&self) -> Result<(), Error> {
        match (self.lifetime, self.max_notification_delay) {
            (None, None) => {}
            (Some(lifetime), Some(max_delay))
                if lifetime != 0 && max_delay <= 3_600 && max_delay < lifetime => {}
            (Some(_), Some(_)) => {
                return Err(Error::Encoding(
                    "SubscribeCOVPropertyMultiple timing values are out of range".into(),
                ));
            }
            _ => {
                return Err(Error::Encoding(
                    "SubscribeCOVPropertyMultiple lifetime and max-notification-delay must both be present or absent".into(),
                ));
            }
        }
        let mut total_references = 0usize;
        for spec in &self.list_of_cov_subscription_specifications {
            if spec.list_of_cov_references.is_empty() {
                return Err(Error::Encoding(
                    "SubscribeCOVPropertyMultiple COV-reference list must not be empty".into(),
                ));
            }
            total_references = total_references
                .checked_add(spec.list_of_cov_references.len())
                .ok_or_else(|| Error::Encoding("COV-reference count overflow".into()))?;
            if total_references > MAX_DECODED_ITEMS {
                return Err(Error::Encoding(format!(
                    "SubscribeCOVPropertyMultiple exceeds {MAX_DECODED_ITEMS} COV references"
                )));
            }
            for cov_ref in &spec.list_of_cov_references {
                validate_property_identifier(
                    cov_ref.monitored_property.property_identifier,
                    "SubscribeCOVPropertyMultiple monitored property",
                )?;
            }
        }
        Ok(())
    }

    fn encode_validated(&self, buf: &mut BytesMut) {
        // [0] subscriberProcessIdentifier
        primitives::encode_ctx_unsigned(buf, 0, self.subscriber_process_identifier as u64);
        // [1] issueConfirmedNotifications
        primitives::encode_ctx_boolean(buf, 1, self.issue_confirmed_notifications);
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
                // [2] timestamped
                primitives::encode_ctx_boolean(buf, 2, cov_ref.timestamped);
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
        let (issue_confirmed_notifications, end) = decode_required_bool(
            data,
            offset,
            1,
            "SubscribeCOVPropertyMultiple confirmed-notifications",
        )?;
        offset = end;

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
        let mut total_references = 0usize;
        loop {
            if offset >= data.len() {
                return Err(Error::decoding(
                    offset,
                    "SubscribeCOVPropertyMultiple missing closing tag 4",
                ));
            }
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_closing_tag(4) {
                offset = tag_end;
                break;
            }
            if specs.len() >= MAX_DECODED_ITEMS {
                return Err(reject(
                    RejectReason::BUFFER_OVERFLOW,
                    "too many subscription specs",
                ));
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
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if tag.is_closing_tag(1) {
                    if refs.is_empty() {
                        return Err(reject(
                            RejectReason::PARAMETER_OUT_OF_RANGE,
                            "SubscribeCOVPropertyMultiple COV-reference list is empty",
                        ));
                    }
                    offset = tag_end;
                    break;
                }
                if total_references >= MAX_DECODED_ITEMS {
                    return Err(reject(
                        RejectReason::BUFFER_OVERFLOW,
                        "too many COV references",
                    ));
                }

                // [0] monitoredProperty — opening tag 0
                if !tag.is_opening_tag(0) {
                    return Err(Error::decoding(
                        offset,
                        "SubscribeCOVPropertyMultiple expected opening tag 0 for property ref",
                    ));
                }
                let (prop_ref, new_off) = PropertyReference::decode(data, tag_end)?;
                validate_decoded_property_identifier(
                    prop_ref.property_identifier,
                    "SubscribeCOVPropertyMultiple monitored property",
                )?;
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

                // [2] timestamped
                let (timestamped, end) = decode_required_bool(
                    data,
                    offset,
                    2,
                    "SubscribeCOVPropertyMultiple timestamped",
                )?;
                offset = end;

                refs.push(COVReference {
                    monitored_property: prop_ref,
                    cov_increment,
                    timestamped,
                });
                total_references += 1;
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
    pub time_of_change: Option<Time>,
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
    /// Date and time of the last conveyed timestamped change.
    pub timestamp: Option<(Date, Time)>,
    pub list_of_cov_notifications: Vec<COVNotificationItem>,
}

impl COVNotificationMultipleRequest {
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
        let mut encoded = BytesMut::new();
        // [0] subscriberProcessIdentifier
        primitives::encode_ctx_unsigned(&mut encoded, 0, self.subscriber_process_identifier as u64);
        // [1] initiatingDeviceIdentifier
        primitives::encode_ctx_object_id(&mut encoded, 1, &self.initiating_device_identifier);
        // [2] timeRemaining
        primitives::encode_ctx_unsigned(&mut encoded, 2, self.time_remaining as u64);
        // [3] timestamp OPTIONAL — BACnetDateTime, not BACnetTimeStamp
        if let Some((date, time)) = &self.timestamp {
            tags::encode_opening_tag(&mut encoded, 3);
            primitives::encode_app_date(&mut encoded, date);
            primitives::encode_app_time(&mut encoded, time);
            tags::encode_closing_tag(&mut encoded, 3);
        }
        // [4] listOfCovNotifications
        tags::encode_opening_tag(&mut encoded, 4);
        for item in &self.list_of_cov_notifications {
            // [0] monitoredObjectIdentifier
            primitives::encode_ctx_object_id(&mut encoded, 0, &item.monitored_object_identifier);
            // [1] listOfValues
            tags::encode_opening_tag(&mut encoded, 1);
            for val in &item.list_of_values {
                // [0] propertyIdentifier
                primitives::encode_ctx_unsigned(
                    &mut encoded,
                    0,
                    val.property_identifier.to_raw() as u64,
                );
                // [1] propertyArrayIndex OPTIONAL
                if let Some(idx) = val.property_array_index {
                    primitives::encode_ctx_unsigned(&mut encoded, 1, idx as u64);
                }
                // [2] value (opening/closing)
                tags::encode_opening_tag(&mut encoded, 2);
                encoded.extend_from_slice(&val.value);
                tags::encode_closing_tag(&mut encoded, 2);
                // [3] timeOfChange OPTIONAL — primitive context Time
                if let Some(time) = &val.time_of_change {
                    tags::encode_tag(&mut encoded, 3, tags::TagClass::Context, 4);
                    encoded.extend_from_slice(&time.encode());
                }
            }
            tags::encode_closing_tag(&mut encoded, 1);
        }
        tags::encode_closing_tag(&mut encoded, 4);
        buf.extend_from_slice(&encoded);
        Ok(())
    }

    fn validate(&self) -> Result<(), Error> {
        if self.list_of_cov_notifications.is_empty() {
            return Err(Error::Encoding(
                "COVNotificationMultiple notification list must not be empty".into(),
            ));
        }
        let mut total_values = 0usize;
        let mut has_time_of_change = false;
        if let Some((date, time)) = &self.timestamp {
            validate_actual_date_time(date, time)?;
        }
        for item in &self.list_of_cov_notifications {
            if item.list_of_values.is_empty() {
                return Err(Error::Encoding(
                    "COVNotificationMultiple value list must not be empty".into(),
                ));
            }
            total_values = total_values
                .checked_add(item.list_of_values.len())
                .ok_or_else(|| Error::Encoding("notification value count overflow".into()))?;
            if total_values > MAX_DECODED_ITEMS {
                return Err(Error::Encoding(format!(
                    "COVNotificationMultiple exceeds {MAX_DECODED_ITEMS} values"
                )));
            }
            for value in &item.list_of_values {
                validate_property_identifier(
                    value.property_identifier,
                    "COVNotificationMultiple property",
                )?;
                validate_raw_property_value(&value.value)?;
                if let Some(time) = &value.time_of_change {
                    validate_actual_time(time)?;
                }
                has_time_of_change |= value.time_of_change.is_some();
            }
        }
        if self.timestamp.is_some() != has_time_of_change {
            return Err(Error::Encoding(
                "COVNotificationMultiple timestamp must be present iff a time-of-change is present"
                    .into(),
            ));
        }
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

        // [3] timestamp OPTIONAL — BACnetDateTime
        let mut timestamp = None;
        if offset < data.len() {
            let (tag, _) = tags::decode_tag(data, offset)?;
            if tag.is_opening_tag(3) {
                let (date_time, end) = decode_date_time(data, offset, 3)?;
                timestamp = Some(date_time);
                offset = end;
            }
        }

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
        let mut total_values = 0usize;
        let mut has_time_of_change = false;
        loop {
            if offset >= data.len() {
                return Err(Error::decoding(
                    offset,
                    "COVNotificationMultiple missing closing tag 4",
                ));
            }
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_closing_tag(4) {
                if items.is_empty() {
                    return Err(reject(
                        RejectReason::PARAMETER_OUT_OF_RANGE,
                        "COVNotificationMultiple notification list is empty",
                    ));
                }
                offset = tag_end;
                break;
            }
            if items.len() >= MAX_DECODED_ITEMS {
                return Err(reject(
                    RejectReason::BUFFER_OVERFLOW,
                    "too many notification items",
                ));
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
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if tag.is_closing_tag(1) {
                    if values.is_empty() {
                        return Err(reject(
                            RejectReason::PARAMETER_OUT_OF_RANGE,
                            "COVNotificationMultiple value list is empty",
                        ));
                    }
                    offset = tag_end;
                    break;
                }
                if total_values >= MAX_DECODED_ITEMS {
                    return Err(reject(
                        RejectReason::BUFFER_OVERFLOW,
                        "too many notification values",
                    ));
                }

                // [0] propertyIdentifier
                let (prop_id, end) =
                    decode_context_u32(data, offset, 0, "COVNotificationMultiple property-id")?;
                offset = end;
                let property_identifier = PropertyIdentifier::from_raw(prop_id);
                validate_decoded_property_identifier(
                    property_identifier,
                    "COVNotificationMultiple property",
                )?;

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
                if value_bytes.is_empty() {
                    return Err(reject(
                        RejectReason::INVALID_DATA_ENCODING,
                        "COVNotificationMultiple property value is empty",
                    ));
                }
                let value = value_bytes.to_vec();
                offset = new_off;

                // [3] timeOfChange OPTIONAL — primitive context Time
                let mut time_of_change = None;
                if offset < data.len() {
                    let (peek, _) = tags::decode_tag(data, offset)?;
                    if peek.is_context(3) {
                        let (content, end) = decode_context(
                            data,
                            offset,
                            3,
                            "COVNotificationMultiple time-of-change",
                        )?;
                        let decoded_time = Time::decode(content).map_err(|_| {
                            reject(
                                RejectReason::INVALID_DATA_ENCODING,
                                "COVNotificationMultiple time-of-change is malformed",
                            )
                        })?;
                        if !actual_time_is_valid(&decoded_time) {
                            return Err(reject(
                                RejectReason::INVALID_DATA_ENCODING,
                                "COVNotificationMultiple time-of-change is not an actual Time",
                            ));
                        }
                        time_of_change = Some(decoded_time);
                        has_time_of_change = true;
                        offset = end;
                    }
                }

                values.push(COVNotificationValue {
                    property_identifier,
                    property_array_index: array_index,
                    value,
                    time_of_change,
                });
                total_values += 1;
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
        if timestamp.is_some() != has_time_of_change {
            return Err(reject(
                RejectReason::PARAMETER_OUT_OF_RANGE,
                "COVNotificationMultiple timestamp/time-of-change mismatch",
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
#[path = "cov_multiple_conformance_tests.rs"]
mod conformance_tests;

#[cfg(test)]
#[path = "cov_multiple_tests.rs"]
mod tests;
