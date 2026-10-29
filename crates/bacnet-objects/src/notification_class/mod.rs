//! NotificationClass object: priorities, acknowledgment requirements, and recipients.
//!
//! # Recipient-list day/time convention
//!
//! `RECIPIENT_LIST` entries are `BACnetDestination` notification destinations.
//! Their `valid_days` is a [`DaysOfWeek`] (Monday first, Clause 21) and their
//! `transitions` an [`EventTransitionBits`]. The recipient filters take the
//! current day as a `DaysOfWeek` flag, which [`local_day_and_time`] derives.
//! Both bit strings convert to their MSB-first Clause 20.2.10 wire octets
//! through `to_bacnet`/`from_bacnet`.
//!
//! `from_time`/`to_time` are BACnet `Time` values interpreted in the device's
//! *local* time, derived from the wall clock plus the Device object's
//! `UTC_Offset` property (signed minutes) at the sender. A window with
//! `to_time < from_time` (e.g. 22:00–02:00) crosses midnight and is active
//! outside the `[from, to]` interval; see `time_in_window`.
//!
//! # Restarts
//!
//! Clause 12.21.8 asks for the Recipient_List to survive a restart. A class
//! built with [`NotificationClass::with_persistence`] saves a written list in
//! a [`NotificationClassPersistence`] before serving it, and restores it when
//! built again; one built with [`NotificationClass::new`] keeps the list in
//! memory only. A written, saved list wins over the destinations the
//! application configures with [`add_destination`], which are never saved.
//! See the `saving` module for when saves run.
//!
//! [`add_destination`]: NotificationClass::add_destination

use crate::prelude::*;
use alloc::borrow::Cow;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use bacnet_types::enums::{EventState, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags, Time};

use crate::common::{self, read_common_properties};
use crate::database::ObjectDatabase;
use crate::durable::DurableWrites;
use crate::event::EventTransition;
use crate::traits::BACnetObject;

mod enrollment_summary;
mod metadata;
mod persistence;
pub(crate) mod recipient_list;
mod saving;
#[doc(hidden)]
pub use enrollment_summary::{
    EnrollmentSummaryClassProjection, EnrollmentSummaryClassProjectionError,
    resolve_enrollment_summary_class_internal,
};
#[cfg(feature = "std")]
pub use persistence::FileNotificationClassPersistence;
pub use persistence::{NotificationClassPersistence, NotificationClassSnapshot};
pub use recipient_list::MAX_RECIPIENT_LIST_DESTINATIONS;

/// BACnet NotificationClass object.
///
/// Stores notification routing configuration: which priorities, acknowledgement
/// requirements, and recipient destinations apply to event notifications
/// referencing this class number.
pub struct NotificationClass {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
    reliability: Reliability,
    /// The notification class number.
    pub notification_class: u32,
    /// Priority: [TO_OFFNORMAL, TO_FAULT, TO_NORMAL]. Default [255, 255, 255].
    pub priority: [u8; 3],
    /// Transitions whose notifications require acknowledgment. Default empty.
    pub ack_required: EventTransitionBits,
    /// Recipient list, at most [`MAX_RECIPIENT_LIST_DESTINATIONS`] long.
    recipient_list: Vec<BACnetDestination>,
    /// Where a written Recipient_List is saved, with persistence.
    storage: Option<saving::Storage>,
    /// Recipient_List writes taken, so a staged write can tell whether
    /// another came between.
    list_writes: u64,
    /// A write set Recipient_List, now or before a restart, so storage keeps
    /// it and configured destinations no longer apply.
    recipient_list_written: bool,
}

impl NotificationClass {
    /// Create a new NotificationClass object. Its Recipient_List is kept in
    /// memory only; see [`with_persistence`](Self::with_persistence).
    ///
    /// The `notification_class` number defaults to the instance number.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
            reliability: Reliability::NO_FAULT_DETECTED,
            notification_class: instance,
            priority: [255, 255, 255],
            ack_required: EventTransitionBits::empty(),
            recipient_list: Vec::new(),
            storage: None,
            list_writes: 0,
            recipient_list_written: false,
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Add a destination to the recipient list.
    ///
    /// Refuses, as a network write would, an address recipient whose MAC is
    /// longer than [`BACnetAddress::MAX_MAC_LEN`] octets (PROPERTY /
    /// INVALID_DATA_TYPE, #1124) and a destination past the
    /// [`MAX_RECIPIENT_LIST_DESTINATIONS`] cap (RESOURCES /
    /// NO_SPACE_TO_WRITE_PROPERTY).
    ///
    /// This configures the list the application starts with; it is not
    /// saved. On a class [`with_persistence`](Self::with_persistence) whose
    /// storage holds a written Recipient_List
    /// ([`recipient_list_saved`](Self::recipient_list_saved)), the saved list
    /// wins: the destination is checked but not added.
    ///
    /// [`BACnetAddress::MAX_MAC_LEN`]: bacnet_types::constructed::BACnetAddress::MAX_MAC_LEN
    pub fn add_destination(&mut self, dest: BACnetDestination) -> Result<(), Error> {
        recipient_list::check_added(&dest)?;
        if self.recipient_list_saved() {
            log_debug!(
                class = %self.oid,
                "Saved Recipient_List kept over a configured destination"
            );
            return Ok(());
        }
        if self.recipient_list.len() >= MAX_RECIPIENT_LIST_DESTINATIONS {
            return Err(recipient_list::no_space_error());
        }
        self.recipient_list.push(dest);
        Ok(())
    }

    /// The Recipient_List destinations, in list order.
    pub fn recipient_list(&self) -> &[BACnetDestination] {
        &self.recipient_list
    }
}

impl BACnetObject for NotificationClass {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        // Table 12-24 has no Out_Of_Service (#1064), and Clause 12.21 holds the
        // OUT_OF_SERVICE flag FALSE.
        if let Some(result) =
            read_common_properties!(self, property, array_index, no_out_of_service)
        {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::NOTIFICATION_CLASS.to_raw(),
            )),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(EventState::NORMAL.to_raw()))
            }
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => {
                Ok(PropertyValue::Unsigned(self.notification_class as u64))
            }
            p if p == PropertyIdentifier::PRIORITY => match array_index {
                Some(0) => Ok(PropertyValue::Unsigned(3)),
                Some(idx) if (1..=3).contains(&idx) => Ok(PropertyValue::Unsigned(
                    self.priority[(idx - 1) as usize] as u64,
                )),
                None => Ok(PropertyValue::List(vec![
                    PropertyValue::Unsigned(self.priority[0] as u64),
                    PropertyValue::Unsigned(self.priority[1] as u64),
                    PropertyValue::Unsigned(self.priority[2] as u64),
                ])),
                _ => Err(common::invalid_array_index_error()),
            },
            p if p == PropertyIdentifier::ACK_REQUIRED => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.ack_required.to_bacnet()],
            }),
            p if p == PropertyIdentifier::RECIPIENT_LIST => {
                // Full ASN.1 framing: BACnetLIST of BACnetDestination — each
                // entry a 7-element application-tagged SEQUENCE with the
                // recipient discriminated by context tag (device [0]
                // primitive / address [1] constructed), Clause 12.21 + 21.
                let mut buf = bytes::BytesMut::new();
                bacnet_encoding::constructed::encode_destination_list(
                    &mut buf,
                    &self.recipient_list,
                )?;
                Ok(PropertyValue::ApplicationData(buf.to_vec()))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::NOTIFICATION_CLASS {
            if let PropertyValue::Unsigned(v) = value {
                self.notification_class = common::u64_to_u32(v)?;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::RECIPIENT_LIST {
            // Indexed (array-element) access: Recipient_List is a BACnetLIST
            // (Table 12-24), not an array, so Clause 12.1.5.2 makes ReadRange
            // the only positional access. The RP/RPM/WP/WPM handlers gate
            // indexed access to PROPERTY / PROPERTY_IS_NOT_AN_ARRAY
            // (Clause 15.5.1.3 / 15.9.1.3) via `is_array_property`; mirror
            // the same classification here for direct object-layer calls.
            if array_index.is_some() {
                return Err(common::property_is_not_an_array_error());
            }
            return self.write_recipient_list(value);
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            array_index,
        ))
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn advance_monotonic_time_internal(&mut self, now: core::time::Duration) -> bool {
        self.expire_staged_write(now);
        false
    }

    fn durable_writes_internal(&mut self) -> Option<&mut dyn DurableWrites> {
        Some(self)
    }
}

/// Convert a `Time` to centiseconds (hundredths of a second since midnight).
fn time_to_centiseconds(t: &Time) -> u32 {
    let h = if t.hour == Time::UNSPECIFIED {
        0
    } else {
        t.hour as u32
    };
    let m = if t.minute == Time::UNSPECIFIED {
        0
    } else {
        t.minute as u32
    };
    let s = if t.second == Time::UNSPECIFIED {
        0
    } else {
        t.second as u32
    };
    let cs = if t.hundredths == Time::UNSPECIFIED {
        0
    } else {
        t.hundredths as u32
    };
    h * 360_000 + m * 6_000 + s * 100 + cs
}

/// Check if `current` falls within the `[from, to]` time window.
///
/// If either bound has an unspecified hour (0xFF), the window is treated as
/// "all day". A window whose `to` is earlier than its `from` (e.g.
/// 22:00–02:00) crosses midnight: it is active from `from` up to midnight and
/// again from midnight up to `to`, i.e. when `current >= from || current <= to`.
/// This matches the ASHRAE 135-2020 reading that `To_Time` is the end of the
/// active period; a wrap-around pair denotes an overnight schedule.
fn time_in_window(current: &Time, from: &Time, to: &Time) -> bool {
    if from.hour == Time::UNSPECIFIED || to.hour == Time::UNSPECIFIED {
        return true;
    }
    let cur = time_to_centiseconds(current);
    let from_cs = time_to_centiseconds(from);
    let to_cs = time_to_centiseconds(to);
    if to_cs < from_cs {
        // Overnight window crossing midnight.
        cur >= from_cs || cur <= to_cs
    } else {
        cur >= from_cs && cur <= to_cs
    }
}

/// Derive the local day of the week and time of day for recipient filtering.
///
/// `utc_secs` is seconds since the Unix epoch (1970-01-01, a Thursday).
/// `utc_offset_minutes` is the Device object's `UTC_Offset` (signed minutes
/// west of UTC); 0 keeps UTC. The day comes back as the single matching
/// [`DaysOfWeek`] flag: the `+3` makes Monday day 0 because the epoch was a
/// Thursday. The returned `Time` is the local time of day (hundredths are
/// supplied by the caller via `subsec`).
pub fn local_day_and_time(utc_secs: u64, utc_offset_minutes: i32) -> (DaysOfWeek, Time) {
    // BACnet UTC_Offset is signed minutes west of UTC, so local standard time
    // subtracts it. Saturation only affects values close to the Unix epoch.
    let local_secs = utc_secs.saturating_add_signed(-i64::from(utc_offset_minutes) * 60);
    let dow = (local_secs / 86400 + 3) % 7;
    let today = DaysOfWeek::from_bits_truncate(1 << dow);
    let day_secs = (local_secs % 86400) as u32;
    let current_time = Time {
        hour: (day_secs / 3600) as u8,
        minute: ((day_secs % 3600) / 60) as u8,
        second: (day_secs % 60) as u8,
        hundredths: 0,
    };
    (today, current_time)
}

/// Resolve the NotificationClass object whose `Notification_Class` property
/// equals `notification_class`.
///
/// Tries a direct OID lookup first (instance == notification_class is the
/// common case), then falls back to scanning every NotificationClass object.
/// Returns `None` when no matching class is configured.
fn find_notification_class(
    db: &ObjectDatabase,
    notification_class: u32,
) -> Option<&dyn BACnetObject> {
    // Try direct OID lookup first (instance == notification_class is the common case)
    if let Ok(nc_oid) = ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, notification_class)
        && let Some(obj) = db.get(&nc_oid)
        && matches!(
            obj.read_property(PropertyIdentifier::NOTIFICATION_CLASS, None),
            Ok(PropertyValue::Unsigned(n)) if n as u32 == notification_class
        )
    {
        return Some(obj);
    }

    // Fall back to scanning all NotificationClass objects
    db.find_by_type(ObjectType::NOTIFICATION_CLASS)
        .iter()
        .find_map(|oid| {
            let obj = db.get(oid)?;
            match obj.read_property(PropertyIdentifier::NOTIFICATION_CLASS, None) {
                Ok(PropertyValue::Unsigned(n)) if n as u32 == notification_class => Some(obj),
                _ => None,
            }
        })
}

/// Resolve the per-transition `Priority` and `Ack_Required` for an event
/// notification from the referenced NotificationClass.
///
/// Per ASHRAE 135-2020 Clause 12.21, the `Priority` and `Ack_Required`
/// projected into an `EventNotification` come from the NotificationClass
/// referenced by the event-generating object's `Notification_Class` property,
/// selected by the transition coordinate (TO_OFFNORMAL, TO_FAULT, or
/// TO_NORMAL). `Priority` is a 3-element array ordered
/// `[TO_OFFNORMAL, TO_FAULT, TO_NORMAL]`, indexed by [`EventTransition::index`];
/// `Ack_Required` is a `BACnetEventTransitionBits` string, tested with
/// [`EventTransition::bit_mask`].
///
/// When no NotificationClass matches the given number (the object's
/// `Notification_Class` was never configured or points at a missing class),
/// the spec leaves the projection undefined; we fall back to the BACnet
/// defaults — `Priority = 255` (lowest) and `Ack_Required = false`.
///
/// Those defaults no longer reach the wire on the server's send path. A class
/// that does not exist also names no recipients, and the sender distributes
/// nothing when the recipient set is empty, so the fallback survives only for
/// direct callers of this function.
pub fn resolve_transition_priority_ack(
    db: &ObjectDatabase,
    notification_class: u32,
    transition: EventTransition,
) -> (u8, bool) {
    let Some(nc) = find_notification_class(db, notification_class) else {
        return (255, false);
    };
    let idx = transition.index();

    // PRIORITY is a 3-element array; index 0 is the array length, 1..=3 the
    // per-transition values. Read the slot directly, defaulting to 255 when
    // the property is absent or malformed (matches the NotificationClass
    // default and the missing-class fallback).
    let priority = nc
        .read_property(PropertyIdentifier::PRIORITY, Some(idx as u32 + 1))
        .ok()
        .and_then(|v| match v {
            PropertyValue::Unsigned(n) => Some(n as u8),
            _ => None,
        })
        .unwrap_or(255);

    let ack_required = match nc.read_property(PropertyIdentifier::ACK_REQUIRED, None) {
        Ok(PropertyValue::BitString { data, .. }) => {
            EventTransitionBits::from_bacnet(&data).intersects(transition.bit_mask())
        }
        _ => false,
    };

    (priority, ack_required)
}

/// The complete outcome of looking up and selecting Notification Class recipients.
///
/// Broadcast is not an outcome. It is represented only by an address inside
/// [`Matched`](Self::Matched), after that configured destination passes the
/// day, time, and transition filters. Device recipients are also successful
/// matches here; resolving them to network addresses is a later routing step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipientLookupOutcome {
    /// No Notification Class has the requested class number.
    NotificationClassMissing,
    /// The class exists, but its recipient-list property could not be read.
    RecipientListUnavailable,
    /// The complete recipient-list value could not be decoded.
    RecipientListInvalid,
    /// The class serves more than [`MAX_RECIPIENT_LIST_DESTINATIONS`]
    /// destinations, which only a custom Notification Class object can do.
    /// No destination is selected: a transition never reaches only part of a
    /// list (#1124).
    RecipientListTooLong,
    /// The class contains a valid list with zero configured destinations.
    NoConfiguredDestinations,
    /// Destinations are configured, but none is eligible for this selection.
    NoMatchingDestinations,
    /// Eligible configured destinations and their delivery settings.
    Matched(Vec<(BACnetRecipient, u32, bool)>),
}

/// Look up and select recipients for a Notification Class transition.
///
/// This is the canonical recipient lookup API and distinguishes configuration
/// failures from valid empty or ineligible configuration. Selection uses the
/// configured destination's valid days, local time window, and transitions.
/// `today` is the current local day, as [`local_day_and_time`] returns it.
///
/// A malformed complete list returns
/// [`RecipientListInvalid`](RecipientLookupOutcome::RecipientListInvalid);
/// no decodable prefix is selected. A list longer than
/// [`MAX_RECIPIENT_LIST_DESTINATIONS`] returns
/// [`RecipientListTooLong`](RecipientLookupOutcome::RecipientListTooLong)
/// without decoding the destinations past the cap. Every non-matched outcome
/// is fail-closed and names no implicit destination.
pub fn lookup_notification_recipients(
    db: &ObjectDatabase,
    notification_class: u32,
    transition: EventTransition,
    today: DaysOfWeek,
    current_time: &Time,
) -> RecipientLookupOutcome {
    let Some(nc) = find_notification_class(db, notification_class) else {
        return RecipientLookupOutcome::NotificationClassMissing;
    };
    let Ok(recipient_list_value) = nc.read_property(PropertyIdentifier::RECIPIENT_LIST, None)
    else {
        return RecipientLookupOutcome::RecipientListUnavailable;
    };
    let destinations = match routed_destinations(&recipient_list_value) {
        Ok(destinations) => destinations,
        Err(outcome) => return outcome,
    };
    if destinations.is_empty() {
        return RecipientLookupOutcome::NoConfiguredDestinations;
    }

    let recipients = filter_destinations(destinations, transition, today, current_time);
    if recipients.is_empty() {
        RecipientLookupOutcome::NoMatchingDestinations
    } else {
        RecipientLookupOutcome::Matched(recipients)
    }
}

/// Get notification recipients for a given class number and transition.
///
/// This source-compatible wrapper delegates to
/// [`lookup_notification_recipients`] and returns the matched recipient tuples.
/// Every other outcome maps to an empty vector, preserving the legacy API.
pub fn get_notification_recipients(
    db: &ObjectDatabase,
    notification_class: u32,
    transition: EventTransition,
    today: DaysOfWeek,
    current_time: &Time,
) -> Vec<(BACnetRecipient, u32, bool)> {
    match lookup_notification_recipients(db, notification_class, transition, today, current_time) {
        RecipientLookupOutcome::Matched(recipients) => recipients,
        RecipientLookupOutcome::NotificationClassMissing
        | RecipientLookupOutcome::RecipientListUnavailable
        | RecipientLookupOutcome::RecipientListInvalid
        | RecipientLookupOutcome::RecipientListTooLong
        | RecipientLookupOutcome::NoConfiguredDestinations
        | RecipientLookupOutcome::NoMatchingDestinations => Vec::new(),
    }
}

/// Strict variant of [`get_notification_recipients`] for fail-closed routing.
///
/// This source-compatible wrapper delegates to
/// [`lookup_notification_recipients`]. It preserves `None` for an invalid or
/// undecodable complete list, and a list past the cap, and `Some([])` for
/// missing class, property-read failure, configured empty, and no-match
/// outcomes. Successful matches return `Some(recipients)`.
pub fn get_notification_recipients_strict(
    db: &ObjectDatabase,
    notification_class: u32,
    transition: EventTransition,
    today: DaysOfWeek,
    current_time: &Time,
) -> Option<Vec<(BACnetRecipient, u32, bool)>> {
    match lookup_notification_recipients(db, notification_class, transition, today, current_time) {
        RecipientLookupOutcome::RecipientListInvalid
        | RecipientLookupOutcome::RecipientListTooLong => None,
        RecipientLookupOutcome::Matched(recipients) => Some(recipients),
        RecipientLookupOutcome::NotificationClassMissing
        | RecipientLookupOutcome::RecipientListUnavailable
        | RecipientLookupOutcome::NoConfiguredDestinations
        | RecipientLookupOutcome::NoMatchingDestinations => Some(Vec::new()),
    }
}

/// Strictly decode a `RECIPIENT_LIST` property value into its destinations.
///
/// Only the framed wire form ([`PropertyValue::ApplicationData`]) is a
/// Recipient_List value (#1125). The FIRST malformed destination (or trailing
/// bytes) fails the whole decode: a prefix-tolerant walk would silently route
/// notifications to only a subset of the configured recipients.
///
/// This applies no cap beyond the codec's own item limit. GetEnrollmentSummary
/// reads membership from the list as configured; routing goes through
/// [`routed_destinations`], which holds every class to
/// [`MAX_RECIPIENT_LIST_DESTINATIONS`].
pub(super) fn decode_destination_list_pv(
    value: &PropertyValue,
) -> Result<Vec<BACnetDestination>, Error> {
    match value {
        PropertyValue::ApplicationData(bytes) => {
            bacnet_encoding::constructed::decode_destination_list(bytes)
        }
        _ => Err(Error::decoding(
            0,
            "Recipient_List: expected framed application data",
        )),
    }
}

/// Decode a `RECIPIENT_LIST` value for routing: the framed form only, at most
/// [`MAX_RECIPIENT_LIST_DESTINATIONS`] destinations, all or nothing (#1124).
/// The error is the lookup outcome that names why nothing is routed.
fn routed_destinations(
    value: &PropertyValue,
) -> Result<Vec<BACnetDestination>, RecipientLookupOutcome> {
    let PropertyValue::ApplicationData(bytes) = value else {
        return Err(RecipientLookupOutcome::RecipientListInvalid);
    };
    recipient_list::decode_capped(bytes).map_err(|error| match error {
        recipient_list::CappedListError::Malformed => RecipientLookupOutcome::RecipientListInvalid,
        recipient_list::CappedListError::PastTheCap(_) => {
            RecipientLookupOutcome::RecipientListTooLong
        }
    })
}

/// Filter decoded destinations by day, time, and transition — the shared
/// selection step of [`filter_recipient_list`] and
/// [`lookup_notification_recipients`].
fn filter_destinations(
    destinations: Vec<BACnetDestination>,
    transition: EventTransition,
    today: DaysOfWeek,
    current_time: &Time,
) -> Vec<(BACnetRecipient, u32, bool)> {
    let transition_mask = transition.bit_mask();
    destinations
        .into_iter()
        .filter(|dest| dest.valid_days.intersects(today))
        .filter(|dest| time_in_window(current_time, &dest.from_time, &dest.to_time))
        .filter(|dest| dest.transitions.intersects(transition_mask))
        .map(|dest| {
            (
                dest.recipient,
                dest.process_identifier,
                dest.issue_confirmed_notifications,
            )
        })
        .collect()
}

/// Filter an encoded `RECIPIENT_LIST` property value by day, time, and transition.
///
/// Parses the value as returned by `read_property(RECIPIENT_LIST)`, the
/// framed `BACnetLIST of BACnetDestination` form, and returns only those
/// recipients matching the given filters. A list that fails to decode (even
/// partially), or holds more than [`MAX_RECIPIENT_LIST_DESTINATIONS`]
/// destinations, yields NO recipients: routing fails closed rather than
/// notifying a silently-truncated prefix of the configured destinations.
pub fn filter_recipient_list(
    recipient_list_value: &PropertyValue,
    transition: EventTransition,
    today: DaysOfWeek,
    current_time: &Time,
) -> Vec<(BACnetRecipient, u32, bool)> {
    let Ok(destinations) = routed_destinations(recipient_list_value) else {
        return Vec::new();
    };
    filter_destinations(destinations, transition, today, current_time)
}

#[cfg(test)]
mod tests;
