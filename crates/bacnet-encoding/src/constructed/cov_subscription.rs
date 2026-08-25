//! Encode the constructed value carried by Device `Active_COV_Subscriptions`.
//!
//! Each subscription is a bare Clause 21 sequence. A BACnetLIST concatenates
//! those sequences without adding a list or per-entry wrapper.

use bacnet_types::constructed::BACnetCOVSubscription;
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
