//! When a Notification Class saves its Recipient_List (Clause 12.21.8,
//! #1315).
//!
//! A class built with [`NotificationClass::with_persistence`] saves a written
//! Recipient_List before serving it, and refuses a write it cannot save with
//! DEVICE / OPERATIONAL_PROBLEM, keeping the old list. This is the
//! Notification Forwarder's scheme ([`crate::durable`]), shared with it
//! through `StagedSaves`: the bundled server stages the write, the save runs
//! on the class's writer thread while the database guard is dropped, and the
//! write then takes the saved list without saving again. A
//! WritePropertyMultiple that writes the list more than once stages one
//! save of the last (#1423). A write that was not staged queues its save and
//! waits for it where it is.
//!
//! A staged write its request releases without making is dropped, and the
//! class at once queues a save of the list it serves, so storage goes back to
//! that list. A staged write whose request vanished without releasing it, as
//! when `stop()` aborts a request or an application drops a local write's
//! future, is dropped the same way once
//! [`STAGED_WRITE_LIFETIME`](crate::durable::STAGED_WRITE_LIFETIME) has
//! passed since its save finished: by the next write that stages, or by the
//! server's once-a-second operation task, which calls
//! `advance_monotonic_time_internal`. The class saves nothing else between
//! writes.
//!
//! Neither runs once the server has stopped. So `stop()`, once it has
//! joined its requests, drops a staged write that is still held and waits
//! for the save of the served list, and a class dropped with one still
//! held saves the served list as it goes, unless the staged save failed
//! (#1363). Either way a restart serves the list the class served.
//!
//! A written list wins over the destinations the application configures:
//! once a write has set the list and it was saved, a rebuilt class serves the
//! saved list, and [`add_destination`](NotificationClass::add_destination)
//! checks a destination without adding it. The configured destinations are
//! never saved.

use crate::prelude::*;
use alloc::sync::Arc;
use core::time::Duration;

use bacnet_types::constructed::BACnetDestination;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::persistence::{NotificationClassPersistence, NotificationClassSnapshot};
use super::{MAX_RECIPIENT_LIST_DESTINATIONS, NotificationClass, recipient_list};
use crate::common;
use crate::durable::staged::{self, StagedSaves};
use crate::durable::{DurableWrites, PendingWrite, SaveWait, SaveWriter, StageStep};

/// A class's writer and the Recipient_List write staged on it.
pub(super) type Storage = StagedSaves<NotificationClassSnapshot, Vec<BACnetDestination>>;

impl NotificationClass {
    /// A Notification Class that keeps a written Recipient_List in
    /// `persistence`, starting from the list saved there for this object, if
    /// any. Its Notification_Class number defaults to the instance number,
    /// as with [`new`](Self::new).
    ///
    /// Fails when the saved list cannot be loaded, or holds what a write
    /// would refuse: more than [`MAX_RECIPIENT_LIST_DESTINATIONS`]
    /// destinations, or an address recipient whose MAC is too long.
    pub fn with_persistence(
        instance: u32,
        name: impl Into<String>,
        persistence: Arc<dyn NotificationClassPersistence>,
    ) -> Result<Self, Error> {
        let mut class = Self::new(instance, name)?;
        let oid = class.oid;
        if let Some(list) = persistence
            .load(oid)?
            .and_then(|saved| saved.recipient_list)
        {
            if list.len() > MAX_RECIPIENT_LIST_DESTINATIONS {
                return Err(recipient_list::no_space_error());
            }
            for destination in &list {
                recipient_list::check_added(destination)?;
            }
            class.recipient_list = list;
            class.recipient_list_written = true;
        }
        let writer = SaveWriter::new(
            format!("bacnet-nc-{instance}-save"),
            move |snapshot: &NotificationClassSnapshot| persistence.save(oid, snapshot),
            move |_, result| {
                if let Err(error) = result {
                    log_warn!(
                        class = %oid,
                        %error,
                        "Failed to save a Notification Class Recipient_List"
                    );
                }
            },
        );
        class.storage = Some(StagedSaves::new(writer));
        Ok(class)
    }

    /// Whether storage holds a Recipient_List a write set, now or before a
    /// restart. [`add_destination`](Self::add_destination) then leaves the
    /// list alone. Always false without persistence.
    pub fn recipient_list_saved(&self) -> bool {
        self.storage.is_some() && self.recipient_list_written
    }

    /// Block until the saves queued so far have run. Dropping the class
    /// waits for them too.
    pub fn wait_for_saves(&self) {
        if let Some(storage) = &self.storage {
            storage.wait_idle();
        }
    }

    /// What storage holds for the list the class serves.
    fn snapshot(&self) -> NotificationClassSnapshot {
        NotificationClassSnapshot {
            recipient_list: self
                .recipient_list_written
                .then(|| self.recipient_list.clone()),
        }
    }

    fn install(&mut self, list: Vec<BACnetDestination>) {
        self.recipient_list = list;
        self.recipient_list_written = true;
        self.list_writes = self.list_writes.wrapping_add(1);
    }

    /// Replace Recipient_List with a written list, saving it before the class
    /// serves it. A staged write takes the list already saved; any other
    /// saves now. A list that cannot be saved is refused with DEVICE /
    /// OPERATIONAL_PROBLEM and the old list stays.
    pub(super) fn write_recipient_list(&mut self, value: PropertyValue) -> Result<(), Error> {
        let refused =
            |_| common::protocol_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM);
        if let Some(taken) = self.take_staged(&value) {
            return taken.map_err(refused);
        }
        // A list the class refuses leaves a write staged for another
        // request alone (#1424).
        let next = recipient_list::decode_write(value)?;
        if self.storage.is_some() {
            let mut snapshot = self.snapshot();
            apply(&mut snapshot, &next);
            // This write will be made, so it supersedes a staged one: storage
            // goes back to the served list ahead of its save.
            self.with_storage(Storage::drop_staged);
            let storage = self.storage.as_mut().expect("checked above");
            storage.save_now(snapshot).map_err(refused)?;
        }
        self.install(next);
        Ok(())
    }

    /// Take the staged step for a Recipient_List write of `value`, if it is
    /// the next one: the class then serves its list, or the save's error
    /// comes back. `None` when nothing staged is this write.
    fn take_staged(&mut self, value: &PropertyValue) -> Option<Result<(), Error>> {
        let mut storage = self.storage.take()?;
        let base = self.list_writes;
        let taken = storage
            .claim(PropertyIdentifier::RECIPIENT_LIST, None, value, base)
            .map(|claimed| claimed.map(|list| self.install(list)));
        // Steps the request has still to take keep the list served now.
        storage.correct(|| self.snapshot());
        self.storage = Some(storage);
        taken
    }

    /// From the server's operation task at monotonic `now`: drop a staged
    /// write whose request is gone and queue the save of the served list
    /// (see the module docs). The save runs on the writer thread.
    pub(super) fn expire_staged_write(&mut self, now: Duration) {
        self.with_storage(|storage| storage.expire(now));
    }

    /// Run `f` on storage; then, if `f` dropped a staged write, queue the
    /// save that puts storage back to the served list. `None` without
    /// persistence.
    fn with_storage<R>(&mut self, f: impl FnOnce(&mut Storage) -> R) -> Option<R> {
        let mut storage = self.storage.take()?;
        let result = f(&mut storage);
        storage.correct(|| self.snapshot());
        self.storage = Some(storage);
        Some(result)
    }
}

/// Put a written Recipient_List into `snapshot`.
fn apply(snapshot: &mut NotificationClassSnapshot, list: &[BACnetDestination]) {
    snapshot.recipient_list = Some(list.to_vec());
}

impl DurableWrites for NotificationClass {
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

    /// Fold the request's Recipient_List writes into one staged save
    /// (#1423); the list each leaves replaces the one before it.
    fn stage_writes(&mut self, writes: &[PendingWrite]) -> StageStep {
        let listed = |write: &PendingWrite| write.property == PropertyIdentifier::RECIPIENT_LIST;
        if self.storage.is_none() || !writes.iter().any(listed) {
            return StageStep::Skip;
        }
        if let Some(wait) = self.with_storage(Storage::busy).flatten() {
            return StageStep::Busy(wait);
        }
        let steps = staged::steps(writes, |_, write| {
            if !listed(write) {
                return Ok(None);
            }
            if write.array_index.is_some() {
                return Err(common::property_is_not_an_array_error());
            }
            recipient_list::decode_write(write.value.clone()).map(Some)
        });
        let served = self.snapshot();
        let base = self.list_writes;
        let storage = self.storage.as_mut().expect("checked above");
        storage.stage(steps, base, served, |snapshot, list| apply(snapshot, list))
    }

    fn release_staged_write(&mut self, staged: &SaveWait) {
        self.with_storage(|storage| storage.release(staged));
    }

    fn has_staged_write(&self) -> bool {
        self.storage.as_ref().is_some_and(Storage::is_staged)
    }

    fn settle_forgotten_writes(&mut self) -> Option<SaveWait> {
        let mut storage = self.storage.take()?;
        let wait = storage.drop_forgotten(|| self.snapshot());
        self.storage = Some(storage);
        Some(wait)
    }
}
