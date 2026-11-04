//! Live Device `Active_COV_Multiple_Subscriptions` projection (Clause 12.11;
//! Table 12-13 footnote 18).
//!
//! Each list entry is one COV-multiple context: an original BACnet client address,
//! process identifier and notification form, with one remaining lifetime and
//! one maximum notification delay, nesting its accepted references by
//! monitored object. The server COV table is the canonical owner:
//! `subscribe_multiple` refreshes the route, expiry and delay of every reference
//! in a context together. The route is delivery state, not projected identity.
use super::*;
use alloc::vec;
use bacnet_encoding::constructed::encode_cov_multiple_subscription_list;
use bacnet_types::constructed::{
    BACnetCOVMultipleSubscription, BACnetCOVReference, BACnetCOVSubscriptionSpecification,
};

/// One Multiple reference copied from the table at a sampled instant.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveCovMultipleEntry {
    context: MultipleContextKey,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    time_remaining: u32,
    max_notification_delay: u32,
    explicit_increment: Option<f32>,
    timestamped: bool,
}

impl CovTable {
    /// Copy the Multiple references live at `now`.
    ///
    /// Read-only: a reference whose context deadline has been reached is
    /// omitted without purging. Remaining lifetime is the same projection the
    /// notification Time_Remaining uses. Ordinary and Single entries belong to
    /// `Active_COV_Subscriptions` and are skipped.
    pub fn active_cov_multiple_entries(&self, now: Instant) -> Vec<ActiveCovMultipleEntry> {
        self.entries()
            .filter_map(|entry| {
                let CovSubscriptionKey::Multiple {
                    context,
                    object,
                    property,
                    index,
                } = entry.key()
                else {
                    return None;
                };
                let time_remaining = CovTimeRemaining::at(entry.expires_at, now).wire_seconds()?;
                Some(ActiveCovMultipleEntry {
                    context: context.clone(),
                    object: *object,
                    property: *property,
                    index: *index,
                    time_remaining,
                    // `subscribe_multiple` is the only admission path for a
                    // Multiple reference and always records its delay.
                    max_notification_delay: entry.max_notification_delay()?,
                    explicit_increment: entry.cov_increment,
                    timestamped: entry.timestamped,
                })
            })
            .collect()
    }
}

/// Request-local value of the selected Device's
/// `Active_COV_Multiple_Subscriptions`.
#[derive(Debug, Clone)]
pub struct ActiveCovMultipleSubscriptions {
    device: ObjectIdentifier,
    encoded: Vec<u8>,
}

impl ActiveCovMultipleSubscriptions {
    /// A stopped server services no context, so it reports none.
    pub fn stopped(device: ObjectIdentifier) -> Self {
        Self {
            device,
            encoded: Vec::new(),
        }
    }

    /// Group copied references into contexts while the caller holds the
    /// database guard.
    ///
    /// A reference whose monitored object no longer exists has been
    /// terminated by DeleteObject and is omitted; a context left without any
    /// reference is removed entirely, as is an accepted request that
    /// established no reference.
    pub fn project(
        db: &ObjectDatabase,
        device: ObjectIdentifier,
        mut entries: Vec<ActiveCovMultipleEntry>,
    ) -> Self {
        // The order is total, so an unstable sort gives the same list, and
        // links no 64-bit division on a 32-bit target, as the stable one's
        // run-length scale does.
        entries.sort_unstable_by(order);
        let mut contexts: Vec<BACnetCOVMultipleSubscription> = Vec::new();
        let mut current: Option<&MultipleContextKey> = None;
        for entry in &entries {
            let Some(object) = db.get(&entry.object) else {
                continue;
            };
            let reference = BACnetCOVReference {
                property_identifier: entry.property,
                property_array_index: entry.index,
                cov_increment: increment_in_use(
                    object,
                    entry.property,
                    entry.index,
                    entry.explicit_increment,
                ),
                timestamped: entry.timestamped,
            };
            if current != Some(&entry.context) {
                current = Some(&entry.context);
                contexts.push(BACnetCOVMultipleSubscription {
                    recipient: BACnetRecipientProcess {
                        recipient: multiple_recipient(&entry.context.recipient),
                        process_identifier: entry.context.process_id,
                    },
                    issue_confirmed_notifications: entry.context.confirmed,
                    time_remaining: entry.time_remaining,
                    max_notification_delay: entry.max_notification_delay,
                    list_of_cov_subscription_specifications: Vec::new(),
                });
            }
            let context = contexts.last_mut().expect("context pushed above");
            debug_assert_eq!(
                (context.time_remaining, context.max_notification_delay),
                (entry.time_remaining, entry.max_notification_delay),
                "a context refreshes its references together"
            );
            let specifications = &mut context.list_of_cov_subscription_specifications;
            match specifications.last_mut() {
                Some(specification)
                    if specification.monitored_object_identifier == entry.object =>
                {
                    specification.list_of_cov_references.push(reference);
                }
                _ => specifications.push(BACnetCOVSubscriptionSpecification {
                    monitored_object_identifier: entry.object,
                    list_of_cov_references: vec![reference],
                }),
            }
        }
        let mut encoded = BytesMut::new();
        // Admission holds every recipient to BACnetAddress::MAX_MAC_LEN
        // (`CovRecipient::validate`, #1156), so each context encodes.
        encode_cov_multiple_subscription_list(&mut encoded, &contexts)
            .expect("COV admission bounds recipient MACs");
        Self {
            device,
            encoded: encoded.to_vec(),
        }
    }

    /// The live value for exactly the selected Device and this property.
    pub fn resolve(
        &self,
        oid: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> Option<PropertyValue> {
        (oid == self.device && property == PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS)
            .then(|| PropertyValue::ApplicationData(self.encoded.clone()))
    }
}

/// Contexts in canonical recipient order, then process and form; within a context,
/// references in the shared object/property/index coordinate order. The table
/// identity is unique, so the order is total and deterministic.
fn order(a: &ActiveCovMultipleEntry, b: &ActiveCovMultipleEntry) -> CmpOrdering {
    let coordinates = |entry: &ActiveCovMultipleEntry| {
        (
            entry.object.object_type().to_raw(),
            entry.object.instance_number(),
            entry.property.to_raw(),
            entry.index,
        )
    };
    recipient_order(&a.context.recipient, &b.context.recipient)
        .then_with(|| a.context.process_id.cmp(&b.context.process_id))
        .then_with(|| a.context.confirmed.cmp(&b.context.confirmed))
        .then_with(|| coordinates(a).cmp(&coordinates(b)))
}

fn multiple_recipient(recipient: &CovRecipient) -> BACnetRecipient {
    BACnetRecipient::Address(match recipient {
        CovRecipient::Direct(mac) => BACnetAddress {
            network_number: 0,
            mac_address: mac.clone(),
        },
        CovRecipient::Routed(source) => BACnetAddress {
            network_number: source.network,
            mac_address: source.mac_address.clone(),
        },
    })
}

fn recipient_order(a: &CovRecipient, b: &CovRecipient) -> CmpOrdering {
    let coordinates = |recipient: &CovRecipient| match recipient {
        CovRecipient::Direct(mac) => (None, mac.clone()),
        CovRecipient::Routed(source) => (Some(source.network), source.mac_address.clone()),
    };
    coordinates(a).cmp(&coordinates(b))
}

#[cfg(test)]
mod tests;
