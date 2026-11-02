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
    use bacnet_encoding::constructed::{
        decode_device_object_property_reference, decode_timer_state_change_value,
        encode_device_object_property_reference, encode_timer_state_change_value,
    };
    use bacnet_types::enums::ObjectType;
    use bytes::BytesMut;

    use crate::channel::{MAX_CHANNEL_MEMBERS, MAX_CONTROL_GROUPS};
    use crate::durable::file::ObjectFile;
    use crate::timer::MAX_TIMER_REFERENCES;

    /// The format's tag. One format serves both object types; the octet
    /// after the shared header names which one the body is for.
    const MAGIC: &[u8; 8] = b"RBNWCF01";
    /// A Timer's body follows.
    const KIND_TIMER: u8 = 1;
    /// A Channel's body follows.
    const KIND_CHANNEL: u8 = 2;
    /// A full member list, or reference list, of the longest references
    /// takes about 30 KiB; the cap leaves room for large constructed
    /// State_Change_Values. Anything larger is not a file this backend
    /// wrote, and a save that would pass it is refused.
    const MAX_FILE_BYTES: u64 = 1024 * 1024;
    /// The shared header: the magic tag and the object identifier.
    const HEADER_LEN: usize = 8 + 4;

    /// A file holding one Timer's or Channel's written configuration,
    /// replaced whole on each save the same way as
    /// [`FileNotificationClassPersistence`]: a failed save leaves the
    /// previous file in place, and a completed one survives a power loss.
    ///
    /// After the shared header (a magic tag and the object identifier) the
    /// file holds one octet naming the object type, then each part the
    /// object keeps behind a presence octet, 0 or 1. Variable parts state
    /// their count and their length in octets first, and the file must end
    /// where its last part does, so a file cut short anywhere, or holding a
    /// part of the wrong size, does not load. A Timer's set holds the
    /// priority, the seven State_Change_Values and the references; a
    /// Channel's parts are the members with their delays, Channel_Number,
    /// Control_Groups and Allow_Group_Delay_Inhibit, in that order. Values
    /// and references are in their BACnet encodings, integers big-endian. It
    /// does not coordinate between processes.
    ///
    /// [`FileNotificationClassPersistence`]: crate::notification_class::FileNotificationClassPersistence
    #[derive(Clone, Debug)]
    pub struct FileWriteConfigPersistence {
        file: ObjectFile,
    }

    impl FileWriteConfigPersistence {
        /// Keep the configuration in the file at `path`. Refuses an empty
        /// path.
        pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
            Ok(Self {
                file: ObjectFile::new(path.as_ref(), MAGIC, MAX_FILE_BYTES, "Write configuration")?,
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
            let Some(body) = self.file.load(object)? else {
                return Ok(None);
            };
            let mut reader = Reader {
                file: &self.file,
                body: &body,
                at: 0,
            };
            let snapshot = match (reader.u8()?, object.object_type()) {
                (KIND_TIMER, ObjectType::TIMER) => WriteConfigSnapshot::Timer(reader.timer()?),
                (KIND_CHANNEL, ObjectType::CHANNEL) => {
                    WriteConfigSnapshot::Channel(reader.channel()?)
                }
                _ => {
                    return Err(self
                        .file
                        .corrupt("holds another object type's configuration"));
                }
            };
            if reader.at != body.len() {
                return Err(self.file.corrupt("has octets past its end"));
            }
            Ok(Some(snapshot))
        }

        fn save(
            &self,
            object: ObjectIdentifier,
            snapshot: &WriteConfigSnapshot,
        ) -> Result<(), Error> {
            let mut body = BytesMut::new();
            match (snapshot, object.object_type()) {
                (WriteConfigSnapshot::Timer(config), ObjectType::TIMER) => {
                    body.extend_from_slice(&[KIND_TIMER]);
                    encode_timer(&mut body, config)?;
                }
                (WriteConfigSnapshot::Channel(config), ObjectType::CHANNEL) => {
                    body.extend_from_slice(&[KIND_CHANNEL]);
                    encode_channel(&mut body, config)?;
                }
                _ => {
                    return Err(Error::OutOfRange(format!(
                        "no write configuration of this kind for {object}"
                    )));
                }
            }
            if (HEADER_LEN + body.len()) as u64 > MAX_FILE_BYTES {
                return Err(Error::OutOfRange(
                    "write configuration too large to save".into(),
                ));
            }
            self.file.save(object, &body)
        }
    }

    /// Append `count`, as four octets, and then `section` with its length.
    fn put_section(body: &mut BytesMut, count: usize, section: &[u8]) -> Result<(), Error> {
        for value in [count, section.len()] {
            let value = u32::try_from(value)
                .map_err(|_| Error::OutOfRange("write configuration too large to save".into()))?;
            body.extend_from_slice(&value.to_be_bytes());
        }
        body.extend_from_slice(section);
        Ok(())
    }

    fn references_section(references: &[BACnetDeviceObjectPropertyReference]) -> BytesMut {
        let mut section = BytesMut::new();
        for reference in references {
            encode_device_object_property_reference(&mut section, reference);
        }
        section
    }

    fn encode_timer(body: &mut BytesMut, config: &TimerWriteConfig) -> Result<(), Error> {
        let Some(set) = &config.state_change else {
            body.extend_from_slice(&[0]);
            return Ok(());
        };
        // Only what a load takes back is saved.
        if !(1..=16).contains(&set.priority_for_writing)
            || set.references.len() > MAX_TIMER_REFERENCES
        {
            return Err(Error::OutOfRange("Timer configuration out of range".into()));
        }
        body.extend_from_slice(&[1, set.priority_for_writing]);
        let mut values = BytesMut::new();
        for value in &set.values {
            encode_timer_state_change_value(&mut values, value)?;
        }
        put_section(body, set.values.len(), &values)?;
        put_section(
            body,
            set.references.len(),
            &references_section(&set.references),
        )
    }

    fn encode_channel(body: &mut BytesMut, config: &ChannelWriteConfig) -> Result<(), Error> {
        let out_of_range = || Error::OutOfRange("Channel configuration out of range".into());
        match &config.members {
            None => body.extend_from_slice(&[0]),
            Some(members) => {
                let count = members.references.len();
                if count > MAX_CHANNEL_MEMBERS || members.execution_delay.len() != count {
                    return Err(out_of_range());
                }
                body.extend_from_slice(&[1]);
                put_section(body, count, &references_section(&members.references))?;
                for delay in &members.execution_delay {
                    body.extend_from_slice(&delay.to_be_bytes());
                }
            }
        }
        match config.channel_number {
            None => body.extend_from_slice(&[0]),
            Some(number) => {
                body.extend_from_slice(&[1]);
                body.extend_from_slice(&number.to_be_bytes());
            }
        }
        match &config.control_groups {
            None => body.extend_from_slice(&[0]),
            Some(groups) => {
                let count = u8::try_from(groups.len())
                    .ok()
                    .filter(|count| (1..=MAX_CONTROL_GROUPS).contains(&usize::from(*count)))
                    .ok_or_else(out_of_range)?;
                body.extend_from_slice(&[1, count]);
                for group in groups {
                    body.extend_from_slice(&group.to_be_bytes());
                }
            }
        }
        match config.allow_group_delay_inhibit {
            None => body.extend_from_slice(&[0]),
            Some(allow) => body.extend_from_slice(&[1, u8::from(allow)]),
        }
        Ok(())
    }

    /// The body read front to back. Every read past the end refuses the
    /// file as cut short.
    struct Reader<'a> {
        file: &'a ObjectFile,
        body: &'a [u8],
        at: usize,
    }

    impl<'a> Reader<'a> {
        fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
            let end = self
                .at
                .checked_add(len)
                .filter(|end| *end <= self.body.len())
                .ok_or_else(|| self.file.corrupt("is cut short"))?;
            let taken = &self.body[self.at..end];
            self.at = end;
            Ok(taken)
        }

        fn u8(&mut self) -> Result<u8, Error> {
            Ok(self.take(1)?[0])
        }

        fn u16(&mut self) -> Result<u16, Error> {
            Ok(u16::from_be_bytes(self.take(2)?.try_into().expect("two")))
        }

        fn u32(&mut self) -> Result<u32, Error> {
            Ok(u32::from_be_bytes(self.take(4)?.try_into().expect("four")))
        }

        /// A presence octet or a BOOLEAN: 0 or 1, nothing else.
        fn flag(&mut self) -> Result<bool, Error> {
            match self.u8()? {
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(self.file.corrupt("has a flag that is neither 0 nor 1")),
            }
        }

        /// A count of at most `cap` entries.
        fn count(&mut self, cap: usize) -> Result<usize, Error> {
            usize::try_from(self.u32()?)
                .ok()
                .filter(|count| *count <= cap)
                .ok_or_else(|| self.file.corrupt("holds more entries than the cap"))
        }

        /// A section of exactly `count` elements, each decoded by `decode`.
        fn elements<T>(
            &mut self,
            count: usize,
            decode: impl Fn(&[u8], usize) -> Result<(T, usize), Error>,
        ) -> Result<Vec<T>, Error> {
            let len = self.u32()? as usize;
            let section = self.take(len)?;
            let elements = self.file.decode_capped(section, count, decode)?;
            if elements.len() != count {
                return Err(self.file.corrupt("holds fewer entries than it states"));
            }
            Ok(elements)
        }

        fn timer(&mut self) -> Result<TimerWriteConfig, Error> {
            if !self.flag()? {
                return Ok(TimerWriteConfig::default());
            }
            let priority_for_writing = self.u8()?;
            if !(1..=16).contains(&priority_for_writing) {
                return Err(self.file.corrupt("holds a priority outside 1 to 16"));
            }
            let count = self.count(7)?;
            let values = self.elements(count, decode_timer_state_change_value)?;
            let values = values
                .try_into()
                .map_err(|_| self.file.corrupt("holds other than seven values"))?;
            let count = self.count(MAX_TIMER_REFERENCES)?;
            let references = self.elements(count, decode_device_object_property_reference)?;
            Ok(TimerWriteConfig {
                state_change: Some(TimerStateChange {
                    values,
                    references,
                    priority_for_writing,
                }),
            })
        }

        fn channel(&mut self) -> Result<ChannelWriteConfig, Error> {
            let members = if self.flag()? {
                let count = self.count(MAX_CHANNEL_MEMBERS)?;
                let references = self.elements(count, decode_device_object_property_reference)?;
                let execution_delay = (0..count)
                    .map(|_| self.u32())
                    .collect::<Result<Vec<_>, _>>()?;
                Some(ChannelMembers {
                    references,
                    execution_delay,
                })
            } else {
                None
            };
            let channel_number = if self.flag()? {
                Some(self.u16()?)
            } else {
                None
            };
            let control_groups = if self.flag()? {
                let count = usize::from(self.u8()?);
                if !(1..=MAX_CONTROL_GROUPS).contains(&count) {
                    return Err(self
                        .file
                        .corrupt("holds a Control_Groups size out of range"));
                }
                Some((0..count).map(|_| self.u32()).collect::<Result<_, _>>()?)
            } else {
                None
            };
            let allow_group_delay_inhibit = if self.flag()? {
                Some(self.flag()?)
            } else {
                None
            };
            Ok(ChannelWriteConfig {
                members,
                channel_number,
                control_groups,
                allow_group_delay_inhibit,
            })
        }
    }
}
