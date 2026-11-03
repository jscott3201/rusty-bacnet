//! What a Timer or Channel keeps of the configuration peers write, the
//! storage trait both share, and its file backend (#2243).
//!
//! See the [module documentation](super) for what is kept and when.

use crate::prelude::*;
#[cfg(feature = "std")]
use std::path::Path;

use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use crate::timer::StateChangeValues;

/// What one Timer or Channel keeps across a restart: the configuration
/// peers wrote to it, tagged with the object type it belongs to.
// A Timer's seven values make its variant the larger; a snapshot is built
// once per save, so boxing it would buy nothing but a harder pattern match.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum WriteConfigSnapshot {
    /// A Timer's.
    Timer(TimerWriteConfig),
    /// A Channel's.
    Channel(ChannelWriteConfig),
}

/// What a Timer keeps across a restart.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimerWriteConfig {
    /// State_Change_Values, List_Of_Object_Property_References and
    /// Priority_For_Writing, once a write has set either of the first two.
    /// `None` until then: what the application configures applies at each
    /// start.
    pub state_change: Option<TimerStateChange>,
    /// Default_Timeout in milliseconds, once a write has set it (#2312).
    /// `None` until then. A rebuilt Timer serves it only while it lies
    /// within the Timer's Min_Pres_Value..=Max_Pres_Value range; see
    /// [`TimerObject::set_pres_value_range`].
    ///
    /// [`TimerObject::set_pres_value_range`]: crate::timer::TimerObject::set_pres_value_range
    pub default_timeout: Option<u64>,
}

/// The three properties a Timer serves together (Clause 12.57, Table 12-75
/// footnote 5), as storage keeps them.
#[derive(Clone, Debug, PartialEq)]
pub struct TimerStateChange {
    /// State_Change_Values: element 0 is written on IDLE_TO_RUNNING, through
    /// element 6 on EXPIRED_TO_RUNNING.
    pub values: StateChangeValues,
    /// List_Of_Object_Property_References, in list order.
    pub references: Vec<BACnetDeviceObjectPropertyReference>,
    /// Priority_For_Writing, 1 to 16, as the Timer served it with the other
    /// two. Peers can't write it; it is kept so the set comes back whole.
    pub priority_for_writing: u8,
}

/// What a Channel keeps across a restart. Each member is `None` until a
/// write sets it: what the application configures applies at each start
/// until then.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChannelWriteConfig {
    /// List_Of_Object_Property_References with Execution_Delay, once a
    /// write has set either: the two have one size, so they are kept as one.
    pub members: Option<ChannelMembers>,
    /// Channel_Number, once a write has set it.
    pub channel_number: Option<u16>,
    /// Control_Groups, once a write has set it.
    pub control_groups: Option<Vec<u32>>,
    /// Allow_Group_Delay_Inhibit, once a write has set it.
    pub allow_group_delay_inhibit: Option<bool>,
}

/// A Channel's members and their delays, slot by slot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChannelMembers {
    /// List_Of_Object_Property_References, in array order.
    pub references: Vec<BACnetDeviceObjectPropertyReference>,
    /// Execution_Delay in milliseconds, one per reference.
    pub execution_delay: Vec<u32>,
}

/// Application-owned storage for the configuration peers write to Timers
/// and Channels, one object's at a time.
///
/// The object calls `load` once, as it is built, and `save` from its writer
/// thread, one call at a time. A written value lands in storage a moment
/// before the object serves it: see [storage leads the served
/// state](crate::durable#storage-leads-the-served-state). A backend that
/// refuses a save returns an error and keeps what it held; the write that
/// asked for it is then refused.
///
/// An implementation must not own the object it serves, or the database
/// holding it: the object's writer thread calls it, and dropping the object
/// there would wait for that same thread to end, forever.
pub trait WriteConfigPersistence: Send + Sync {
    /// What was last saved for `object`, or `None` when nothing was saved.
    fn load(&self, object: ObjectIdentifier) -> Result<Option<WriteConfigSnapshot>, Error>;

    /// Durably replace what is saved for `object` with `snapshot`.
    fn save(&self, object: ObjectIdentifier, snapshot: &WriteConfigSnapshot) -> Result<(), Error>;
}

#[cfg(feature = "std")]
pub use file::FileWriteConfigPersistence;

#[cfg(feature = "std")]
mod file {
    use super::*;
    use crate::durable::file::ObjectFile;
    use crate::snapshot::write_config::{self as format, MAGIC, MAX_LEN};

    /// A file holding one Timer's or Channel's written configuration,
    /// replaced whole on each save the same way as
    /// [`FileNotificationClassPersistence`]: a failed save leaves the
    /// previous file in place, and a completed one survives a power loss.
    /// It does not coordinate between processes.
    ///
    /// The file holds what [`snapshot::write_config::encode`] returns; its
    /// module documents the format.
    ///
    /// [`FileNotificationClassPersistence`]: crate::notification_class::FileNotificationClassPersistence
    /// [`snapshot::write_config::encode`]: crate::snapshot::write_config::encode
    #[derive(Clone, Debug)]
    pub struct FileWriteConfigPersistence {
        file: ObjectFile,
    }

    impl FileWriteConfigPersistence {
        /// Keep the configuration in the file at `path`. Refuses an empty
        /// path.
        pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
            Ok(Self {
                file: ObjectFile::new(
                    path.as_ref(),
                    &MAGIC,
                    MAX_LEN as u64,
                    "Write configuration",
                )?,
            })
        }

        /// Test support, not a stable API: keep the file in `memory`'s entry
        /// 0 instead of on disk, decoding and encoding it as on disk
        /// (#2233). [`path`](Self::path) is then a placeholder. Nothing
        /// reaches disk: the state is gone when the process ends, so never
        /// use this outside tests.
        #[cfg(feature = "test-support")]
        #[doc(hidden)]
        pub fn in_memory(memory: crate::durable::MemoryFiles) -> Self {
            let mut store = Self::new("in-memory").expect("a path that isn't empty");
            store.file.keep_in(memory);
            store
        }

        /// The file the configuration is kept in.
        pub fn path(&self) -> &Path {
            self.file.path()
        }
    }

    impl WriteConfigPersistence for FileWriteConfigPersistence {
        fn load(&self, object: ObjectIdentifier) -> Result<Option<WriteConfigSnapshot>, Error> {
            let Some(octets) = self.file.read()? else {
                return Ok(None);
            };
            format::decode(object, &octets)
                .map(Some)
                .map_err(|error| self.file.error(error))
        }

        fn save(
            &self,
            object: ObjectIdentifier,
            snapshot: &WriteConfigSnapshot,
        ) -> Result<(), Error> {
            let octets =
                format::encode(object, snapshot).map_err(|error| self.file.error(error))?;
            self.file.save_octets(octets)
        }
    }
}
