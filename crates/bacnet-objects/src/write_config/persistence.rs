//! What a Timer or Channel keeps of the configuration peers write, and an
//! intrinsic reporter of its reporting settings (#2331), the storage trait
//! they share, and its file backend (#2243).
//!
//! See the [module documentation](super) for what is kept and when.

use crate::prelude::*;
#[cfg(feature = "std")]
use std::path::Path;

use bacnet_types::bitstring::{EventTransitionBits, LimitEnable};
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetObjectPropertyReference,
};
use bacnet_types::enums::{BinaryPV, NotifyType};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use crate::timer::StateChangeValues;

/// What one Timer, Channel or intrinsic reporter keeps across a restart:
/// the configuration peers wrote to it, tagged with what it belongs to.
// A Timer's seven values make its variant the larger; a snapshot is built
// once per save, so boxing it would buy nothing but a harder pattern match.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
// More object types will keep their configuration here (#2331), so a match
// outside the crate takes a wildcard arm.
#[non_exhaustive]
pub enum WriteConfigSnapshot {
    /// A Timer's.
    Timer(TimerWriteConfig),
    /// A Channel's.
    Channel(ChannelWriteConfig),
    /// The reporting settings of an object with intrinsic reporting (#2331).
    Reporting(ReportingWriteConfig),
}

/// The reporting settings every intrinsic reporter shares, as storage keeps
/// them once a write sets them (#2331), and its Description (#2364). Each
/// is `None` until then: what the application configures applies at each
/// start until then.
///
/// More settings may join it, so outside the crate it is built from
/// [`Default`] with its fields then set.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ReportingWriteConfig {
    /// Notification_Class.
    pub notification_class: Option<u32>,
    /// Event_Enable.
    pub event_enable: Option<EventTransitionBits>,
    /// Notify_Type.
    pub notify_type: Option<NotifyType>,
    /// Event_Detection_Enable.
    pub event_detection_enable: Option<bool>,
    /// Event_Message_Texts_Config, ordered TO_OFFNORMAL, TO_FAULT,
    /// TO_NORMAL; at most 16 KiB of UTF-8 each, as a durable object refuses
    /// a longer write.
    pub event_message_texts_config: Option<[String; 3]>,
    /// Event_Algorithm_Inhibit_Ref, when a write set one. A write of the
    /// unset form leaves `None`, which reopens as unset too.
    pub event_algorithm_inhibit_ref: Option<BACnetObjectPropertyReference>,
    /// Event_Algorithm_Inhibit.
    pub event_algorithm_inhibit: Option<bool>,
    /// Description, which the same objects keep beside these settings
    /// (#2364); at most 16 KiB of UTF-8, as a durable object refuses a
    /// longer write.
    pub description: Option<String>,
    /// The event algorithm parameters a write set (#2331).
    pub algorithm: AlgorithmWriteConfig,
}

/// The event algorithm parameters an intrinsic reporter keeps once a write
/// sets them (#2331), beside its reporting settings. Each is `None` until
/// then: what the application configures applies at each start until then.
///
/// An object type keeps only the parameters it has; storage holding another
/// is refused. `snapshot::write_config::keeps_parameter` names them per type.
/// More parameters will join it, so outside the crate it is built from
/// [`Default`] with its fields then set.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct AlgorithmWriteConfig {
    /// Time_Delay, in seconds.
    pub time_delay: Option<u32>,
    /// Time_Delay_Normal, in seconds.
    pub time_delay_normal: Option<u32>,
    /// High_Limit of an analog object; finite, as a write is.
    pub high_limit: Option<f32>,
    /// Low_Limit of an analog object; finite.
    pub low_limit: Option<f32>,
    /// Deadband of an analog object; finite and not negative.
    pub deadband: Option<f32>,
    /// Limit_Enable of an analog object.
    pub limit_enable: Option<LimitEnable>,
    /// Alarm_Value of a Binary Input or Value: INACTIVE or ACTIVE.
    pub alarm_value: Option<BinaryPV>,
    /// Alarm_Values of a Multi-state Input or Value, in list order: at most
    /// 1,024 states, as a write is, none of them zero. A state past the
    /// object's Number_Of_States, which a write refuses, is restored as
    /// stored: the state count isn't kept. A written empty list is kept as
    /// one.
    pub alarm_values: Option<Vec<u32>>,
}

/// What a Timer keeps across a restart.
///
/// More parts may join it, so outside the crate it is built from
/// [`Default`] with its fields then set.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
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
    /// Description, once a write has set it (#2364); at most 16 KiB of
    /// UTF-8.
    pub description: Option<String>,
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
///
/// More parts may join it, so outside the crate it is built from
/// [`Default`] with its fields then set.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
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
    /// Description, once a write has set it (#2364); at most 16 KiB of
    /// UTF-8.
    pub description: Option<String>,
}

/// A Channel's members and their delays, slot by slot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChannelMembers {
    /// List_Of_Object_Property_References, in array order.
    pub references: Vec<BACnetDeviceObjectPropertyReference>,
    /// Execution_Delay in milliseconds, one per reference.
    pub execution_delay: Vec<u32>,
}

impl WriteConfigSnapshot {
    /// The Description a write set, kept beside the configuration.
    pub(crate) fn description(&self) -> Option<&str> {
        match self {
            Self::Timer(config) => config.description.as_deref(),
            Self::Channel(config) => config.description.as_deref(),
            Self::Reporting(config) => config.description.as_deref(),
        }
    }

    /// Where the kept Description goes.
    pub(crate) fn description_mut(&mut self) -> &mut Option<String> {
        match self {
            Self::Timer(config) => &mut config.description,
            Self::Channel(config) => &mut config.description,
            Self::Reporting(config) => &mut config.description,
        }
    }
}

/// Application-owned storage for the configuration peers write to Timers,
/// Channels and intrinsic reporters, one object's at a time.
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
    use crate::snapshot::write_config::{self as format, MAX_LEN};

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
                file: ObjectFile::new(path.as_ref(), MAX_LEN as u64, "Write configuration file")?,
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
            Ok(Some(format::decode(object, &octets)?))
        }

        fn save(
            &self,
            object: ObjectIdentifier,
            snapshot: &WriteConfigSnapshot,
        ) -> Result<(), Error> {
            self.file.save_octets(format::encode(object, snapshot)?)
        }
    }
}
