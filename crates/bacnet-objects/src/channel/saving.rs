//! Keeping the configuration a peer writes to a Channel across a restart
//! (#2243); see [`crate::write_config`].
//!
//! A Channel [`with_persistence`](ChannelObject::with_persistence) saves a
//! write to its members or Execution_Delay (kept together, as they share one
//! size), Channel_Number, Control_Groups, Allow_Group_Delay_Inhibit or
//! Description (#2364) before serving it, and a rebuilt Channel serves each
//! saved part over the one the application configures. Present_Value,
//! Write_Status and Reliability are not kept.

use crate::prelude::*;
use alloc::sync::Arc;

use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;

use super::arrays::{self, ChannelConfig, Written};
use super::{ChannelObject, MAX_CHANNEL_MEMBERS};
use crate::durable::PendingWrite;
use crate::durable::staged::StagedSaves;
use crate::write_config::description;
use crate::write_config::saving::{self, ConfigWrites, Storage, impl_config_durable};
use crate::write_config::{
    ChannelMembers, ChannelWriteConfig, WriteConfigPersistence, WriteConfigSnapshot,
};

/// Refuse a member list a Channel can't hold, as
/// [`ChannelObject::set_members`] documents.
pub(super) fn check_members(members: &[BACnetDeviceObjectPropertyReference]) -> Result<(), Error> {
    if members.len() > MAX_CHANNEL_MEMBERS {
        return Err(arrays::no_space_error());
    }
    crate::device_reference::check_device_members(members)
}

impl ChannelObject {
    /// A Channel that keeps the configuration peers write in
    /// `persistence`, starting from what is saved there for this object,
    /// if anything. Otherwise as [`new`](Self::new); a saved Channel_Number
    /// wins over `channel_number`.
    ///
    /// A write to List_Of_Object_Property_References, Execution_Delay,
    /// Channel_Number, Control_Groups, Allow_Group_Delay_Inhibit or
    /// Description (#2364) then saves before the Channel serves it, and is
    /// refused with DEVICE / OPERATIONAL_PROBLEM, leaving the property as it
    /// was, when the save fails; a Description past 16 KiB is refused with
    /// RESOURCES / NO_SPACE_TO_WRITE_PROPERTY. A part a write saved is no
    /// longer replaced by its setter.
    ///
    /// Fails when what storage holds can't be loaded, belongs to another
    /// object type, or holds what the setters would refuse or a
    /// Description past 16 KiB.
    pub fn with_persistence(
        instance: u32,
        name: impl Into<String>,
        channel_number: u16,
        persistence: Arc<dyn WriteConfigPersistence>,
    ) -> Result<Self, Error> {
        let mut channel = Self::new(instance, name, channel_number)?;
        let oid = channel.oid;
        match persistence.load(oid)? {
            None => {}
            Some(WriteConfigSnapshot::Channel(saved)) => channel.restore(saved)?,
            Some(_) => {
                return Err(Error::Encoding(
                    "saved write configuration is not a Channel's".into(),
                ));
            }
        }
        channel.config_storage = Some(StagedSaves::new(saving::writer(oid, persistence)));
        Ok(channel)
    }

    /// Serve the parts `saved` holds.
    fn restore(&mut self, saved: ChannelWriteConfig) -> Result<(), Error> {
        description::check_saved(saved.description.as_deref())?;
        if let Some(text) = &saved.description {
            self.description.clone_from(text);
        }
        let config = &mut self.config;
        config.description = saved.description;
        if let Some(members) = saved.members {
            check_members(&members.references)?;
            if members.execution_delay.len() != members.references.len() {
                return Err(Error::Encoding(
                    "saved Channel members and delays differ in number".into(),
                ));
            }
            config.learned = vec![None; members.references.len()];
            config.members = members.references;
            config.execution_delay = members.execution_delay;
            config.written.members = true;
        }
        if let Some(number) = saved.channel_number {
            config.channel_number = number;
            config.written.number = true;
        }
        if let Some(groups) = saved.control_groups {
            arrays::check_group_count(groups.len())?;
            config.control_groups = groups;
            config.written.groups = true;
        }
        if let Some(allow) = saved.allow_group_delay_inhibit {
            config.allow_group_delay_inhibit = allow;
            config.written.inhibit = true;
        }
        Ok(())
    }

    /// The parts storage holds because a write set them, now or before a
    /// restart; their setters leave them alone. None without persistence.
    pub(super) fn saved(&self) -> Written {
        match self.config_storage {
            Some(_) => self.config.written,
            None => Written::default(),
        }
    }

    /// Drop every part a write saved, Description included: storage keeps
    /// none from now on, so the
    /// setters apply again. The configuration served now stays until they,
    /// or a write, change it. Saves before it returns; a save that fails
    /// keeps the written parts and returns storage's error. Does nothing
    /// without storage or a written part.
    /// Call it before the object joins the database, as the setters are,
    /// not under the database guard: it waits for its save.
    pub fn clear_written_config(&mut self) -> Result<(), Error> {
        saving::clear(self)
    }

    /// Block until the saves queued so far have run. Dropping the Channel
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
}

impl ConfigWrites for ChannelObject {
    type Config = ChannelConfig;

    fn storage(&mut self) -> &mut Option<Storage<ChannelConfig>> {
        &mut self.config_storage
    }

    #[cfg(feature = "durable-writes")]
    fn storage_ref(&self) -> Option<&Storage<ChannelConfig>> {
        self.config_storage.as_ref()
    }

    fn config(&self) -> Option<&ChannelConfig> {
        Some(&self.config)
    }

    fn install(&mut self, config: ChannelConfig) {
        if let Some(text) = &config.description {
            self.description.clone_from(text);
        }
        self.config = config;
        self.config_writes = self.config_writes.wrapping_add(1);
    }

    fn writes(&self) -> u64 {
        self.config_writes
    }

    fn snapshot(config: Option<&ChannelConfig>) -> WriteConfigSnapshot {
        let Some(config) = config else {
            return WriteConfigSnapshot::Channel(ChannelWriteConfig::default());
        };
        let written = config.written;
        WriteConfigSnapshot::Channel(ChannelWriteConfig {
            members: written.members.then(|| ChannelMembers {
                references: config.members.clone(),
                execution_delay: config.execution_delay.clone(),
            }),
            channel_number: written.number.then_some(config.channel_number),
            control_groups: written.groups.then(|| config.control_groups.clone()),
            allow_group_delay_inhibit: written.inhibit.then_some(config.allow_group_delay_inhibit),
            description: config.description.clone(),
        })
    }

    #[cfg(feature = "durable-writes")]
    fn saves(property: PropertyIdentifier) -> bool {
        ChannelConfig::written_by(property) || property == PropertyIdentifier::DESCRIPTION
    }

    fn unwrite(config: &mut ChannelConfig) {
        config.written = Written::default();
        config.description = None;
    }

    fn next(config: &ChannelConfig, write: &PendingWrite) -> Result<ChannelConfig, Error> {
        let mut next = config.clone();
        if write.property == PropertyIdentifier::DESCRIPTION {
            next.description = Some(description::written(&write.value)?);
        } else {
            next.write(write.property, write.array_index, write.value.clone())?;
        }
        Ok(next)
    }
}

impl_config_durable!(ChannelObject);
