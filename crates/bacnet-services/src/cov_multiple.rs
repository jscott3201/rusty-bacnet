//! SubscribeCOVPropertyMultiple and COVNotificationMultiple services
//! per ASHRAE 135-2020 Clauses 13.16–13.18.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_encoding::constructed::{decode_property_reference, encode_property_reference};
use bacnet_encoding::primitives;
use bacnet_encoding::tags;
#[cfg(test)]
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::enums::RejectReason;
use bacnet_types::error::{EncodeError, Error};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;
use bacnet_encoding::constructed::tagged::{
    decode_ctx_object_id, decode_ctx_real, decode_ctx_unsigned, expect_end, misplaced_tag,
    next_is_context, unclosed_kind,
};
use bacnet_types::constructed::PropertyReference;

#[path = "cov_multiple_helpers.rs"]
mod helpers;
use helpers::{
    decode_required_bool, reject, validate_decoded_property_identifier,
    validate_property_identifier,
};

// ---------------------------------------------------------------------------
// SubscribeCOVPropertyMultipleRequest
// ---------------------------------------------------------------------------

/// A single COV reference within a subscription specification.
#[derive(Debug, Clone, PartialEq)]
pub struct COVReference {
    /// Property (and optional array element) to monitor.
    pub monitored_property: PropertyReference,
    /// Minimum change that triggers a notification; `None` uses the object's own COV increment.
    pub cov_increment: Option<f32>,
    /// `true` to include the time of each change in notifications for this property.
    pub timestamped: bool,
}

/// A single subscription specification (object + list of property references).
#[derive(Debug, Clone, PartialEq)]
pub struct COVSubscriptionSpecification {
    /// Object being monitored.
    pub monitored_object_identifier: ObjectIdentifier,
    /// Properties of that object to monitor; must not be empty.
    pub list_of_cov_references: Vec<COVReference>,
}

/// SubscribeCOVPropertyMultiple-Request service parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscribeCOVPropertyMultipleRequest {
    /// Subscriber-chosen handle echoed in every notification for this subscription.
    pub subscriber_process_identifier: u32,
    /// `true` for confirmed notifications, `false` for unconfirmed.
    pub issue_confirmed_notifications: bool,
    /// Lifetime in seconds. A subscription needs it nonzero and paired with
    /// `max_notification_delay`; both absent cancels. Encode enforces the pairing, decode does not.
    pub lifetime: Option<u32>,
    /// Longest delay in seconds (at most 3600, below `lifetime`) a notification may be held for
    /// batching; present exactly when `lifetime` is. Checked on encode only.
    pub max_notification_delay: Option<u32>,
    /// Objects and properties to subscribe to.
    pub list_of_cov_subscription_specifications: Vec<COVSubscriptionSpecification>,
}

impl SubscribeCOVPropertyMultipleRequest {
    /// Validate the complete request, then append it without mutating `buf` on failure.
    ///
    /// Empty outer specifications are encodable for cancellation and valid finite timing;
    /// encoding alone does not establish a server-side subscription context.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
        self.encode_validated(buf);
        Ok(())
    }

    fn validate(&self) -> Result<(), Error> {
        match (self.lifetime, self.max_notification_delay) {
            (None, None) => {}
            (Some(lifetime), Some(max_delay))
                if lifetime != 0 && max_delay <= 3_600 && max_delay < lifetime => {}
            (Some(_), Some(_)) => {
                return Err(Error::encode("SubscribeCOVPropertyMultiple timing values are out of range"));
            }
            _ => {
                return Err(Error::encode("SubscribeCOVPropertyMultiple lifetime and max-notification-delay must both be present or absent"));
            }
        }
        let mut total_references = 0usize;
        for spec in &self.list_of_cov_subscription_specifications {
            if spec.list_of_cov_references.is_empty() {
                return Err(Error::encode("SubscribeCOVPropertyMultiple COV-reference list must not be empty"));
            }
            total_references = total_references
                .checked_add(spec.list_of_cov_references.len())
                .ok_or_else(|| Error::encode("COV-reference count overflow"))?;
            if total_references > MAX_DECODED_ITEMS {
                return Err(EncodeError::new(
                    "SubscribeCOVPropertyMultiple exceeds {} COV references",
                )
                .with_value(MAX_DECODED_ITEMS as u32)
                .into());
            }
            for cov_ref in &spec.list_of_cov_references {
                validate_property_identifier(
                    cov_ref.monitored_property.property_identifier,
                    "SubscribeCOVPropertyMultiple monitored property cannot be ALL, OPTIONAL, or REQUIRED",
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
                encode_property_reference(buf, &cov_ref.monitored_property);
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

    /// Decode the request from `data`; errors on missing, malformed or truncated fields.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] subscriberProcessIdentifier
        let (subscriber_process_identifier, end) =
            decode_ctx_unsigned::<u32>(data, offset, 0, "SubscribeCOVPropertyMultiple process-id")?;
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
        if next_is_context(data, offset, 2)? {
            let (value, end) = decode_ctx_unsigned::<u32>(
                data,
                offset,
                2,
                "SubscribeCOVPropertyMultiple lifetime",
            )?;
            lifetime = Some(value);
            offset = end;
        }

        // [3] maxNotificationDelay OPTIONAL
        let mut max_notification_delay = None;
        if next_is_context(data, offset, 3)? {
            let (value, end) = decode_ctx_unsigned::<u32>(
                data,
                offset,
                3,
                "SubscribeCOVPropertyMultiple max-notification-delay",
            )?;
            max_notification_delay = Some(value);
            offset = end;
        }

        // [4] listOfCovSubscriptionSpecifications — opening tag 4
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(4) {
            return Err(misplaced_tag(
                data,
                &tag,
                Some(4),
                offset,
                "SubscribeCOVPropertyMultiple expected opening tag 4",
            ));
        }
        offset = tag_end;

        let mut specs = Vec::new();
        let mut total_references = 0usize;
        loop {
            if offset >= data.len() {
                return Err(Error::missing(
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
            let (oid, end) =
                decode_ctx_object_id(data, offset, 0, "SubscribeCOVPropertyMultiple object-id")?;
            offset = end;

            // [1] listOfCovReferences — opening tag 1
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(1) {
                return Err(misplaced_tag(
                    data,
                    &tag,
                    Some(1),
                    offset,
                    "SubscribeCOVPropertyMultiple expected opening tag 1",
                ));
            }
            offset = tag_end;

            let mut refs = Vec::new();
            loop {
                if offset >= data.len() {
                    return Err(Error::missing(
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
                    return Err(misplaced_tag(
                        data,
                        &tag,
                        Some(0),
                        offset,
                        "SubscribeCOVPropertyMultiple expected opening tag 0 for property ref",
                    ));
                }
                let (prop_ref, new_off) = decode_property_reference(data, tag_end)?;
                validate_decoded_property_identifier(
                    prop_ref.property_identifier,
                    "SubscribeCOVPropertyMultiple monitored property cannot be ALL, OPTIONAL, or REQUIRED",
                )?;
                offset = new_off;
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if !tag.is_closing_tag(0) {
                    return Err(Error::decoding_kind(
                        unclosed_kind(&tag),
                        offset,
                        "SubscribeCOVPropertyMultiple expected closing tag 0",
                    ));
                }
                offset = tag_end;

                // [1] covIncrement OPTIONAL
                let mut cov_increment = None;
                if next_is_context(data, offset, 1)? {
                    let (increment, end) = decode_ctx_real(
                        data,
                        offset,
                        1,
                        "SubscribeCOVPropertyMultiple COV increment",
                    )?;
                    cov_increment = Some(increment);
                    offset = end;
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
        expect_end(data, offset, offset, "SubscribeCOVPropertyMultiple")?;

        Ok(Self {
            subscriber_process_identifier,
            issue_confirmed_notifications,
            lifetime,
            max_notification_delay,
            list_of_cov_subscription_specifications: specs,
        })
    }
}

#[path = "cov_multiple_notification.rs"]
mod notification;
pub use notification::*;

#[path = "cov_multiple_error.rs"]
mod error;
pub use error::SubscribeCOVPropertyMultipleError;

#[cfg(test)]
#[path = "cov_multiple_width_tests.rs"]
mod width_tests;

#[cfg(test)]
#[path = "cov_multiple_conformance_tests.rs"]
mod conformance_tests;

#[cfg(test)]
#[path = "cov_multiple_tests.rs"]
mod tests;
