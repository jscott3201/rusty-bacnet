//! A Notification Forwarder's Subscribed_Recipients (Clause 12.51.9, #1049):
//! a bounded BACnetLIST of BACnetEventNotificationSubscription whose entries
//! lapse when their time runs out.
//!
//! The bundled [`NotificationForwarderObject`] keeps its list in a
//! [`SubscribedRecipients`]. An application that implements its own type-51
//! object can do the same, routing to the store the property's reads and
//! writes ([`read`](SubscribedRecipients::read) and
//! [`write`](SubscribedRecipients::write)) and the object's monotonic clock
//! hooks ([`bind_monotonic_clock`](SubscribedRecipients::bind_monotonic_clock),
//! [`advance_to`](SubscribedRecipients::advance_to) and
//! [`next_deadline`](SubscribedRecipients::next_deadline)). The server then
//! serves the list to ReadProperty, ReadPropertyMultiple and ReadRange, and
//! its AddListElement and RemoveListElement handlers edit it the way the
//! clause asks: an entry is named by its recipient and process identifier, and
//! adding one already present renews it with the other members given.
//!
//! [`NotificationForwarderObject`]: crate::notification_forwarder::NotificationForwarderObject
//!
//! # Lifetimes
//!
//! Time Remaining travels in minutes. The store keeps each entry's deadline on
//! the database's monotonic clock, the one the server's operation task uses
//! for the Access Door's pulse relock, and serves the whole minutes left,
//! rounded up, so a live entry never reads as zero. An entry leaves the list
//! once its deadline passes: reads and writes skip it from then on, and the
//! server's operation task drops it at its deadline (through
//! `advance_monotonic_time_internal`). With no clock bound the store counts
//! the time [`advance_by`](SubscribedRecipients::advance_by) gives it, and
//! binding a clock later keeps the time each entry has left.
//!
//! Clause 12.51.9 asks for the list to survive a restart. The store itself
//! lives in memory; the forwarder object saves it through a
//! [`NotificationForwarderPersistence`] and restores it with
//! [`write`](SubscribedRecipients::write).
//!
//! [`NotificationForwarderPersistence`]: crate::notification_forwarder::NotificationForwarderPersistence

use crate::prelude::*;
use alloc::sync::Arc;
use core::fmt;
use core::time::Duration;

use bacnet_encoding::constructed::{
    decode_event_notification_subscription, encode_event_notification_subscription_list,
};
use bacnet_types::constructed::{BACnetEventNotificationSubscription, BACnetRecipient};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use crate::common;
use crate::traits::MonotonicClock;

/// The most entries a [`SubscribedRecipients`] holds.
///
/// An entry naming a device encodes in at most 17 octets, and one naming an
/// address with the longest MAC this stack carries
/// ([`BACnetAddress::MAX_MAC_LEN`], 18 octets) in at most 37. A full list
/// therefore takes at most 1,184 octets, and a ReadProperty of it fits B/IP's
/// 1476-octet APDU unsegmented. It matches the Notification Class
/// Recipient_List cap ([`MAX_RECIPIENT_LIST_DESTINATIONS`]).
///
/// A write that would leave more entries fails with RESOURCES /
/// NO_SPACE_TO_WRITE_PROPERTY naming the first entry past the cap, which
/// AddListElement reports as NO_SPACE_TO_ADD_LIST_ELEMENT at the request
/// element that brought it (Clause 15.1.1.3). A renewal never counts against
/// the cap.
///
/// [`BACnetAddress::MAX_MAC_LEN`]: bacnet_types::constructed::BACnetAddress::MAX_MAC_LEN
/// [`MAX_RECIPIENT_LIST_DESTINATIONS`]: crate::notification_class::MAX_RECIPIENT_LIST_DESTINATIONS
pub const MAX_SUBSCRIBED_RECIPIENTS: usize = 32;

/// The longest Time Remaining, in minutes, the store accepts: 1,440 (a day).
///
/// Clause 12.51.9 requires a forwarder to take 1 through 1,440 and leaves
/// longer subscriptions to the implementation. This one refuses them, and
/// refuses 0, which no entry may hold, with PROPERTY / VALUE_OUT_OF_RANGE
/// naming the entry.
pub const MAX_SUBSCRIPTION_MINUTES: u32 = 1440;

const NANOS_PER_MINUTE: u128 = 60_000_000_000;

/// One held entry, its lifetime as a monotonic deadline.
#[derive(Clone)]
struct Entry {
    recipient: BACnetRecipient,
    process_identifier: u32,
    issue_confirmed_notifications: bool,
    expires_at: Duration,
}

impl Entry {
    /// Whether `other` is the same entry: the same recipient and process
    /// identifier, whatever its other members.
    fn names_the_same(&self, other: &Entry) -> bool {
        self.recipient == other.recipient && self.process_identifier == other.process_identifier
    }

    fn is_live(&self, now: Duration) -> bool {
        self.expires_at > now
    }

    /// The entry as served at `now`, with the whole minutes left rounded up.
    fn served(&self, now: Duration) -> BACnetEventNotificationSubscription {
        let minutes = self
            .expires_at
            .saturating_sub(now)
            .as_nanos()
            .div_ceil(NANOS_PER_MINUTE);
        BACnetEventNotificationSubscription {
            recipient: self.recipient.clone(),
            process_identifier: self.process_identifier,
            issue_confirmed_notifications: self.issue_confirmed_notifications,
            time_remaining: u32::try_from(minutes).unwrap_or(u32::MAX),
        }
    }
}

/// A Notification Forwarder's Subscribed_Recipients: at most
/// [`MAX_SUBSCRIBED_RECIPIENTS`] entries, each lapsing when its Time
/// Remaining runs out. See the [module documentation](self).
#[derive(Clone, Default)]
pub struct SubscribedRecipients {
    entries: Vec<Entry>,
    monotonic_clock: Option<Arc<MonotonicClock>>,
    /// Elapsed time for a store with no bound clock, advanced by
    /// [`Self::advance_by`].
    logical_now: Duration,
}

impl fmt::Debug for SubscribedRecipients {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubscribedRecipients")
            .field("subscriptions", &self.subscriptions())
            .finish_non_exhaustive()
    }
}

impl SubscribedRecipients {
    /// An empty list.
    pub fn new() -> Self {
        Self::default()
    }

    fn now(&self) -> Duration {
        self.monotonic_clock
            .as_ref()
            .map_or(self.logical_now, |clock| clock())
    }

    /// The store's current time: the bound clock's reading, or the time
    /// [`advance_by`](Self::advance_by) has counted.
    pub(crate) fn current_time(&self) -> Duration {
        self.now()
    }

    /// The live entries in list order, each with the whole minutes it has
    /// left, rounded up.
    pub fn subscriptions(&self) -> Vec<BACnetEventNotificationSubscription> {
        let now = self.now();
        self.entries
            .iter()
            .filter(|entry| entry.is_live(now))
            .map(|entry| entry.served(now))
            .collect()
    }

    /// The property value: the live entries, encoded back to back in
    /// `PropertyValue::ApplicationData`.
    pub fn read(&self) -> PropertyValue {
        let mut buf = BytesMut::new();
        // Every entry arrived through `write`, whose decoder holds recipients
        // to BACnetAddress::MAX_MAC_LEN (#1156), so each one encodes.
        encode_event_notification_subscription_list(&mut buf, &self.subscriptions())
            .expect("stored recipients fit BACnetAddress::MAX_MAC_LEN");
        PropertyValue::ApplicationData(buf.to_vec())
    }

    /// Replace the list with a written one, all or nothing.
    ///
    /// Only the framed form in `PropertyValue::ApplicationData` is a value;
    /// anything else, or an entry that does not decode, fails with PROPERTY /
    /// INVALID_DATA_TYPE. An entry whose Time Remaining is 0 or past
    /// [`MAX_SUBSCRIPTION_MINUTES`] fails with PROPERTY / VALUE_OUT_OF_RANGE,
    /// and the first entry past [`MAX_SUBSCRIBED_RECIPIENTS`] with RESOURCES /
    /// NO_SPACE_TO_WRITE_PROPERTY; both name the entry's position in the
    /// written list.
    ///
    /// Each entry's Time Remaining starts its lifetime over, except that an
    /// entry written as it reads now keeps its deadline, and so does one
    /// written with a minute more, which is how it read if a minute boundary
    /// fell between the read and this write. A list read and written back, or
    /// edited by the list services, so leaves the entries it doesn't change
    /// alone, rather than stretching each to its rounded-up minute. Two
    /// entries naming the same recipient and process are one entry, which
    /// takes the later one's members.
    pub fn write(&mut self, value: PropertyValue) -> Result<(), Error> {
        let PropertyValue::ApplicationData(bytes) = value else {
            return Err(common::invalid_data_type_error());
        };
        let now = self.now();
        let mut written: Vec<Entry> = Vec::new();
        let mut offset = 0;
        let mut index = 0;
        while offset < bytes.len() {
            let (subscription, next) = decode_event_notification_subscription(&bytes, offset)
                .map_err(|_| common::invalid_data_type_error())?;
            offset = next;
            if !(1..=MAX_SUBSCRIPTION_MINUTES).contains(&subscription.time_remaining) {
                return Err(common::at_list_element(
                    common::value_out_of_range_error(),
                    index,
                ));
            }
            let entry = self.entry_for(subscription, now);
            match written.iter().position(|held| held.names_the_same(&entry)) {
                Some(held) => written[held] = entry,
                None if written.len() == MAX_SUBSCRIBED_RECIPIENTS => {
                    return Err(common::at_list_element(
                        common::protocol_error(
                            ErrorClass::RESOURCES,
                            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
                        ),
                        index,
                    ));
                }
                None => written.push(entry),
            }
            index += 1;
        }
        self.entries = written;
        Ok(())
    }

    /// The entry a written `subscription` becomes at `now`. A live entry with
    /// the same recipient, process and confirmation flag keeps its deadline
    /// when the written minutes equal what it serves now or one more.
    fn entry_for(&self, subscription: BACnetEventNotificationSubscription, now: Duration) -> Entry {
        let lifetime = Duration::from_secs(u64::from(subscription.time_remaining) * 60);
        let expires_at = self
            .entries
            .iter()
            .find(|entry| {
                if !entry.is_live(now) {
                    return false;
                }
                let served = entry.served(now);
                served.recipient == subscription.recipient
                    && served.process_identifier == subscription.process_identifier
                    && served.issue_confirmed_notifications
                        == subscription.issue_confirmed_notifications
                    && (served.time_remaining..=served.time_remaining.saturating_add(1))
                        .contains(&subscription.time_remaining)
            })
            .map_or_else(|| now.saturating_add(lifetime), |entry| entry.expires_at);
        Entry {
            recipient: subscription.recipient,
            process_identifier: subscription.process_identifier,
            issue_confirmed_notifications: subscription.issue_confirmed_notifications,
            expires_at,
        }
    }

    /// Bind the database's monotonic clock, from the object's
    /// `bind_monotonic_clock_internal`. Each held entry keeps the time it has
    /// left: its deadline moves from the old time base to the new one, so a
    /// list written or restored before the object joins a database lapses on
    /// time.
    pub fn bind_monotonic_clock(&mut self, clock: Option<Arc<MonotonicClock>>) {
        let before = self.now();
        self.monotonic_clock = clock;
        let after = self.now();
        for entry in &mut self.entries {
            let left = entry.expires_at.saturating_sub(before);
            entry.expires_at = after.saturating_add(left);
        }
    }

    /// Drop every entry whose deadline is at or before `now`, a monotonic
    /// instant, from the object's `advance_monotonic_time_internal`. `true`
    /// when one was dropped.
    pub fn advance_to(&mut self, now: Duration) -> bool {
        let held = self.entries.len();
        self.entries.retain(|entry| entry.is_live(now));
        self.entries.len() != held
    }

    /// Count `elapsed` on a store with no bound clock, then drop what has
    /// lapsed, as [`Self::advance_to`] does. `true` when an entry was dropped.
    pub fn advance_by(&mut self, elapsed: Duration) -> bool {
        self.logical_now = self.logical_now.saturating_add(elapsed);
        self.advance_to(self.now())
    }

    /// The earliest deadline among the held entries, for the object's
    /// `next_monotonic_deadline_internal`.
    pub fn next_deadline(&self) -> Option<Duration> {
        self.entries.iter().map(|entry| entry.expires_at).min()
    }
}

#[cfg(test)]
#[path = "subscribed_recipients_tests.rs"]
mod tests;
