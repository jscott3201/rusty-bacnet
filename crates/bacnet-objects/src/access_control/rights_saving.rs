//! When an Access Rights object saves Positive_Access_Rules,
//! Negative_Access_Rules, Enable (#1392) and Accompaniment (#1393).
//!
//! An object built with [`AccessRightsObject::with_persistence`] saves each
//! write of those four properties before serving it: whole arrays, single
//! elements and index-0 resizes alike, since storage holds whole arrays. A
//! write it cannot save is refused with DEVICE / OPERATIONAL_PROBLEM, and the
//! old value stays. This is the Notification Class's scheme
//! ([`crate::durable`]), shared with it through `StagedSaves`: the bundled
//! server stages the write, the save runs on the object's writer thread while
//! the database guard is dropped, and the write then takes the saved state
//! without saving again. A WritePropertyMultiple's several writes to the
//! object stage one save of the state they leave together, and each takes
//! its own step of it in turn (#1423). A write that was not staged queues its
//! save and waits for it where it is.
//!
//! A staged write its request releases without making is dropped, and the
//! object at once queues a save of what it serves, so storage goes back to
//! that. A staged write whose request vanished is dropped the same way once
//! [`STAGED_WRITE_LIFETIME`](crate::durable::STAGED_WRITE_LIFETIME) has
//! passed since its save finished, by the next write that stages or by the
//! server's once-a-second operation task. The server's `stop()` drops one
//! still held and waits for the correcting save, and an object dropped with
//! one still held saves its served state as it goes, unless the staged save
//! failed (#1363). Either way a restart serves what the object served.
//!
//! A written value wins over what the application configures: once a write
//! has set a property and it was saved, a rebuilt object serves the saved
//! value, and that property's setter checks what it is given without
//! storing it. Configuration alone is never saved, but a write saves the
//! whole array it leaves, so an element or index-0 write to an array no
//! write has set yet also saves the configured rules it didn't touch.

use crate::prelude::*;
use alloc::sync::Arc;
use core::time::Duration;

use bacnet_types::constructed::{BACnetAccessRule, BACnetDeviceObjectReference};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::persistence::{AccessRightsPersistence, AccessRightsSnapshot};
use super::{AccessRightsObject, check_accompaniment, checked_rules};
use crate::common;
use crate::durable::staged::{self, StagedSaves, Step};
use crate::durable::{DurableWrites, PendingWrite, SaveWait, SaveWriter, StageStep};

/// The state a saved write leaves: the property it sets, as the object
/// holds it.
pub(super) enum NextState {
    PositiveAccessRules(Vec<BACnetAccessRule>),
    NegativeAccessRules(Vec<BACnetAccessRule>),
    Enable(bool),
    Accompaniment(BACnetDeviceObjectReference),
}

/// An object's writer and the write staged on it.
pub(super) type Storage = StagedSaves<AccessRightsSnapshot, NextState>;

/// Which saved properties a write has set, now or before a restart.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Written {
    positive: bool,
    negative: bool,
    enable: bool,
    accompaniment: bool,
}

/// Whether `property` is one an Access Rights object saves.
fn saved_property(property: PropertyIdentifier) -> bool {
    matches!(
        property,
        PropertyIdentifier::POSITIVE_ACCESS_RULES
            | PropertyIdentifier::NEGATIVE_ACCESS_RULES
            | PropertyIdentifier::LOG_ENABLE
            | PropertyIdentifier::ACCOMPANIMENT
    )
}

impl AccessRightsObject {
    /// An Access Rights object that keeps written rule arrays, Enable and
    /// Accompaniment in `persistence`, starting from what is saved there for
    /// this object, if anything. A saved Accompaniment serves the optional
    /// row whether or not the application sets one.
    ///
    /// Each saved array and Accompaniment goes through the setters' checks,
    /// so this fails when a saved array holds more than
    /// [`MAX_ACCESS_RULES`](super::MAX_ACCESS_RULES) rules or a rule the
    /// setters refuse, or the saved Accompaniment is one
    /// [`set_accompaniment`](AccessRightsObject::set_accompaniment) refuses,
    /// as well as when storage cannot be read.
    pub fn with_persistence(
        instance: u32,
        name: impl Into<String>,
        persistence: Arc<dyn AccessRightsPersistence>,
    ) -> Result<Self, Error> {
        let mut rights = Self::new(instance, name)?;
        let oid = rights.oid;
        if let Some(saved) = persistence.load(oid)? {
            if let Some(rules) = saved.positive_access_rules {
                rights.positive_access_rules = checked_rules(rules)?;
                rights.written.positive = true;
            }
            if let Some(rules) = saved.negative_access_rules {
                rights.negative_access_rules = checked_rules(rules)?;
                rights.written.negative = true;
            }
            if let Some(enable) = saved.enable {
                rights.enable = enable;
                rights.written.enable = true;
            }
            if let Some(reference) = saved.accompaniment {
                check_accompaniment(&reference)?;
                rights.accompaniment = Some(reference);
                rights.written.accompaniment = true;
            }
        }
        let writer = SaveWriter::new(
            format!("bacnet-ar-{instance}-save"),
            move |snapshot: &AccessRightsSnapshot| persistence.save(oid, snapshot),
            move |_, result| {
                if let Err(error) = result {
                    log_warn!(rights = %oid, %error, "Failed to save Access Rights state");
                }
            },
        );
        rights.storage = Some(StagedSaves::new(writer));
        Ok(rights)
    }

    /// Whether storage holds a value a write set for `property`
    /// (Positive_Access_Rules, Negative_Access_Rules, Enable, property 133,
    /// or Accompaniment), now or before a restart. That property's setter
    /// then leaves it alone. Always false without persistence, and for any
    /// other property.
    pub fn property_saved(&self, property: PropertyIdentifier) -> bool {
        self.storage.is_some()
            && match property {
                PropertyIdentifier::POSITIVE_ACCESS_RULES => self.written.positive,
                PropertyIdentifier::NEGATIVE_ACCESS_RULES => self.written.negative,
                PropertyIdentifier::LOG_ENABLE => self.written.enable,
                PropertyIdentifier::ACCOMPANIMENT => self.written.accompaniment,
                _ => false,
            }
    }

    /// Block until the saves queued so far have run. Dropping the object
    /// waits for them too.
    pub fn wait_for_saves(&self) {
        if let Some(storage) = &self.storage {
            storage.wait_idle();
        }
    }

    /// Whether a setter call for `property` leaves it alone, because a
    /// saved write set it.
    pub(super) fn keeps_saved(&self, property: PropertyIdentifier) -> bool {
        let kept = self.property_saved(property);
        if kept {
            log_debug!(
                rights = %self.oid,
                ?property,
                "Saved Access Rights value kept over a configured one"
            );
        }
        kept
    }

    /// What storage holds for the state the object serves.
    fn snapshot(&self) -> AccessRightsSnapshot {
        let served = |written: bool, rules: &Vec<BACnetAccessRule>| written.then(|| rules.clone());
        AccessRightsSnapshot {
            positive_access_rules: served(self.written.positive, &self.positive_access_rules),
            negative_access_rules: served(self.written.negative, &self.negative_access_rules),
            enable: self.written.enable.then_some(self.enable),
            accompaniment: self
                .written
                .accompaniment
                .then(|| self.accompaniment.clone())
                .flatten(),
        }
    }

    /// The state a write of `value` to `property` (at `array_index`) would
    /// leave on top of the `earlier` steps of its request, or the write's
    /// refusal. The checks are the ones every write makes, with or without
    /// persistence.
    fn next_state(
        &self,
        earlier: &[Step<NextState>],
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<NextState, Error> {
        // An element or index-0 write edits the array an earlier step left.
        let latest = |positive: bool| {
            earlier.iter().rev().find_map(|step| match &step.next {
                NextState::PositiveAccessRules(rules) if positive => Some(rules),
                NextState::NegativeAccessRules(rules) if !positive => Some(rules),
                _ => None,
            })
        };
        let current = match property {
            PropertyIdentifier::POSITIVE_ACCESS_RULES => {
                latest(true).unwrap_or(&self.positive_access_rules)
            }
            PropertyIdentifier::NEGATIVE_ACCESS_RULES => {
                latest(false).unwrap_or(&self.negative_access_rules)
            }
            // Only the application adds the optional row.
            PropertyIdentifier::ACCOMPANIMENT if self.accompaniment.is_none() => {
                return Err(common::unknown_property_error());
            }
            // Enable and Accompaniment are no arrays: the handlers refuse an
            // index before the object sees it, and so does a direct call.
            _ if array_index.is_some() => return Err(common::property_is_not_an_array_error()),
            PropertyIdentifier::ACCOMPANIMENT => {
                let reference: BACnetDeviceObjectReference =
                    crate::device_reference::decode_reference(&value)?;
                check_accompaniment(&reference)?;
                return Ok(NextState::Accompaniment(reference));
            }
            _ => {
                return match value {
                    PropertyValue::Boolean(enable) => Ok(NextState::Enable(enable)),
                    _ => Err(common::invalid_data_type_error()),
                };
            }
        };
        // An indexed write edits the array the object serves; a whole write
        // replaces it.
        let mut next = match array_index {
            Some(_) => current.clone(),
            None => Vec::new(),
        };
        super::super::rights_writes::write_rules(&mut next, array_index, value)?;
        Ok(if property == PropertyIdentifier::POSITIVE_ACCESS_RULES {
            NextState::PositiveAccessRules(next)
        } else {
            NextState::NegativeAccessRules(next)
        })
    }

    fn install(&mut self, next: NextState) {
        match next {
            NextState::PositiveAccessRules(rules) => {
                self.positive_access_rules = rules;
                self.written.positive = true;
            }
            NextState::NegativeAccessRules(rules) => {
                self.negative_access_rules = rules;
                self.written.negative = true;
            }
            NextState::Enable(enable) => {
                self.enable = enable;
                self.written.enable = true;
            }
            NextState::Accompaniment(reference) => {
                self.accompaniment = Some(reference);
                self.written.accompaniment = true;
            }
        }
        self.writes = self.writes.wrapping_add(1);
    }

    /// Write Positive_Access_Rules, Negative_Access_Rules, Enable or
    /// Accompaniment, saving the result before the object serves it. A
    /// staged write takes the state already saved; any other saves now. A
    /// state that cannot be saved is refused with DEVICE /
    /// OPERATIONAL_PROBLEM and the old value stays.
    pub(super) fn write_saved(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let refused =
            |_| common::protocol_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM);
        if let Some(taken) = self.take_staged(property, array_index, &value) {
            return taken.map_err(refused);
        }
        // A write the object refuses, such as one past the end of an array,
        // leaves a write staged for another request alone (#1424).
        let next = self.next_state(&[], property, array_index, value)?;
        if self.storage.is_some() {
            let mut snapshot = self.snapshot();
            apply(&mut snapshot, &next);
            // This write will be made, so it supersedes a staged one: storage
            // goes back to the served state ahead of its save.
            self.with_storage(Storage::drop_staged);
            let storage = self.storage.as_mut().expect("checked above");
            storage.save_now(snapshot).map_err(refused)?;
        }
        self.install(next);
        Ok(())
    }

    /// Take the staged step for a write of `value` to `property` (at
    /// `array_index`), if it is the next one: the object then serves its
    /// state, or the save's error comes back. `None` when nothing staged is
    /// this write.
    fn take_staged(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        let mut storage = self.storage.take()?;
        let taken = storage
            .claim(property, array_index, value, self.writes)
            .map(|claimed| claimed.map(|next| self.install(next)));
        // Steps the request has still to take keep the state served now.
        storage.correct(|| self.snapshot());
        self.storage = Some(storage);
        taken
    }

    /// From the server's operation task at monotonic `now`: drop a staged
    /// write whose request is gone and queue the save of the served state
    /// (see the module docs). The save runs on the writer thread.
    pub(super) fn expire_staged_write(&mut self, now: Duration) {
        self.with_storage(|storage| storage.expire(now));
    }

    /// Run `f` on storage; then, if `f` dropped a staged write, queue the
    /// save that puts storage back to the served state. `None` without
    /// persistence.
    fn with_storage<R>(&mut self, f: impl FnOnce(&mut Storage) -> R) -> Option<R> {
        let mut storage = self.storage.take()?;
        let result = f(&mut storage);
        storage.correct(|| self.snapshot());
        self.storage = Some(storage);
        Some(result)
    }
}

/// Put the state `next` leaves into `snapshot`.
fn apply(snapshot: &mut AccessRightsSnapshot, next: &NextState) {
    match next {
        NextState::PositiveAccessRules(rules) => {
            snapshot.positive_access_rules = Some(rules.clone());
        }
        NextState::NegativeAccessRules(rules) => {
            snapshot.negative_access_rules = Some(rules.clone());
        }
        NextState::Enable(enable) => snapshot.enable = Some(*enable),
        NextState::Accompaniment(reference) => snapshot.accompaniment = Some(reference.clone()),
    }
}

impl DurableWrites for AccessRightsObject {
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

    /// Fold the request's writes of the saved properties into one staged
    /// save (#1423), so a head end that provisions both rule arrays and
    /// Enable in one WritePropertyMultiple saves once, off the guard.
    fn stage_writes(&mut self, writes: &[PendingWrite]) -> StageStep {
        if self.storage.is_none() || !writes.iter().any(|write| saved_property(write.property)) {
            return StageStep::Skip;
        }
        if let Some(wait) = self.with_storage(Storage::busy).flatten() {
            return StageStep::Busy(wait);
        }
        let steps = staged::steps(writes, |earlier, write| {
            if !saved_property(write.property) {
                return Ok(None);
            }
            let PendingWrite {
                property,
                array_index,
                value,
            } = write;
            self.next_state(earlier, *property, *array_index, value.clone())
                .map(Some)
        });
        let served = self.snapshot();
        let base = self.writes;
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
}
