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
//! [`DurableWrites::expire_staged`]. The class saves nothing else between
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
//!
//! A written Description is kept the same way, in the same file (#2364):
//! saved before it is answered, refused with DEVICE / OPERATIONAL_PROBLEM
//! when the save fails, and winning over `set_description` once saved. A
//! NULL changes nothing, so it is neither saved nor staged, and one past
//! 16 KiB is refused with RESOURCES / NO_SPACE_TO_WRITE_PROPERTY.

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
use crate::write_config::description;

/// What a saved write leaves: the property it sets, as the class holds it.
pub(super) enum Next {
    RecipientList(Vec<BACnetDestination>),
    Description(String),
}

/// A class's writer and the write staged on it.
pub(super) type Storage = StagedSaves<NotificationClassSnapshot, Next>;

/// Whether the class keeps a write of `value` to `property`: a
/// Recipient_List, or a Description other than a NULL, which changes
/// nothing.
pub(super) fn kept(property: PropertyIdentifier, value: &PropertyValue) -> bool {
    property == PropertyIdentifier::RECIPIENT_LIST
        || (property == PropertyIdentifier::DESCRIPTION
            && !description::null_write(property, value))
}

impl NotificationClass {
    /// A Notification Class that keeps a written Recipient_List in
    /// `persistence`, starting from the list saved there for this object, if
    /// any. Its Notification_Class number defaults to the instance number,
    /// as with [`new`](Self::new).
    ///
    /// A written Description is kept there too (#2364), and the saved one
    /// served.
    ///
    /// Fails when the saved list cannot be loaded, or holds what a write
    /// would refuse: more than [`MAX_RECIPIENT_LIST_DESTINATIONS`]
    /// destinations, an address recipient whose MAC is too long, or a
    /// Description past 16 KiB.
    pub fn with_persistence(
        instance: u32,
        name: impl Into<String>,
        persistence: Arc<dyn NotificationClassPersistence>,
    ) -> Result<Self, Error> {
        let mut class = Self::new(instance, name)?;
        let oid = class.oid;
        let saved = persistence.load(oid)?.unwrap_or_default();
        description::check_saved(saved.description.as_deref())?;
        if let Some(list) = saved.recipient_list {
            if list.len() > MAX_RECIPIENT_LIST_DESTINATIONS {
                return Err(recipient_list::no_space_error());
            }
            for destination in &list {
                recipient_list::check_added(destination)?;
            }
            class.recipient_list = list;
            class.recipient_list_written = true;
        }
        if let Some(text) = saved.description {
            class.description = text;
            class.description_written = true;
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

    /// Whether a setter of Description leaves it alone: storage holds a
    /// Description a write set, now or before a restart (#2364).
    pub(super) fn description_saved(&self) -> bool {
        self.storage.is_some() && self.description_written
    }

    /// What storage holds for the state the class serves.
    fn snapshot(&self) -> NotificationClassSnapshot {
        NotificationClassSnapshot {
            recipient_list: self
                .recipient_list_written
                .then(|| self.recipient_list.clone()),
            description: self.description_written.then(|| self.description.clone()),
        }
    }

    fn install(&mut self, next: Next) {
        match next {
            Next::RecipientList(list) => {
                self.recipient_list = list;
                self.recipient_list_written = true;
            }
            Next::Description(text) => {
                self.description = text;
                self.description_written = true;
            }
        }
        self.kept_writes = self.kept_writes.wrapping_add(1);
    }

    /// What a write of `value` to `property`, one the class [`kept`], leaves,
    /// or its refusal.
    fn next(property: PropertyIdentifier, value: PropertyValue) -> Result<Next, Error> {
        if property == PropertyIdentifier::DESCRIPTION {
            return description::written(&value).map(Next::Description);
        }
        recipient_list::decode_write(value).map(Next::RecipientList)
    }

    /// Make a write of Recipient_List, or of Description with storage,
    /// saving it before the class serves it. A staged write takes the state
    /// already saved; any other saves now. A write that cannot be saved is
    /// refused with DEVICE / OPERATIONAL_PROBLEM and the old value stays.
    pub(super) fn write_kept(
        &mut self,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let refused =
            |_| common::protocol_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM);
        if let Some(taken) = self.take_staged(property, &value) {
            return taken.map_err(refused);
        }
        // A write the class refuses leaves a write staged for another
        // request alone (#1424).
        let next = Self::next(property, value)?;
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

    /// Take the staged step for a write of `value` to `property`, if it is
    /// the next one: the class then serves its state, or the save's error
    /// comes back. `None` when nothing staged is this write.
    fn take_staged(
        &mut self,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        let mut storage = self.storage.take()?;
        let base = self.kept_writes;
        let taken = storage
            .claim(property, None, value, base)
            .map(|claimed| claimed.map(|next| self.install(next)));
        // Steps the request has still to take keep the list served now.
        storage.correct(|| self.snapshot());
        self.storage = Some(storage);
        taken
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

/// Put what a write leaves into `snapshot`.
fn apply(snapshot: &mut NotificationClassSnapshot, next: &Next) {
    match next {
        Next::RecipientList(list) => snapshot.recipient_list = Some(list.clone()),
        Next::Description(text) => snapshot.description = Some(text.clone()),
    }
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

    /// Fold the request's Recipient_List and Description writes into one
    /// staged save (#1423); each leaves its property on top of the ones
    /// before it.
    fn stage_writes(&mut self, writes: &[PendingWrite]) -> StageStep {
        let listed = |write: &PendingWrite| kept(write.property, &write.value);
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
            Self::next(write.property, write.value.clone()).map(Some)
        });
        let served = self.snapshot();
        let base = self.kept_writes;
        let storage = self.storage.as_mut().expect("checked above");
        storage.stage(steps, base, served, apply)
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

    fn expire_staged(&mut self, now: Duration) {
        self.with_storage(|storage| storage.expire(now));
    }
}
