//! Notification Forwarder object (type 51, Clause 12.51).
//!
//! A forwarder originates no events. It takes the event notifications this
//! device receives, and those its own objects address to the local Device,
//! and sends each one on to the destinations it holds: the configured
//! Recipient_List and the expiring Subscribed_Recipients. The server does the
//! sending; see [`forwarding_targets`] for which notifications a forwarder
//! takes and where it sends them.
//!
//! # Rows
//!
//! The object serves every required row of Table 12-58, plus Description and,
//! when the application configures one, Port_Filter. Over the network:
//!
//! - Recipient_List takes the same writes as a Notification Class's, capped
//!   at [`MAX_RECIPIENT_LIST_DESTINATIONS`] destinations.
//! - Subscribed_Recipients is a [`SubscribedRecipients`] store: entries run
//!   out after their Time Remaining and are edited by AddListElement and
//!   RemoveListElement.
//! - Process_Identifier_Filter takes an Unsigned32, or NULL to forward every
//!   process identifier; Local_Forwarding_Only and Out_Of_Service take a
//!   BOOLEAN.
//! - Port_Filter takes writes of its Enabled members only (Clause 12.51.11):
//!   the array keeps its size and Port_IDs.
//! - Reliability is always NO_FAULT_DETECTED and is not writable.
//!
//! # Restarts
//!
//! Clauses 12.51.8 and 12.51.9 ask for both lists to survive a restart. A
//! forwarder built with [`NotificationForwarderObject::with_persistence`]
//! saves Recipient_List and Subscribed_Recipients together, each
//! subscription with the minutes it has left, when a write changes either
//! list, when an entry lapses, and at most once a minute while the minutes
//! the entries serve fall; it restores both when built again. A restored
//! entry counts down from its saved minutes: no fewer than it had left when
//! the device stopped, about a minute more at most, and never more than its
//! last subscription gave it, so restarts do not keep an entry alive.
//!
//! Saves run on the forwarder's own writer thread, never while the database
//! guard is held ([`crate::durable`]), and a list write that cannot be saved
//! is refused with DEVICE / OPERATIONAL_PROBLEM, leaving the old list. A
//! failed save is logged and counted ([`ForwarderSaveCounters`]); one the
//! operation task made is retried a minute later.
//!
//! A written Recipient_List wins over the one the application configures:
//! once a write has set the list and it was saved, a rebuilt forwarder
//! serves the saved list and ignores [`add_destination`] calls. Until then
//! the configured destinations apply at every start; they are not saved. A
//! forwarder built with [`NotificationForwarderObject::new`] keeps both lists
//! in memory only.
//!
//! [`add_destination`]: NotificationForwarderObject::add_destination

use crate::prelude::*;
use alloc::borrow::Cow;
use alloc::sync::Arc;
use core::time::Duration;

use bacnet_types::constructed::{
    BACnetDestination, BACnetEventNotificationSubscription, BACnetPortPermission,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use bytes::BytesMut;

use crate::common::{self, read_common_properties};
use crate::durable::staged::{self, Step};
use crate::durable::{DurableWrites, PendingWrite, SaveWait, StageStep};
use crate::notification_class::recipient_list;
use crate::subscribed_recipients::SubscribedRecipients;
use crate::traits::{BACnetObject, MonotonicClock};

mod metadata;
mod persistence;
mod port_filter;
mod saving;
mod selection;

pub use crate::notification_class::MAX_RECIPIENT_LIST_DESTINATIONS;
#[cfg(feature = "std")]
pub use persistence::FileNotificationForwarderPersistence;
pub use persistence::{ForwarderSnapshot, NotificationForwarderPersistence};
pub use saving::ForwarderSaveCounters;
pub use selection::{ForwardingInput, ForwardingTargets, forwarding_targets};

/// BACnet Notification Forwarder object (type 51). See the
/// [module documentation](self).
///
/// # Unwind safety
///
/// The forwarder is neither `UnwindSafe` nor `RefUnwindSafe`, by decision
/// (#1452). Its Subscribed_Recipients keep the monotonic clock the server
/// binds, a closure the caller supplies and runs on its own thread. Nothing
/// catches a panic from it, so wrapping it in `AssertUnwindSafe`, as the
/// save writer's parts are, would claim more than the type can promise.
/// Caller clocks get no unwind-safety bound either.
pub struct NotificationForwarderObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
    reliability: Reliability,
    out_of_service: bool,
    recipient_list: Vec<BACnetDestination>,
    subscribed_recipients: SubscribedRecipients,
    process_identifier_filter: Option<u32>,
    local_forwarding_only: bool,
    port_filter: Option<Vec<BACnetPortPermission>>,
    storage: Option<saving::Storage>,
    /// Writes either list has taken, so a staged write can tell whether
    /// another came between.
    list_writes: u64,
    /// A write set Recipient_List, now or before a restart, so storage keeps
    /// it and configured destinations no longer apply.
    recipient_list_written: bool,
    save_counters: ForwarderSaveCounters,
}

impl NotificationForwarderObject {
    /// A forwarder with empty lists that forwards every process identifier
    /// for any device. Its lists are kept in memory only.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        Ok(Self {
            oid: ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, instance)?,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
            reliability: Reliability::NO_FAULT_DETECTED,
            out_of_service: false,
            recipient_list: Vec::new(),
            subscribed_recipients: SubscribedRecipients::new(),
            process_identifier_filter: None,
            local_forwarding_only: false,
            port_filter: None,
            storage: None,
            list_writes: 0,
            recipient_list_written: false,
            save_counters: ForwarderSaveCounters::default(),
        })
    }

    /// A forwarder that keeps its Recipient_List and Subscribed_Recipients
    /// in `persistence`, starting from the lists saved there for this object,
    /// if any.
    ///
    /// Fails when the saved lists cannot be loaded, or hold an entry a write
    /// would refuse: a destination a framed write could not carry, a
    /// subscription with a Time Remaining of 0 or past a day, or more entries
    /// than a list's cap.
    pub fn with_persistence(
        instance: u32,
        name: impl Into<String>,
        persistence: Arc<dyn NotificationForwarderPersistence>,
    ) -> Result<Self, Error> {
        let mut forwarder = Self::new(instance, name)?;
        if let Some(saved) = persistence.load(forwarder.oid)? {
            if let Some(list) = saved.recipient_list {
                if list.len() > MAX_RECIPIENT_LIST_DESTINATIONS {
                    return Err(recipient_list::no_space_error());
                }
                for destination in &list {
                    recipient_list::check_added(destination)?;
                }
                forwarder.recipient_list = list;
                forwarder.recipient_list_written = true;
            }
            if !saved.subscribed_recipients.is_empty() {
                forwarder
                    .subscribed_recipients
                    .write(framed(&saved.subscribed_recipients)?)?;
            }
        }
        forwarder.storage = Some(saving::Storage::new(
            forwarder.oid,
            persistence,
            forwarder.snapshot(),
            forwarder.save_counters.clone(),
        ));
        Ok(forwarder)
    }

    /// This forwarder's save counters, shared with the object, so they stay
    /// readable after it joins a database. They stay zero without
    /// persistence.
    pub fn save_counters(&self) -> ForwarderSaveCounters {
        self.save_counters.clone()
    }

    /// Set the description string.
    pub fn set_description(&mut self, description: impl Into<String>) {
        self.description = description.into();
    }

    /// The process identifier a notification must carry to be forwarded, or
    /// `None` (Process_Identifier_Filter NULL) to forward every one.
    pub fn process_identifier_filter(&self) -> Option<u32> {
        self.process_identifier_filter
    }

    /// Set Process_Identifier_Filter; `None` is NULL.
    pub fn set_process_identifier_filter(&mut self, filter: Option<u32>) {
        self.process_identifier_filter = filter;
    }

    /// Whether only notifications this device's own objects generate are
    /// forwarded.
    pub fn local_forwarding_only(&self) -> bool {
        self.local_forwarding_only
    }

    /// Set Local_Forwarding_Only.
    pub fn set_local_forwarding_only(&mut self, local_only: bool) {
        self.local_forwarding_only = local_only;
    }

    /// Add a destination to Recipient_List, with the checks and cap of a
    /// Notification Class's
    /// [`add_destination`](crate::notification_class::NotificationClass::add_destination).
    ///
    /// This configures the list the application starts with; it is not
    /// saved. On a forwarder [`with_persistence`](Self::with_persistence)
    /// whose storage holds a written Recipient_List
    /// ([`recipient_list_saved`](Self::recipient_list_saved)), the saved list
    /// wins: the destination is checked but not added.
    pub fn add_destination(&mut self, destination: BACnetDestination) -> Result<(), Error> {
        recipient_list::check_added(&destination)?;
        if self.recipient_list_saved() {
            log_debug!(
                forwarder = %self.oid,
                "Saved Recipient_List kept over a configured destination"
            );
            return Ok(());
        }
        if self.recipient_list.len() >= MAX_RECIPIENT_LIST_DESTINATIONS {
            return Err(recipient_list::no_space_error());
        }
        self.recipient_list.push(destination);
        Ok(())
    }

    /// The Recipient_List destinations, in list order.
    pub fn recipient_list(&self) -> &[BACnetDestination] {
        &self.recipient_list
    }

    /// The live Subscribed_Recipients entries, each with the whole minutes it
    /// has left.
    pub fn subscriptions(&self) -> Vec<BACnetEventNotificationSubscription> {
        self.subscribed_recipients.subscriptions()
    }

    /// Serve Port_Filter with one element per network port, or `None` to
    /// leave the property out, as a device that does not route may
    /// (Clause 12.51.11). The server receives through one port, Port_ID 0.
    pub fn set_port_filter(&mut self, ports: Option<Vec<BACnetPortPermission>>) {
        self.port_filter = ports;
    }

    /// The Port_Filter elements, when the property is served.
    pub fn port_filter(&self) -> Option<&[BACnetPortPermission]> {
        self.port_filter.as_deref()
    }

    /// Whether storage holds a Recipient_List a write set, now or before a
    /// restart. [`add_destination`](Self::add_destination) then leaves the
    /// list alone. Always false without persistence.
    pub fn recipient_list_saved(&self) -> bool {
        self.storage.is_some() && self.recipient_list_written
    }

    /// Block until the saves queued so far have run. Dropping the forwarder
    /// waits for them too.
    pub fn wait_for_saves(&self) {
        if let Some(storage) = &self.storage {
            storage.wait_idle();
        }
    }

    /// What storage holds for the lists the forwarder serves.
    fn snapshot(&self) -> ForwarderSnapshot {
        ForwarderSnapshot {
            recipient_list: self
                .recipient_list_written
                .then(|| self.recipient_list.clone()),
            subscribed_recipients: self.subscribed_recipients.subscriptions(),
        }
    }

    /// The list a write of `value` to `property` would leave on top of the
    /// `earlier` steps of its request, or the write's refusal.
    fn next_list(
        &self,
        earlier: &[Step<saving::NextList>],
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<saving::NextList, Error> {
        if property == PropertyIdentifier::RECIPIENT_LIST {
            return recipient_list::decode_write(value).map(saving::NextList::RecipientList);
        }
        // A written entry keeps the deadline of a live one it renews, so the
        // store builds on the one an earlier step left.
        let current = earlier
            .iter()
            .rev()
            .find_map(|step| match &step.next {
                saving::NextList::SubscribedRecipients(store) => Some(store),
                saving::NextList::RecipientList(_) => None,
            })
            .unwrap_or(&self.subscribed_recipients);
        let mut next = current.clone();
        next.write(value)?;
        Ok(saving::NextList::SubscribedRecipients(next))
    }

    fn install(&mut self, next: saving::NextList) {
        match next {
            saving::NextList::RecipientList(list) => {
                self.recipient_list = list;
                self.recipient_list_written = true;
            }
            saving::NextList::SubscribedRecipients(store) => self.subscribed_recipients = store,
        }
        self.list_writes = self.list_writes.wrapping_add(1);
    }

    /// Replace Recipient_List or Subscribed_Recipients with a written list,
    /// saving both lists before the forwarder serves it. A staged write takes
    /// the lists already saved; any other saves now. A list that cannot be
    /// saved is refused with DEVICE / OPERATIONAL_PROBLEM and the old list
    /// stays.
    fn write_list(
        &mut self,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let refused = |_| {
            common::protocol_error(
                bacnet_types::enums::ErrorClass::DEVICE,
                bacnet_types::enums::ErrorCode::OPERATIONAL_PROBLEM,
            )
        };
        if let Some(taken) = self.take_staged(property, &value) {
            return taken.map_err(refused);
        }
        // A list the forwarder refuses leaves a write staged for another
        // request alone (#1424).
        let next = self.next_list(&[], property, value)?;
        if self.storage.is_some() {
            let mut snapshot = self.snapshot();
            saving::apply(&mut snapshot, &next);
            // This write will be made, so it supersedes a staged one: storage
            // goes back to the served lists ahead of its save.
            self.with_storage(|storage, _| storage.drop_staged());
            let storage = self.storage.as_mut().expect("checked above");
            storage.save_now(snapshot).map_err(refused)?;
        }
        self.install(next);
        Ok(())
    }

    /// Take the staged step for a write of `value` to `property`, if it is
    /// the next one: the forwarder then serves its list, or the save's error
    /// comes back. `None` when nothing staged is this write.
    fn take_staged(
        &mut self,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        let mut storage = self.storage.take()?;
        let taken = storage
            .claim(property, value, self.list_writes)
            .map(|claimed| claimed.map(|next| self.install(next)));
        // Steps the request has still to take keep the lists served now.
        storage.correct(|| self.snapshot());
        self.storage = Some(storage);
        taken
    }

    /// After time passes, keep storage current (see the module docs).
    fn after_time_passed(&mut self, lapsed: bool) -> bool {
        let now = self.subscribed_recipients.current_time();
        self.with_storage(|storage, served| storage.keep_current(now, lapsed, served));
        lapsed
    }

    /// Run `f` on storage, handing it the lists the forwarder serves; then,
    /// if `f` dropped a staged write, queue the save that puts storage back
    /// to the served lists. `None` without persistence.
    fn with_storage<R>(
        &mut self,
        f: impl FnOnce(&mut saving::Storage, &dyn Fn() -> ForwarderSnapshot) -> R,
    ) -> Option<R> {
        let mut storage = self.storage.take()?;
        let served = || self.snapshot();
        let result = f(&mut storage, &served);
        storage.correct(served);
        self.storage = Some(storage);
        Some(result)
    }
}

/// Whether `property` is one of the two lists a forwarder saves.
fn listed(property: PropertyIdentifier) -> bool {
    matches!(
        property,
        PropertyIdentifier::RECIPIENT_LIST | PropertyIdentifier::SUBSCRIBED_RECIPIENTS
    )
}

impl DurableWrites for NotificationForwarderObject {
    fn stage_write(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> StageStep {
        self.stage_writes(&[PendingWrite {
            property,
            array_index,
            value: value.clone(),
        }])
    }

    /// Fold the request's writes to both lists into one staged save
    /// (#1423).
    fn stage_writes(&mut self, writes: &[PendingWrite]) -> StageStep {
        if self.storage.is_none() || !writes.iter().any(|write| listed(write.property)) {
            return StageStep::Skip;
        }
        if let Some(wait) = self.with_storage(|storage, _| storage.busy()).flatten() {
            return StageStep::Busy(wait);
        }
        let steps = staged::steps(writes, |earlier, write| {
            if !listed(write.property) {
                return Ok(None);
            }
            if write.array_index.is_some() {
                return Err(common::property_is_not_an_array_error());
            }
            self.next_list(earlier, write.property, write.value.clone())
                .map(Some)
        });
        let served = self.snapshot();
        let base = self.list_writes;
        let storage = self.storage.as_mut().expect("checked above");
        storage.stage(steps, base, served)
    }

    fn release_staged_write(&mut self, staged: &SaveWait) {
        self.with_storage(|storage, _| storage.release(staged));
    }

    fn has_staged_write(&self) -> bool {
        self.storage
            .as_ref()
            .is_some_and(saving::Storage::is_staged)
    }

    fn settle_forgotten_writes(&mut self) -> Option<SaveWait> {
        self.with_storage(|storage, served| storage.drop_forgotten(served))
    }
}

/// A list of subscriptions in the framed form the store writes. Fails for a
/// recipient MAC past `BACnetAddress::MAX_MAC_LEN`, which a loaded list cannot
/// hold since its decoder refuses one.
fn framed(subscriptions: &[BACnetEventNotificationSubscription]) -> Result<PropertyValue, Error> {
    let mut buf = BytesMut::new();
    bacnet_encoding::constructed::encode_event_notification_subscription_list(
        &mut buf,
        subscriptions,
    )?;
    Ok(PropertyValue::ApplicationData(buf.to_vec()))
}

fn read_bool(value: &PropertyValue) -> Result<bool, Error> {
    match value {
        PropertyValue::Boolean(value) => Ok(*value),
        _ => Err(common::invalid_data_type_error()),
    }
}

impl BACnetObject for NotificationForwarderObject {
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
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::NOTIFICATION_FORWARDER.to_raw(),
            )),
            PropertyIdentifier::RECIPIENT_LIST => {
                let mut buf = BytesMut::new();
                bacnet_encoding::constructed::encode_destination_list(
                    &mut buf,
                    &self.recipient_list,
                )?;
                Ok(PropertyValue::ApplicationData(buf.to_vec()))
            }
            PropertyIdentifier::SUBSCRIBED_RECIPIENTS => Ok(self.subscribed_recipients.read()),
            PropertyIdentifier::PROCESS_IDENTIFIER_FILTER => Ok(self
                .process_identifier_filter
                .map_or(PropertyValue::Null, |id| PropertyValue::Unsigned(id.into()))),
            PropertyIdentifier::LOCAL_FORWARDING_ONLY => {
                Ok(PropertyValue::Boolean(self.local_forwarding_only))
            }
            PropertyIdentifier::PORT_FILTER => match &self.port_filter {
                Some(ports) => port_filter::read(ports, array_index),
                None => Err(common::unknown_property_error()),
            },
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
        if listed(property) && array_index.is_some() {
            return Err(common::property_is_not_an_array_error());
        }
        match property {
            PropertyIdentifier::RECIPIENT_LIST | PropertyIdentifier::SUBSCRIBED_RECIPIENTS => {
                return self.write_list(property, value);
            }
            PropertyIdentifier::PROCESS_IDENTIFIER_FILTER if array_index.is_none() => {
                self.process_identifier_filter = match value {
                    PropertyValue::Null => None,
                    PropertyValue::Unsigned(id) => {
                        Some(u32::try_from(id).map_err(|_| common::value_out_of_range_error())?)
                    }
                    _ => return Err(common::invalid_data_type_error()),
                };
                return Ok(());
            }
            PropertyIdentifier::LOCAL_FORWARDING_ONLY if array_index.is_none() => {
                self.local_forwarding_only = read_bool(&value)?;
                return Ok(());
            }
            PropertyIdentifier::PORT_FILTER => {
                if let Some(ports) = &mut self.port_filter {
                    return port_filter::write(ports, array_index, value);
                }
            }
            _ => {}
        }
        if array_index.is_none() {
            if let Some(result) =
                common::write_out_of_service(&mut self.out_of_service, property, &value)
            {
                return result;
            }
            if let Some(result) = common::write_description(&mut self.description, property, &value)
            {
                return result;
            }
        }
        Err(common::unhandled_write_error(
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

    fn advance_time_internal(&mut self, elapsed: Duration) -> bool {
        let lapsed = self.subscribed_recipients.advance_by(elapsed);
        self.after_time_passed(lapsed)
    }

    fn bind_monotonic_clock_internal(&mut self, clock: Option<Arc<MonotonicClock>>) {
        self.subscribed_recipients.bind_monotonic_clock(clock);
        self.with_storage(|storage, _| storage.clock_changed());
    }

    fn advance_monotonic_time_internal(&mut self, now: Duration) -> bool {
        let lapsed = self.subscribed_recipients.advance_to(now);
        self.after_time_passed(lapsed)
    }

    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        self.subscribed_recipients.next_deadline()
    }

    fn durable_writes_internal(&mut self) -> Option<&mut dyn DurableWrites> {
        Some(self)
    }
}

#[cfg(test)]
mod tests;
