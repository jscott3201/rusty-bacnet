//! What one change reports: for each ordinary or Single subscription to an
//! object, whether a notification goes out now, and that notification
//! (Clause 13.1).
//!
//! A host calls [`evaluate`] with the object borrowed under the guard it
//! read it under and the [`CovTable`] borrowed beside it, so the tickets it
//! issues follow the order the observations were captured in (#826).
//!
//! - A whole-object subscription reports when its leading value
//!   (Present_Value, or an Access Point's Access_Event) moves by the
//!   increment in use, when Status_Flags change, or when another of its
//!   type's Table 13-1 triggers changes. The type's other values ride
//!   along ([`reported`](super::reported)).
//! - A property subscription (Table 13-1a) measures its value with its own
//!   increment, else the object's for a numeric Present_Value, else any
//!   change, and reports a Status_Flags change too. Value_Source on a
//!   commandable or source-tracking object takes the Table 13-1a-2 report
//!   ([`value_source`]).
//! - Each compares with the observation the subscription's last delivered
//!   notification established, not one merely sent.
//!
//! A confirmed subscription whose report is outstanding is skipped, and its
//! report's Ack owes it a fresh look ([`CovTable::delivered`]). One holding
//! off after a failure is skipped too; nothing looks again when the
//! hold-off ends, and its next change reports. The input's [`EventBudget`] bounds what one
//! change sends; a report past it is counted
//! ([`CovCounters::notifications_throttled_fanout`](super::CovCounters::notifications_throttled_fanout))
//! and dropped.

use alloc::vec;
use alloc::vec::Vec;

use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::traits::{BACnetObject, CovReportedProperty};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_time::Instant;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};

use super::flags::PreparedFlags;
use super::prepare::prepare_value;
use super::reported::{PreparedReported, lead};
use super::value_source::{self, PreparedValueSource};
use super::{
    CovNotificationKind, CovObservation, CovSample, CovSubscriptionSnapshot, CovTable,
    CovTimeRemaining, EventBudget, PreparedCovCompletion, Token,
};

#[cfg(test)]
mod tests;

/// Octets an UnconfirmedCOVNotification APDU adds to its parameters.
const UNCONFIRMED_HEADER: usize = 2;
/// Octets an unsegmented ConfirmedCOVNotification APDU adds to its
/// parameters.
const CONFIRMED_HEADER: usize = 4;

/// The parts of one evaluation besides the table, the object and the
/// subscriptions.
#[derive(Debug, Clone, Copy)]
pub struct Evaluation {
    /// The Device the notifications name as their initiating device.
    pub device: ObjectIdentifier,
    /// Report every subscription whatever its baseline: an initial
    /// notification, or a Life Safety change whose Status_Flags moved.
    pub force: bool,
}

/// One notification [`evaluate`] decided to send.
#[derive(Debug)]
pub struct CovReport<L> {
    /// Where the subscription it reports to sits in the evaluated slice.
    pub index: usize,
    /// The COVNotification service parameters (Clauses 13.6 and 13.7).
    pub params: Bytes,
    /// Its end comes back through [`CovTable::delivered`]: an unconfirmed
    /// report's send, a confirmed report's answer.
    pub token: Token,
    /// What the host's admission gave the report.
    pub lease: L,
}

/// Decide what a change of `object` reports to `subscriptions`, all of them
/// ordinary or Single subscriptions to it, and hand each report to the host
/// in their order.
///
/// Each report first takes `admit`'s lease, given the subscription and the
/// octets of its APDU. The host makes the lease of whatever it needs to send
/// the report (an in-flight slot and an invoke ID for a confirmed one, room
/// in an outbox); a report that gets none is dropped, its budget unspent,
/// and the host may owe its subscription a later look. Every admitted
/// report then spends the budget for its APDU and takes a ticket: a
/// confirmed one begins its flight ([`CovTable::begin_confirmed`]), and an
/// unconfirmed one waits for its send to end. The host sends each, and
/// reports how it ended through [`CovTable::delivered`].
pub fn evaluate<L>(
    table: &mut CovTable,
    object: &dyn BACnetObject,
    subscriptions: &[CovSubscriptionSnapshot],
    evaluation: Evaluation,
    budget: &mut EventBudget,
    now: Instant,
    mut admit: impl FnMut(&CovSubscriptionSnapshot, usize) -> Option<L>,
) -> Vec<CovReport<L>> {
    let mut reports = Vec::new();
    if budget.is_exhausted() {
        return reports;
    }
    debug_assert!(
        subscriptions.iter().all(|sub| {
            sub.notification_kind == CovNotificationKind::Single
                && sub.monitored_object_identifier == object.object_identifier()
        }),
        "evaluate takes ordinary and Single subscriptions to `object` only"
    );
    // Once per evaluation, not per report: both guards are held.
    table.prune_pending(now);
    let ordinary = subscriptions
        .iter()
        .any(|sub| sub.monitored_property.is_none())
        .then(|| Ordinary::capture(object))
        .flatten();
    for (index, sub) in subscriptions.iter().enumerate() {
        let decided = match sub.monitored_property {
            None => ordinary
                .as_ref()
                .filter(|ordinary| ordinary.reports(sub, evaluation.force))
                .map(Ordinary::report),
            Some(property) => property_report(object, sub, property, evaluation.force),
        };
        let Some((values, observation)) = decided else {
            continue;
        };
        // A confirmed reference with an outstanding report waits for it (#896).
        let Some(time_remaining) = table
            .remaining_lifetime(sub, now)
            .and_then(CovTimeRemaining::wire_seconds)
            .filter(|_| table.confirmed_idle(sub, now))
        else {
            continue;
        };
        if budget.is_exhausted() {
            table.counts.notifications_throttled_fanout += 1;
            continue;
        }
        let mut params = BytesMut::new();
        let notification = COVNotificationRequest {
            subscriber_process_identifier: sub.subscriber_process_identifier,
            initiating_device_identifier: evaluation.device,
            monitored_object_identifier: sub.monitored_object_identifier,
            time_remaining,
            list_of_values: values,
        };
        // A value the encoder refuses (a legacy-form Event_Parameters value
        // that wouldn't decode back, #2376) can't be reported.
        if notification.encode(&mut params).is_err() {
            continue;
        }
        let octets = params.len()
            + if sub.issue_confirmed_notifications {
                CONFIRMED_HEADER
            } else {
                UNCONFIRMED_HEADER
            };
        let Some(lease) = admit(sub, octets) else {
            continue;
        };
        let Some(completion) = table.prepare_completion(sub) else {
            continue;
        };
        if !budget.try_consume(octets) {
            table.counts.notifications_throttled_fanout += 1;
            continue;
        }
        let token = match completion {
            PreparedCovCompletion::Confirmed(_) => {
                match table.begin_confirmed(completion, [(sub, observation)], now) {
                    Ok(token) => token,
                    // Not reached: the same borrow found the reference
                    // current and idle above.
                    Err(_) => {
                        budget.refund(octets);
                        continue;
                    }
                }
            }
            PreparedCovCompletion::Unconfirmed(ticket) => {
                table.begin_unconfirmed(ticket, sub, observation)
            }
        };
        reports.push(CovReport {
            index,
            params: params.freeze(),
            token,
            lease,
        });
    }
    reports
}

/// The one observation every whole-object subscription to the object
/// shares, captured once.
struct Ordinary {
    lead: CovReportedProperty,
    values: Vec<BACnetPropertyValue>,
    observation: CovObservation,
    /// The object's COV_Increment, for a subscription that names none.
    increment: Option<f64>,
}

impl Ordinary {
    /// The leading value, Status_Flags and the Table 13-1 extras, in that
    /// order. `None` when any of them can't be read or bounded.
    fn capture(object: &dyn BACnetObject) -> Option<Self> {
        // Present_Value leads, except on an Access Point (Access_Event).
        let lead = lead(object.object_identifier().object_type());
        let leading = object.read_property(lead.property(), None).ok()?;
        let sample = CovSample::new(&leading).ok()?;
        let flags = PreparedFlags::read(object).ok()?;
        let reported = PreparedReported::read(object).ok()?;
        let mut buf = BytesMut::new();
        encode_property_value(&mut buf, sample.value()).ok()?;
        let mut values = vec![BACnetPropertyValue {
            property_identifier: lead.property(),
            property_array_index: None,
            value: buf.to_vec(),
            priority: None,
        }];
        if let Some(encoded) = &flags.encoded {
            values.push(BACnetPropertyValue {
                property_identifier: PropertyIdentifier::STATUS_FLAGS,
                property_array_index: None,
                value: encoded.clone(),
                priority: None,
            });
        }
        // Table 13-1 extras follow the leading value and flags, in the
        // object's order.
        values.extend(reported.values);
        Some(Self {
            lead,
            values,
            observation: flags.observation(sample).with_triggers(reported.triggers),
            increment: object.cov().and_then(|cov| cov.cov_increment()),
        })
    }

    /// Whether `sub` reports this observation.
    fn reports(&self, sub: &CovSubscriptionSnapshot, force: bool) -> bool {
        let last = sub.last_notified_observation.as_ref();
        force
            || (self.lead.triggers()
                && CovTable::should_notify(
                    sub,
                    Some(self.observation.sample()),
                    sub.cov_increment.map(f64::from).or(self.increment),
                ))
            || self.observation.flags_changed(last)
            || self.observation.triggers_changed(last)
    }

    fn report(&self) -> (Vec<BACnetPropertyValue>, CovObservation) {
        (self.values.clone(), self.observation.clone())
    }
}

/// What a property subscription to `property` reports, if anything: its
/// value and Status_Flags, or the Table 13-1a-2 report.
fn property_report(
    object: &dyn BACnetObject,
    sub: &CovSubscriptionSnapshot,
    property: PropertyIdentifier,
    force: bool,
) -> Option<(Vec<BACnetPropertyValue>, CovObservation)> {
    let flags = PreparedFlags::read(object).ok()?;
    let last = sub.last_notified_observation.as_ref();
    if value_source::applies(object, property) {
        if sub.monitored_property_array_index.is_some() {
            return None;
        }
        let prepared = PreparedValueSource::read(object, &flags).ok()?;
        if !force && !prepared.reports(last) {
            return None;
        }
        return Some((prepared.values(), prepared.observation));
    }
    let index = sub.monitored_property_array_index;
    let prepared = if property == PropertyIdentifier::STATUS_FLAGS {
        flags.selected(object, index)
    } else {
        object
            .read_property(property, index)
            .and_then(|value| prepare_value(object, property, index, sub.cov_increment, &value))
    }
    .ok()?;
    let observation = flags.observation(prepared.sample.clone());
    if !force
        && !prepared.reports(last.map(CovObservation::sample))
        && !observation.flags_changed(last)
    {
        return None;
    }
    let mut values = vec![BACnetPropertyValue {
        property_identifier: property,
        property_array_index: index,
        value: prepared.encoded,
        priority: None,
    }];
    if property != PropertyIdentifier::STATUS_FLAGS
        && let Some(encoded) = flags.encoded
    {
        values.push(BACnetPropertyValue {
            property_identifier: PropertyIdentifier::STATUS_FLAGS,
            property_array_index: None,
            value: encoded,
            priority: None,
        });
    }
    Some((values, observation))
}
