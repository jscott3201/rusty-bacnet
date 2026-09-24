//! Encode the constructed values carried by Device `Active_COV_Subscriptions`
//! and `Active_COV_Multiple_Subscriptions`.
//!
//! Each subscription is a bare Clause 21 sequence. A BACnetLIST concatenates
//! those sequences without adding a list or per-entry wrapper.

use bacnet_types::constructed::{BACnetCOVMultipleSubscription, BACnetCOVSubscription};
use bytes::BytesMut;

use crate::{primitives, tags};

use super::{encode_object_property_reference, encode_recipient};

/// Encode one bare `BACnetCOVSubscription` sequence.
pub fn encode_cov_subscription(buf: &mut BytesMut, subscription: &BACnetCOVSubscription) {
    tags::encode_opening_tag(buf, 0);
    tags::encode_opening_tag(buf, 0);
    encode_recipient(buf, &subscription.recipient.recipient);
    tags::encode_closing_tag(buf, 0);
    primitives::encode_ctx_unsigned(buf, 1, subscription.recipient.process_identifier as u64);
    tags::encode_closing_tag(buf, 0);

    tags::encode_opening_tag(buf, 1);
    encode_object_property_reference(buf, &subscription.monitored_property_reference);
    tags::encode_closing_tag(buf, 1);

    primitives::encode_ctx_boolean(buf, 2, subscription.issue_confirmed_notifications);
    primitives::encode_ctx_unsigned(buf, 3, subscription.time_remaining as u64);
    if let Some(increment) = subscription.cov_increment {
        primitives::encode_ctx_real(buf, 4, increment);
    }
}

/// Encode a `BACnetLIST of BACnetCOVSubscription` in slice order.
pub fn encode_cov_subscription_list(buf: &mut BytesMut, subscriptions: &[BACnetCOVSubscription]) {
    for subscription in subscriptions {
        encode_cov_subscription(buf, subscription);
    }
}

/// Encode one bare `BACnetCOVMultipleSubscription` sequence: `[0]` recipient
/// process, `[1]` form, `[2]` time remaining, `[3]` maximum notification
/// delay and `[4]` the nested specifications, each an `[0]` object
/// identifier plus `[1]` references of `[0]` BACnetPropertyReference,
/// optional `[1]` REAL increment and `[2]` timestamped flag.
pub fn encode_cov_multiple_subscription(
    buf: &mut BytesMut,
    subscription: &BACnetCOVMultipleSubscription,
) {
    tags::encode_opening_tag(buf, 0);
    tags::encode_opening_tag(buf, 0);
    encode_recipient(buf, &subscription.recipient.recipient);
    tags::encode_closing_tag(buf, 0);
    primitives::encode_ctx_unsigned(buf, 1, subscription.recipient.process_identifier as u64);
    tags::encode_closing_tag(buf, 0);

    primitives::encode_ctx_boolean(buf, 1, subscription.issue_confirmed_notifications);
    primitives::encode_ctx_unsigned(buf, 2, subscription.time_remaining as u64);
    primitives::encode_ctx_unsigned(buf, 3, subscription.max_notification_delay as u64);

    tags::encode_opening_tag(buf, 4);
    for specification in &subscription.list_of_cov_subscription_specifications {
        primitives::encode_ctx_object_id(buf, 0, &specification.monitored_object_identifier);
        tags::encode_opening_tag(buf, 1);
        for reference in &specification.list_of_cov_references {
            tags::encode_opening_tag(buf, 0);
            primitives::encode_ctx_unsigned(buf, 0, reference.property_identifier.to_raw() as u64);
            if let Some(index) = reference.property_array_index {
                primitives::encode_ctx_unsigned(buf, 1, index as u64);
            }
            tags::encode_closing_tag(buf, 0);
            if let Some(increment) = reference.cov_increment {
                primitives::encode_ctx_real(buf, 1, increment);
            }
            primitives::encode_ctx_boolean(buf, 2, reference.timestamped);
        }
        tags::encode_closing_tag(buf, 1);
    }
    tags::encode_closing_tag(buf, 4);
}

/// Encode a `BACnetLIST of BACnetCOVMultipleSubscription` in slice order.
pub fn encode_cov_multiple_subscription_list(
    buf: &mut BytesMut,
    subscriptions: &[BACnetCOVMultipleSubscription],
) {
    for subscription in subscriptions {
        encode_cov_multiple_subscription(buf, subscription);
    }
}
