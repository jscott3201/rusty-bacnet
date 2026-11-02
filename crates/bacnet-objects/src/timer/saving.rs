//! Keeping a written State_Change_Values and List_Of_Object_Property_References
//! across a restart (#2243); see [`crate::write_config`].
//!
//! A Timer [`with_persistence`](TimerObject::with_persistence) saves its set
//! whenever a write changes either property, Priority_For_Writing with them,
//! and a rebuilt Timer serves the saved set. The Timer's state machine is
//! not kept: it still starts IDLE.

use crate::prelude::*;
use alloc::sync::Arc;

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;

use super::TimerObject;
use super::state_change::{StateChange, TimerSet, check_set};
use crate::durable::PendingWrite;
use crate::durable::staged::StagedSaves;
use crate::write_config::saving::{self, ConfigWrites, Storage, impl_config_durable};
use crate::write_config::{
    TimerStateChange, TimerWriteConfig, WriteConfigPersistence, WriteConfigSnapshot,
};

impl TimerObject {
    /// A Timer that keeps a written State_Change_Values and
    /// List_Of_Object_Property_References in `persistence`, with
    /// Priority_For_Writing, starting from the set saved there for this
    /// object, if any. Otherwise as [`new`](Self::new).
    ///
    /// A write to either property then saves the set before the Timer serves
    /// it, and is refused with DEVICE / OPERATIONAL_PROBLEM, leaving the set
    /// as it was, when the save fails. Once a write has saved a set,
    /// [`set_state_change`](Self::set_state_change) no longer replaces it.
    ///
    /// Fails when what storage holds can't be loaded, belongs to another
    /// object type, or holds a set `set_state_change` would refuse.
    pub fn with_persistence(
        instance: u32,
        name: impl Into<String>,
        persistence: Arc<dyn WriteConfigPersistence>,
    ) -> Result<Self, Error> {
        let mut timer = Self::new(instance, name)?;
        let oid = timer.oid;
        match persistence.load(oid)? {
            None | Some(WriteConfigSnapshot::Timer(TimerWriteConfig { state_change: None })) => {}
            Some(WriteConfigSnapshot::Timer(TimerWriteConfig {
                state_change: Some(saved),
            })) => {
                check_set(&saved.values, &saved.references, saved.priority_for_writing)?;
                timer.state_change = Some(StateChange::new(TimerSet {
                    values: saved.values,
                    references: saved.references,
                    priority: saved.priority_for_writing,
                    written: true,
                }));
            }
            Some(_) => {
                return Err(Error::Encoding(
                    "saved write configuration is not a Timer's".into(),
                ));
            }
        }
        timer.config_storage = Some(StagedSaves::new(saving::writer(oid, persistence)));
        Ok(timer)
    }

    /// Whether storage holds a set a write made, now or before a restart.
    /// [`set_state_change`](Self::set_state_change) then leaves the set
    /// alone. Always false without persistence.
    pub fn state_change_saved(&self) -> bool {
        self.config_storage.is_some()
            && self
                .state_change
                .as_ref()
                .is_some_and(|change| change.set.written)
    }

    /// Drop the set a write saved: storage keeps none from now on, so
    /// [`set_state_change`](Self::set_state_change) applies again. The set
    /// served now stays until it, or a write, changes it. Saves before it
    /// returns; a save that fails keeps the written set and returns
    /// storage's error. Does nothing without storage or a written set.
    /// Call it before the object joins the database, as the setters are,
    /// not under the database guard: it waits for its save.
    pub fn clear_written_state_change(&mut self) -> Result<(), Error> {
        saving::clear(self)
    }

    /// Block until the saves queued so far have run. Dropping the Timer
    /// waits for them too.
    pub fn wait_for_saves(&self) {
        if let Some(storage) = &self.config_storage {
            storage.wait_idle();
        }
    }

    /// The application changed the set: count it, and let a staged step
    /// worked out from the old set give way.
    pub(super) fn config_changed(&mut self) {
        saving::changed(self);
        self.config_writes = self.config_writes.wrapping_add(1);
    }
}

impl ConfigWrites for TimerObject {
    type Config = TimerSet;

    fn storage(&mut self) -> &mut Option<Storage<TimerSet>> {
        &mut self.config_storage
    }

    fn storage_ref(&self) -> Option<&Storage<TimerSet>> {
        self.config_storage.as_ref()
    }

    fn config(&self) -> Option<&TimerSet> {
        self.state_change.as_ref().map(|change| &change.set)
    }

    fn install(&mut self, set: TimerSet) {
        match &mut self.state_change {
            Some(change) => change.set = set,
            None => self.state_change = Some(StateChange::new(set)),
        }
        self.config_writes = self.config_writes.wrapping_add(1);
    }

    fn writes(&self) -> u64 {
        self.config_writes
    }

    fn snapshot(set: Option<&TimerSet>) -> WriteConfigSnapshot {
        WriteConfigSnapshot::Timer(TimerWriteConfig {
            state_change: set.filter(|set| set.written).map(|set| TimerStateChange {
                values: set.values.clone(),
                references: set.references.clone(),
                priority_for_writing: set.priority,
            }),
        })
    }

    fn saves(property: PropertyIdentifier) -> bool {
        TimerSet::written_by(property)
    }

    fn next(set: &TimerSet, write: &PendingWrite) -> Result<TimerSet, Error> {
        set.after_write(write.property, write.array_index, &write.value)
    }

    fn unwrite(set: &mut TimerSet) {
        set.written = false;
    }
}

impl_config_durable!(TimerObject);
