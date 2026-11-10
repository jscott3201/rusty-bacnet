//! A Timer's or Channel's written configuration, or an intrinsic
//! reporter's written reporting settings (`RBNWCF01`), as
//! `FileWriteConfigPersistence` (with std) writes its file.
//!
//! After the magic tag and the four-octet object identifier the file holds
//! one octet naming the body's kind, then each part the object keeps behind
//! a presence octet, 0 or 1. Variable parts state their count and their
//! length in octets first, and the body must end where its last part does,
//! so octets cut short anywhere, or holding a part of the wrong size, don't
//! decode. Values and references are in their BACnet encodings, integers
//! big-endian.
//!
//! - [`KIND_TIMER`]: a Timer's set (the priority, the seven
//!   State_Change_Values and the references), then Default_Timeout in eight
//!   octets.
//! - [`KIND_CHANNEL`]: a Channel's members with their delays,
//!   Channel_Number, Control_Groups and Allow_Group_Delay_Inhibit, in that
//!   order.
//! - [`KIND_TIMER_SET_ONLY`]: a Timer's set alone, as saved before
//!   Default_Timeout was kept. It still decodes, with no Default_Timeout
//!   written; [`encode`] never writes it.
//! - [`KIND_REPORTING`]: the reporting settings an object with intrinsic
//!   reporting keeps (#2331): Notification_Class (four octets),
//!   Event_Enable (one octet of transition bits), Notify_Type (its
//!   four-octet enumeration value), Event_Detection_Enable,
//!   Event_Message_Texts_Config (three UTF-8 strings, each its length and
//!   then its octets), Event_Algorithm_Inhibit_Ref (its length and its
//!   BACnet encoding) and Event_Algorithm_Inhibit, in that order. Only the
//!   object types that keep them take this kind (see [`keeps_reporting`])
//!   and kind 6, which goes on from it.
//! - [`KIND_WITH_DESCRIPTION`]: a written Description (#2364), its length
//!   and then its UTF-8 octets, at most 16 KiB, followed by the kind octet
//!   and body of one of kinds 2, 3, 4 and 6. [`encode`] writes it only
//!   for a snapshot holding a Description, so a file without one keeps its
//!   plain kind, octet for octet, and a build that predates kind 5 still
//!   reads it; one holding a Description is refused there as an unknown
//!   kind.
//! - [`KIND_REPORTING_PARAMETERS`]: the body of kind 4, then the event
//!   algorithm parameters the object keeps (#2331): a four-octet presence
//!   mask, then each parameter whose bit is set, in bit order. Bit 0 is
//!   Time_Delay and bit 1 Time_Delay_Normal (four octets each); bits 2, 3
//!   and 4 are High_Limit, Low_Limit and Deadband (the four octets of an
//!   IEEE 754 single each); bit 5 is Limit_Enable (one octet, bit 0 the low
//!   limit and bit 1 the high); bit 6 is a binary object's Alarm_Value (one
//!   octet, 0 or 1); bit 7 is Alarm_Values as a list of states (a count of
//!   at most 1,024, then each state in four octets; a count past the cap is
//!   refused before a state is read, and the count fixes the row's length,
//!   so a row cut short or going on is refused too); bits 8 and 9 are an
//!   Access Door's Fault_Values and Masked_Alarm_Values, in the form of
//!   bit 7; bits 10 and 11 are an Event Enrollment's Event_Parameters and
//!   Fault_Parameters, each a length of at most 64 KiB and then the value
//!   in its BACnet encoding (a length past the cap is refused before the
//!   value is read; the octets must be one whole value, in the encoding a
//!   save writes, so a load followed by a save leaves them as they were).
//!   An Event Enrollment keeps bit 1 without bit 0: its Time_Delay is a
//!   member of its Event_Parameters. What a listed state may be is the
//!   rule of the object type the
//!   file's header names: on a multi-state object any state but zero, and
//!   on an Access Door or an Access Zone a member of the type's
//!   enumeration, named or proprietary, other than NORMAL. A file is
//!   opened for one object and refused for another, so its lists are
//!   never read under another type's rule. [`keeps_parameter`] names the
//!   parameters each object type keeps. [`encode`] writes this kind only for a
//!   snapshot holding a parameter, so one without keeps kind 4, octet for
//!   octet, and a build that predates kind 6 refuses a file with one as an
//!   unknown kind. A later parameter takes the next bit, and a later build
//!   may keep a known one for more object types: a set bit this build
//!   doesn't keep for the object's type, known to it or not, refuses the
//!   file as an unknown kind too, before a parameter is read, and is never
//!   damage. No set bit at all is damage, as is a kept parameter holding a
//!   value no write leaves. Kind 5 wraps this kind as it wraps kind 4.
//!
//! Every length and count above is four octets, big-endian.
//!
//! A kind must match the object's type, and an unknown kind is refused, so
//! a later kind can be added without a new magic tag.

use crate::prelude::*;
use bacnet_encoding::constructed::{
    decode_device_object_property_reference, decode_timer_state_change_value,
    encode_device_object_property_reference, encode_timer_state_change_value,
};
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::framing::{self, HEADER_LEN};
use super::{SnapshotError, SnapshotErrorKind};
use crate::channel::{MAX_CHANNEL_MEMBERS, MAX_CONTROL_GROUPS};
use crate::timer::MAX_TIMER_REFERENCES;
use crate::write_config::description::MAX_OCTETS as MAX_DESCRIPTION_OCTETS;
use crate::write_config::{
    ChannelMembers, ChannelWriteConfig, TimerStateChange, TimerWriteConfig, WriteConfigSnapshot,
};

pub(crate) mod parameters;
mod reporting;

pub use parameters::keeps_parameter;
#[cfg(feature = "event-reporting")]
pub(crate) use reporting::invalid as reporting_invalid;
pub use reporting::keeps_reporting;

/// The format's tag. One format serves both object types; the kind octet
/// after the header names which one the body is for.
pub const MAGIC: [u8; 8] = *b"RBNWCF01";
/// A Timer's set alone (#2243): still decoded, never encoded.
pub const KIND_TIMER_SET_ONLY: u8 = 1;
/// A Channel's body.
pub const KIND_CHANNEL: u8 = 2;
/// A Timer's set, then Default_Timeout (#2312). A new kind rather than a
/// part added to kind 1, so that a kind 1 file cut short can't pass for one
/// without the new part, and a reader that predates it refuses the file
/// instead of dropping the part.
pub const KIND_TIMER: u8 = 3;
/// The reporting settings of an object with intrinsic reporting (#2331).
pub const KIND_REPORTING: u8 = 4;
/// A written Description, then the kind and body of one of the kinds
/// [`KIND_CHANNEL`], [`KIND_TIMER`], [`KIND_REPORTING`] and
/// [`KIND_REPORTING_PARAMETERS`] (#2364). A kind
/// around the others rather than a part added to each, so their octets
/// don't change and a reader that predates it refuses the file instead of
/// dropping the Description.
pub const KIND_WITH_DESCRIPTION: u8 = 5;
/// The reporting settings of kind [`KIND_REPORTING`], then the event
/// algorithm parameters a write set (#2331). A new kind rather than a part
/// added to kind 4, for the reasons [`KIND_TIMER`] gives; its presence mask
/// lets a parameter join later without another.
pub const KIND_REPORTING_PARAMETERS: u8 = 6;
/// The most octets a file may take. A full member list, or reference list,
/// of the longest references takes about 30 KiB; the cap leaves room for
/// large constructed State_Change_Values.
pub const MAX_LEN: usize = 1024 * 1024;

/// The file octets for `snapshot`, saved for `object`.
///
/// Refuses ([`OutOfRange`](SnapshotErrorKind::OutOfRange)) a snapshot of
/// another object type than `object`'s, one holding what [`decode`]
/// wouldn't take back (a priority outside 1 to 16, more references or
/// members than the object keeps, delays that don't pair with the members,
/// a Control_Groups size outside 1 to [`MAX_CONTROL_GROUPS`], reporting
/// settings or an algorithm parameter no write leaves, a parameter the
/// object's type doesn't keep, Event_Parameters or Fault_Parameters with no
/// encoding or one past 64 KiB, a Description past 16 KiB), and one past
/// [`MAX_LEN`].
pub fn encode(
    object: ObjectIdentifier,
    snapshot: &WriteConfigSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    encode_file(object, snapshot).map_err(|error| error.in_format("write configuration"))
}

/// The snapshot a file's `octets` hold for `object`.
///
/// Refuses octets past [`MAX_LEN`], without this format's tag, saved for
/// another object, of a kind that is unknown or for another object type,
/// cut short, or holding a value [`encode`] never writes.
pub fn decode(
    object: ObjectIdentifier,
    octets: &[u8],
) -> Result<WriteConfigSnapshot, SnapshotError> {
    decode_file(object, octets).map_err(|error| error.in_format("Write configuration file"))
}

fn encode_file(
    object: ObjectIdentifier,
    snapshot: &WriteConfigSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    let mut body = BytesMut::new();
    if let Some(description) = snapshot.description() {
        if description.len() > MAX_DESCRIPTION_OCTETS {
            return Err(SnapshotError::out_of_range(
                "holds a Description past its cap",
            ));
        }
        body.extend_from_slice(&[KIND_WITH_DESCRIPTION]);
        put_octets(&mut body, description.as_bytes());
    }
    match (snapshot, object.object_type()) {
        (WriteConfigSnapshot::Timer(config), ObjectType::TIMER) => {
            body.extend_from_slice(&[KIND_TIMER]);
            encode_timer(&mut body, config)?;
        }
        (WriteConfigSnapshot::Channel(config), ObjectType::CHANNEL) => {
            body.extend_from_slice(&[KIND_CHANNEL]);
            encode_channel(&mut body, config)?;
        }
        (WriteConfigSnapshot::Reporting(config), kind) if keeps_reporting(kind) => {
            // Only a snapshot holding a parameter takes the newer kind, so
            // one without keeps its octets.
            if parameters::mask(&config.algorithm) == 0 {
                body.extend_from_slice(&[KIND_REPORTING]);
                reporting::encode(&mut body, config)?;
            } else {
                body.extend_from_slice(&[KIND_REPORTING_PARAMETERS]);
                reporting::encode(&mut body, config)?;
                parameters::encode(&mut body, kind, &config.algorithm)?;
            }
        }
        _ => {
            return Err(SnapshotError::out_of_range(
                "for another object type can't be saved for this object",
            ));
        }
    }
    if HEADER_LEN + body.len() > MAX_LEN {
        return Err(SnapshotError::out_of_range("is too large to save"));
    }
    Ok(framing::seal(&MAGIC, object, &body))
}

fn decode_file(
    object: ObjectIdentifier,
    octets: &[u8],
) -> Result<WriteConfigSnapshot, SnapshotError> {
    let (_, body) = framing::open(octets, &MAGIC, None, MAX_LEN, object)?;
    let mut reader = Reader { body, at: 0 };
    let mut kind = reader.u8()?;
    let description = if kind == KIND_WITH_DESCRIPTION {
        let description = reader.description()?;
        kind = reader.u8()?;
        if matches!(kind, KIND_WITH_DESCRIPTION | KIND_TIMER_SET_ONLY) {
            return Err(SnapshotError::damaged(
                "wraps a Description around a kind it never wraps",
            ));
        }
        Some(description)
    } else {
        None
    };
    let mut snapshot = match (kind, object.object_type()) {
        (KIND_TIMER, ObjectType::TIMER) => WriteConfigSnapshot::Timer(reader.timer(true)?),
        (KIND_TIMER_SET_ONLY, ObjectType::TIMER) => {
            WriteConfigSnapshot::Timer(reader.timer(false)?)
        }
        (KIND_CHANNEL, ObjectType::CHANNEL) => WriteConfigSnapshot::Channel(reader.channel()?),
        (KIND_REPORTING, kind) if keeps_reporting(kind) => {
            WriteConfigSnapshot::Reporting(reader.reporting()?)
        }
        (KIND_REPORTING_PARAMETERS, kind) if keeps_reporting(kind) => {
            let mut config = reader.reporting()?;
            config.algorithm = reader.parameters(kind)?;
            WriteConfigSnapshot::Reporting(config)
        }
        (
            KIND_TIMER
            | KIND_TIMER_SET_ONLY
            | KIND_CHANNEL
            | KIND_REPORTING
            | KIND_REPORTING_PARAMETERS,
            _,
        ) => {
            return Err(SnapshotError::new(
                SnapshotErrorKind::OtherObject,
                "holds another object type's configuration",
            ));
        }
        (kind, _) => {
            return Err(SnapshotError::new(
                SnapshotErrorKind::UnknownKind(kind),
                "holds a configuration of an unknown kind",
            ));
        }
    };
    if reader.at != body.len() {
        return Err(SnapshotError::damaged("has octets past its end"));
    }
    *snapshot.description_mut() = description;
    Ok(snapshot)
}

fn too_large() -> SnapshotError {
    SnapshotError::out_of_range("is too large to save")
}

/// A length, then that many octets. The callers' caps keep every length
/// well inside 32 bits.
fn put_octets(body: &mut BytesMut, octets: &[u8]) {
    body.extend_from_slice(&(octets.len() as u32).to_be_bytes());
    body.extend_from_slice(octets);
}

/// Append `count`, as four octets, and then `section` with its length.
fn put_section(body: &mut BytesMut, count: usize, section: &[u8]) -> Result<(), SnapshotError> {
    for value in [count, section.len()] {
        let value = u32::try_from(value).map_err(|_| too_large())?;
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

fn encode_timer(body: &mut BytesMut, config: &TimerWriteConfig) -> Result<(), SnapshotError> {
    encode_timer_set(body, config.state_change.as_ref())?;
    match config.default_timeout {
        None => body.extend_from_slice(&[0]),
        Some(timeout) => {
            body.extend_from_slice(&[1]);
            body.extend_from_slice(&timeout.to_be_bytes());
        }
    }
    Ok(())
}

fn encode_timer_set(
    body: &mut BytesMut,
    set: Option<&TimerStateChange>,
) -> Result<(), SnapshotError> {
    let Some(set) = set else {
        body.extend_from_slice(&[0]);
        return Ok(());
    };
    let out_of_range = SnapshotError::out_of_range("holds a Timer setting out of range");
    // Only what a load takes back is saved.
    if !(1..=16).contains(&set.priority_for_writing) || set.references.len() > MAX_TIMER_REFERENCES
    {
        return Err(out_of_range);
    }
    body.extend_from_slice(&[1, set.priority_for_writing]);
    let mut values = BytesMut::new();
    for value in &set.values {
        encode_timer_state_change_value(&mut values, value).map_err(|_| out_of_range)?;
    }
    put_section(body, set.values.len(), &values)?;
    put_section(
        body,
        set.references.len(),
        &references_section(&set.references),
    )
}

fn encode_channel(body: &mut BytesMut, config: &ChannelWriteConfig) -> Result<(), SnapshotError> {
    let out_of_range = SnapshotError::out_of_range("holds a Channel setting out of range");
    match &config.members {
        None => body.extend_from_slice(&[0]),
        Some(members) => {
            let count = members.references.len();
            if count > MAX_CHANNEL_MEMBERS || members.execution_delay.len() != count {
                return Err(out_of_range);
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
                .ok_or(out_of_range)?;
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

/// The body read front to back. Every read past the end refuses the body
/// as cut short.
struct Reader<'a> {
    body: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], SnapshotError> {
        let end = self
            .at
            .checked_add(len)
            .filter(|end| *end <= self.body.len())
            .ok_or(SnapshotError::truncated("is cut short"))?;
        let taken = &self.body[self.at..end];
        self.at = end;
        Ok(taken)
    }

    fn u8(&mut self) -> Result<u8, SnapshotError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, SnapshotError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().expect("two")))
    }

    fn u32(&mut self) -> Result<u32, SnapshotError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().expect("four")))
    }

    fn u64(&mut self) -> Result<u64, SnapshotError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().expect("eight")))
    }

    /// A length of at most `cap`, then that many octets.
    fn octets(&mut self, cap: usize) -> Result<&'a [u8], SnapshotError> {
        let len = usize::try_from(self.u32()?)
            .ok()
            .filter(|len| *len <= cap)
            .ok_or(SnapshotError::damaged("holds a part longer than its cap"))?;
        self.take(len)
    }

    /// A kept Description: at most [`MAX_DESCRIPTION_OCTETS`] of UTF-8.
    fn description(&mut self) -> Result<String, SnapshotError> {
        let octets = self.octets(MAX_DESCRIPTION_OCTETS)?;
        String::from_utf8(octets.to_vec())
            .map_err(|_| SnapshotError::damaged("holds a Description that isn't UTF-8"))
    }

    /// A presence octet or a BOOLEAN: 0 or 1, nothing else.
    fn flag(&mut self) -> Result<bool, SnapshotError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(SnapshotError::damaged("has a flag that is neither 0 nor 1")),
        }
    }

    /// A count of at most `cap` entries.
    fn count(&mut self, cap: usize) -> Result<usize, SnapshotError> {
        usize::try_from(self.u32()?)
            .ok()
            .filter(|count| *count <= cap)
            .ok_or(SnapshotError::damaged("holds more entries than the cap"))
    }

    /// A section of exactly `count` elements, each decoded by `decode`.
    fn elements<T>(
        &mut self,
        count: usize,
        decode: impl Fn(&[u8], usize) -> Result<(T, usize), Error>,
    ) -> Result<Vec<T>, SnapshotError> {
        let len = self.u32()? as usize;
        let section = self.take(len)?;
        let elements = framing::decode_capped(section, count, decode)?;
        if elements.len() != count {
            return Err(SnapshotError::damaged("holds fewer entries than it states"));
        }
        Ok(elements)
    }

    /// A Timer's body; `with_timeout` for one that goes on past its set
    /// with Default_Timeout.
    fn timer(&mut self, with_timeout: bool) -> Result<TimerWriteConfig, SnapshotError> {
        let state_change = self.timer_set()?;
        let default_timeout = if with_timeout && self.flag()? {
            Some(self.u64()?)
        } else {
            None
        };
        Ok(TimerWriteConfig {
            state_change,
            default_timeout,
            description: None,
        })
    }

    fn timer_set(&mut self) -> Result<Option<TimerStateChange>, SnapshotError> {
        if !self.flag()? {
            return Ok(None);
        }
        let priority_for_writing = self.u8()?;
        if !(1..=16).contains(&priority_for_writing) {
            return Err(SnapshotError::damaged("holds a priority outside 1 to 16"));
        }
        let count = self.count(7)?;
        let values = self.elements(count, decode_timer_state_change_value)?;
        let values = values
            .try_into()
            .map_err(|_| SnapshotError::damaged("holds other than seven values"))?;
        let count = self.count(MAX_TIMER_REFERENCES)?;
        let references = self.elements(count, decode_device_object_property_reference)?;
        Ok(Some(TimerStateChange {
            values,
            references,
            priority_for_writing,
        }))
    }

    fn channel(&mut self) -> Result<ChannelWriteConfig, SnapshotError> {
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
                return Err(SnapshotError::damaged(
                    "holds a Control_Groups size out of range",
                ));
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
            description: None,
        })
    }
}
