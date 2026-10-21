//! COVNotificationMultiple-Request types and codec (split from `cov_multiple`).

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::{PropertyIdentifier, RejectReason};
use bacnet_types::error::{EncodeError, Error};
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::BytesMut;

use super::helpers::{
    actual_time_is_valid, decode_date_time, reject, validate_actual_date_time,
    validate_actual_time, validate_decoded_property_identifier, validate_property_identifier,
    validate_raw_property_value,
};
use crate::common::MAX_DECODED_ITEMS;
use bacnet_encoding::constructed::tagged::{
    decode_ctx_fixed, decode_ctx_object_id, decode_ctx_unsigned, expect_end, misplaced_tag, Fixed,
    next_is_context,
};

// ---------------------------------------------------------------------------
// COVNotificationMultipleRequest
// ---------------------------------------------------------------------------

/// A single value entry in a COV notification list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct COVNotificationValue {
    /// Property that changed.
    pub property_identifier: PropertyIdentifier,
    /// Array element that changed; `None` means the whole property.
    pub property_array_index: Option<u32>,
    /// Raw application-tagged bytes for the value.
    pub value: Vec<u8>,
    /// Time of the change; `None` when the sender did not timestamp this value.
    pub time_of_change: Option<Time>,
}

/// A single object notification within a COVNotificationMultiple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct COVNotificationItem {
    /// Object whose values are reported.
    pub monitored_object_identifier: ObjectIdentifier,
    /// Changed properties of that object with their new values.
    pub list_of_values: Vec<COVNotificationValue>,
}

/// COVNotificationMultiple-Request service parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct COVNotificationMultipleRequest {
    /// Subscriber-chosen handle from the subscription, used to route the notification.
    pub subscriber_process_identifier: u32,
    /// Device that sent the notification.
    pub initiating_device_identifier: ObjectIdentifier,
    /// Seconds left before the subscription expires; 0 means it never expires.
    pub time_remaining: u32,
    /// Date and time of the last conveyed timestamped change.
    pub timestamp: Option<(Date, Time)>,
    /// Per-object groups of changed values carried by this notification.
    pub list_of_cov_notifications: Vec<COVNotificationItem>,
}

impl COVNotificationMultipleRequest {
    /// Validate and append the ASN.1 encoding to `buf`; `buf` is unchanged on failure.
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
            return Err(Error::encode("COVNotificationMultiple notification list must not be empty"));
        }
        let mut total_values = 0usize;
        let mut has_time_of_change = false;
        if let Some((date, time)) = &self.timestamp {
            validate_actual_date_time(date, time)?;
        }
        for item in &self.list_of_cov_notifications {
            if item.list_of_values.is_empty() {
                return Err(Error::encode("COVNotificationMultiple value list must not be empty"));
            }
            total_values = total_values
                .checked_add(item.list_of_values.len())
                .ok_or_else(|| Error::encode("notification value count overflow"))?;
            if total_values > MAX_DECODED_ITEMS {
                return Err(EncodeError::new("COVNotificationMultiple exceeds {} values")
                    .with_value(MAX_DECODED_ITEMS as u32)
                    .into());
            }
            for value in &item.list_of_values {
                validate_property_identifier(
                    value.property_identifier,
                    "COVNotificationMultiple property cannot be ALL, OPTIONAL, or REQUIRED",
                )?;
                validate_raw_property_value(&value.value)?;
                if let Some(time) = &value.time_of_change {
                    validate_actual_time(time)?;
                }
                has_time_of_change |= value.time_of_change.is_some();
            }
        }
        if self.timestamp.is_some() != has_time_of_change {
            return Err(Error::encode("COVNotificationMultiple timestamp must be present iff a time-of-change is present"));
        }
        Ok(())
    }

    /// Decode the notification from `data`; errors on missing, malformed or truncated fields.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] subscriberProcessIdentifier
        let (subscriber_process_identifier, end) =
            decode_ctx_unsigned::<u32>(data, offset, 0, "COVNotificationMultiple process-id")?;
        offset = end;

        // [1] initiatingDeviceIdentifier
        let (initiating_device_identifier, end) =
            decode_ctx_object_id(data, offset, 1, "COVNotificationMultiple device-id")?;
        offset = end;

        // [2] timeRemaining
        let (time_remaining, end) =
            decode_ctx_unsigned::<u32>(data, offset, 2, "COVNotificationMultiple time-remaining")?;
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
            return Err(misplaced_tag(
                data,
                &tag,
                Some(4),
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
                return Err(Error::missing(
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
            let (oid, end) =
                decode_ctx_object_id(data, offset, 0, "COVNotificationMultiple monitored-id")?;
            offset = end;

            // [1] listOfValues — opening tag 1
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(1) {
                return Err(misplaced_tag(
                    data,
                    &tag,
                    Some(1),
                    offset,
                    "COVNotificationMultiple expected opening tag 1",
                ));
            }
            offset = tag_end;

            let mut values = Vec::new();
            loop {
                if offset >= data.len() {
                    return Err(Error::missing(
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
                let (prop_id, end) = decode_ctx_unsigned::<u32>(
                    data,
                    offset,
                    0,
                    "COVNotificationMultiple property-id",
                )?;
                offset = end;
                let property_identifier = PropertyIdentifier::from_raw(prop_id);
                validate_decoded_property_identifier(
                    property_identifier,
                    "COVNotificationMultiple property cannot be ALL, OPTIONAL, or REQUIRED",
                )?;

                // [1] propertyArrayIndex OPTIONAL
                let mut array_index = None;
                if next_is_context(data, offset, 1)? {
                    let (value, end) = decode_ctx_unsigned::<u32>(
                        data,
                        offset,
                        1,
                        "COVNotificationMultiple array-index",
                    )?;
                    array_index = Some(value);
                    offset = end;
                }

                // [2] value (opening/closing)
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if !tag.is_opening_tag(2) {
                    return Err(misplaced_tag(
                        data,
                        &tag,
                        Some(2),
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
                if next_is_context(data, offset, 3)? {
                    let malformed = || {
                        reject(
                            RejectReason::INVALID_DATA_ENCODING,
                            "COVNotificationMultiple time-of-change is malformed",
                        )
                    };
                    // The peek has read the tag, so the only other refusal
                    // is a header of the wrong length: a malformed Time even
                    // when the data also stops early.
                    let (content, end) = decode_ctx_fixed(
                        data,
                        offset,
                        3,
                        Fixed::TIME,
                        "COVNotificationMultiple time-of-change",
                    )
                    .map_err(|error| match error {
                        Error::BufferTooShort { .. } => error,
                        _ => malformed(),
                    })?;
                    let decoded_time = Time::decode(content).map_err(|_| malformed())?;
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
        expect_end(data, offset, offset, "COVNotificationMultiple")?;
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
