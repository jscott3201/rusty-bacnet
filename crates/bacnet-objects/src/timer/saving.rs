//! Keeping a written State_Change_Values, List_Of_Object_Property_References
//! and Default_Timeout across a restart (#2243, #2312); see
//! [`crate::write_config`].
//!
//! A Timer [`with_persistence`](TimerObject::with_persistence) saves its set
//! whenever a write changes either property, Priority_For_Writing with them,
//! a written Default_Timeout and a written Description (#2364), and a
//! rebuilt Timer serves what was saved.
//! A saved Default_Timeout outside the range the application gives the
//! Timer is dropped for the application's value (see
//! [`TimerObject::set_pres_value_range`]). The Timer's state machine is not
//! kept: it still starts IDLE.

use crate::prelude::*;
use alloc::sync::Arc;

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::state_change::{TimerSet, check_set};
use super::{TimerConfig, TimerObject};
use crate::common;
use crate::durable::PendingWrite;
use crate::durable::staged::StagedSaves;
use crate::write_config::description;
use crate::write_config::saving::{self, ConfigWrites, Storage, impl_config_durable};
use crate::write_config::{
    TimerStateChange, TimerWriteConfig, WriteConfigPersistence, WriteConfigSnapshot,
};

impl TimerObject {
    /// A Timer that keeps a written State_Change_Values and
    /// List_Of_Object_Property_References in `persistence`, with
    /// Priority_For_Writing, and a written Default_Timeout and Description
    /// (#2364), starting from what is saved there for this object, if
    /// anything. Otherwise as [`new`](Self::new).
    ///
    /// Writing one of them then saves it before the Timer serves it, and is
    /// refused with DEVICE / OPERATIONAL_PROBLEM, leaving the value as it
    /// was, when the save fails; a Description past 16 KiB is refused with
    /// RESOURCES / NO_SPACE_TO_WRITE_PROPERTY. Once a write has saved a set,
    /// [`set_state_change`](Self::set_state_change) no longer replaces it,
    /// and once one has saved a Default_Timeout,
    /// [`set_default_timeout`](Self::set_default_timeout) no longer does.
    ///
    /// Fails when what storage holds can't be loaded, belongs to another
    /// object type, or holds a set `set_state_change` would refuse or a
    /// Description past 16 KiB. A saved
    /// Default_Timeout of zero, which no range holds, is dropped with a
    /// warning. One past the range a new Timer takes is held aside, unserved
    /// and still kept in storage, until
    /// [`set_pres_value_range`](Self::set_pres_value_range) gives the range
    /// that decides it; without that call it stays held.
    pub fn with_persistence(
        instance: u32,
        name: impl Into<String>,
        persistence: Arc<dyn WriteConfigPersistence>,
    ) -> Result<Self, Error> {
        let mut timer = Self::new(instance, name)?;
        let oid = timer.oid;
        let saved = match persistence.load(oid)? {
            None => TimerWriteConfig::default(),
            Some(WriteConfigSnapshot::Timer(saved)) => saved,
            Some(_) => {
                return Err(Error::Encoding(
                    "saved write configuration is not a Timer's".into(),
                ));
            }
        };
        description::check_saved(saved.description.as_deref())?;
        if let Some(saved) = saved.state_change {
            check_set(&saved.values, &saved.references, saved.priority_for_writing)?;
            timer.config.set = Some(TimerSet {
                values: saved.values,
                references: saved.references,
                priority: saved.priority_for_writing,
                written: true,
            });
        }
        if let Some(text) = saved.description {
            timer.description.clone_from(&text);
            timer.config.description = Some(text);
        }
        timer.config_storage = Some(StagedSaves::new(saving::writer(oid, persistence)));
        match saved.default_timeout {
            None => {}
            Some(timeout) if timer.in_range(timeout) => {
                timer.config.default_timeout = timeout;
                timer.config.timeout_written = true;
            }
            // No Timer takes zero: the minimum is at least 1.
            Some(0) => timer.drop_saved_timeout(0),
            // An application range may reach it.
            Some(timeout) => {
                log_debug!(timer = %oid, timeout, "Saved Default_Timeout held until the range is set");
                timer.config.held_timeout = Some(timeout);
            }
        }
        Ok(timer)
    }

    /// Whether storage holds a set a write made, now or before a restart.
    /// [`set_state_change`](Self::set_state_change) then leaves the set
    /// alone. Always false without persistence.
    pub fn state_change_saved(&self) -> bool {
        self.config_storage.is_some() && self.config.set.as_ref().is_some_and(|set| set.written)
    }

    /// Whether the Timer serves a Default_Timeout a write made and storage
    /// holds, now or before a restart.
    /// [`set_default_timeout`](Self::set_default_timeout) then leaves the
    /// served value alone. False for a saved value held aside (see
    /// [`with_persistence`](Self::with_persistence)) until the range serves
    /// it, and always without persistence.
    pub fn default_timeout_saved(&self) -> bool {
        self.config_storage.is_some() && self.config.timeout_written
    }

    /// Drop everything a write saved, the set, Default_Timeout and
    /// Description: storage
    /// keeps nothing from now on, so the setters apply again. What the
    /// Timer serves now stays until they, or a write, change it. Saves
    /// before it returns; a save that fails keeps the written values and
    /// returns storage's error. Does nothing without storage or a written
    /// value.
    /// Call it before the object joins the database, as the setters are,
    /// not under the database guard: it waits for its save.
    pub fn clear_written_config(&mut self) -> Result<(), Error> {
        saving::clear(self)
    }

    /// Block until the saves queued so far have run. Dropping the Timer
    /// waits for them too.
    pub fn wait_for_saves(&self) {
        if let Some(storage) = &self.config_storage {
            storage.wait_idle();
        }
    }

    /// The application changed the configuration: count it, and let a
    /// staged step worked out from the old one give way.
    pub(super) fn config_changed(&mut self) {
        saving::changed(self);
        self.config_writes = self.config_writes.wrapping_add(1);
    }

    /// The saved Default_Timeout, `saved`, lies outside the range, where
    /// Clause 12.57.16 doesn't let the Timer serve it: serve the
    /// application's value instead, and save without the written one, so it
    /// doesn't come back (#2312).
    pub(super) fn drop_saved_timeout(&mut self, saved: u64) {
        log_warn!(
            object = %self.oid,
            saved = saved,
            min = self.config.min_pres_value,
            max = self.config.max_pres_value,
            "Saved Default_Timeout lies outside Min_Pres_Value..=Max_Pres_Value; the application's value applies"
        );
        self.config.default_timeout = self.configured_timeout;
        self.config.timeout_written = false;
        self.config.held_timeout = None;
        let snapshot = Self::snapshot(Some(&self.config));
        if let Some(storage) = &mut self.config_storage {
            storage.submit_coalescing(snapshot);
        }
    }
}

impl ConfigWrites for TimerObject {
    type Config = TimerConfig;

    fn storage(&mut self) -> &mut Option<Storage<TimerConfig>> {
        &mut self.config_storage
    }

    #[cfg(feature = "durable-writes")]
    fn storage_ref(&self) -> Option<&Storage<TimerConfig>> {
        self.config_storage.as_ref()
    }

    fn config(&self) -> Option<&TimerConfig> {
        Some(&self.config)
    }

    fn install(&mut self, config: TimerConfig) {
        if let Some(text) = &config.description {
            self.description.clone_from(text);
        }
        self.config = config;
        self.config_writes = self.config_writes.wrapping_add(1);
    }

    fn writes(&self) -> u64 {
        self.config_writes
    }

    fn snapshot(config: Option<&TimerConfig>) -> WriteConfigSnapshot {
        let Some(config) = config else {
            return WriteConfigSnapshot::Timer(TimerWriteConfig::default());
        };
        WriteConfigSnapshot::Timer(TimerWriteConfig {
            state_change: config.set.as_ref().filter(|set| set.written).map(|set| {
                TimerStateChange {
                    values: set.values.clone(),
                    references: set.references.clone(),
                    priority_for_writing: set.priority,
                }
            }),
            // A held value is kept in storage, unserved.
            default_timeout: if config.timeout_written {
                Some(config.default_timeout)
            } else {
                config.held_timeout
            },
            description: config.description.clone(),
        })
    }

    #[cfg(feature = "durable-writes")]
    fn saves(property: PropertyIdentifier) -> bool {
        TimerSet::written_by(property)
            || matches!(
                property,
                PropertyIdentifier::DEFAULT_TIMEOUT | PropertyIdentifier::DESCRIPTION
            )
    }

    /// A set write on a Timer without the set is UNKNOWN_PROPERTY: the
    /// Timer doesn't serve the property.
    fn next(config: &TimerConfig, write: &PendingWrite) -> Result<TimerConfig, Error> {
        if write.property == PropertyIdentifier::DESCRIPTION {
            return Ok(TimerConfig {
                description: Some(description::written(&write.value)?),
                ..config.clone()
            });
        }
        if write.property != PropertyIdentifier::DEFAULT_TIMEOUT {
            let set = config
                .set
                .as_ref()
                .ok_or_else(common::unknown_property_error)?;
            let set = set.after_write(write.property, write.array_index, &write.value)?;
            return Ok(TimerConfig {
                set: Some(set),
                ..config.without_set()
            });
        }
        let PropertyValue::Unsigned(timeout) = write.value else {
            return Err(common::invalid_data_type_error());
        };
        if !config.in_range(timeout) {
            return Err(common::value_out_of_range_error());
        }
        Ok(TimerConfig {
            default_timeout: timeout,
            timeout_written: true,
            held_timeout: None,
            ..config.clone()
        })
    }

    fn unwrite(config: &mut TimerConfig) {
        if let Some(set) = &mut config.set {
            set.written = false;
        }
        config.timeout_written = false;
        config.held_timeout = None;
        config.description = None;
    }
}

impl TimerConfig {
    /// A copy without the set, which a write to the set replaces anyway.
    fn without_set(&self) -> Self {
        Self {
            set: None,
            default_timeout: self.default_timeout,
            timeout_written: self.timeout_written,
            held_timeout: self.held_timeout,
            min_pres_value: self.min_pres_value,
            max_pres_value: self.max_pres_value,
            description: self.description.clone(),
        }
    }
}

impl_config_durable!(TimerObject);
