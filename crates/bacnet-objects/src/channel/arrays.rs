//! Writes of a Channel's three arrays: List_Of_Object_Property_References,
//! Execution_Delay and Control_Groups (Clauses 12.53.11, 12.53.12 and
//! 12.53.15).
//!
//! Each takes a whole-array write, a write of one element at a one-based
//! index, and a write of index 0, which resizes it. A write leaves the array
//! unchanged when it fails.
//!
//! The member list and the delays always have the same size (Table 12-62,
//! footnote 1). The clause names the writes that change a size: index 0 of
//! either array grows the other with it, the list with empty references and
//! the delays with zeros (Clauses 12.53.11.2 and 12.53.12.1). Here an
//! index-0 write that shrinks one shrinks the other from the end too, and a
//! whole write of the member list, which says how many members there are,
//! carries the delays to its size the same way. A whole write of
//! Execution_Delay only supplies one delay per member: one of any other
//! length is refused with VALUE_OUT_OF_RANGE, as
//! `ChannelObject::set_execution_delay` refuses it, so writing delays never
//! adds or drops a member.
//!
//! A member may name another device (Clause 12.53.11, #1264); the server
//! writes it there. The object can't tell which Device holds it, so the
//! bundled server drops a Device identifier naming its own Device before the
//! value gets here (#1136), and its runner treats a member still naming this
//! Device as local. A reference whose Device instance is 4194303 is an empty
//! one, not a remote one. A member whose device identifier isn't a Device
//! object at all is refused with PROPERTY / VALUE_OUT_OF_RANGE (#1285), so the
//! server's localizing, which compares the whole identifier, leaves it to that
//! check. The members are decoded and checked by the shared helpers in
//! `device_reference.rs` (#1313).

use crate::prelude::*;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::ChannelObject;
use crate::{common, device_reference};

/// Resource cap on members, and so on Execution_Delay elements: the bound a
/// Schedule puts on its reference list.
pub const MAX_CHANNEL_MEMBERS: usize = 1024;

/// Resource cap on Control_Groups elements, the most groups one Channel
/// joins at once (Clause 12.53.15 leaves it to the implementation).
pub const MAX_CONTROL_GROUPS: usize = 64;

/// The member a resize adds: the shared unset reference, object instance
/// 4194303, which marks the reference empty (Clause 12.53.11.1).
pub(super) fn empty_reference() -> BACnetDeviceObjectPropertyReference {
    device_reference::unset_reference(ObjectType::ANALOG_INPUT)
}

/// Whether `member` is an empty reference, one the Channel skips: its object
/// or Device instance is 4194303 (Clause 12.53.11.1), the shared
/// [`BACnetDeviceObjectPropertyReference::is_unset`] rule.
pub(super) fn is_empty(member: &BACnetDeviceObjectPropertyReference) -> bool {
    member.is_unset()
}

fn no_space_error() -> Error {
    common::protocol_error(ErrorClass::RESOURCES, ErrorCode::NO_SPACE_TO_WRITE_PROPERTY)
}

/// The new size an index-0 write asks for, up to `cap`.
fn new_size(value: PropertyValue, cap: usize) -> Result<usize, Error> {
    let PropertyValue::Unsigned(size) = value else {
        return Err(common::invalid_data_type_error());
    };
    usize::try_from(size)
        .ok()
        .filter(|size| *size <= cap)
        .ok_or_else(no_space_error)
}

/// The zero-based slot of one-based `index` in an array of `len` elements.
fn slot(index: u32, len: usize) -> Result<usize, Error> {
    usize::try_from(index - 1)
        .ok()
        .filter(|slot| *slot < len)
        .ok_or_else(common::invalid_array_index_error)
}

/// The members a whole write holds, each checked in order: a member whose
/// device identifier isn't a Device object is refused, empty or not.
fn decode_members(
    value: &PropertyValue,
) -> Result<Vec<BACnetDeviceObjectPropertyReference>, Error> {
    let members = device_reference::decode_references(value)?;
    if members.len() > MAX_CHANNEL_MEMBERS {
        return Err(no_space_error());
    }
    device_reference::check_device_members(&members)?;
    Ok(members)
}

/// The elements a whole-array write of Unsigned values holds: a list, or the
/// single value a one-element array decodes to.
fn unsigned_elements(value: PropertyValue) -> Result<Vec<u32>, Error> {
    let elements = match value {
        PropertyValue::List(elements) => elements,
        single => vec![single],
    };
    elements.into_iter().map(unsigned_element).collect()
}

/// One element of Execution_Delay or Control_Groups. Both are kept as 32-bit
/// numbers: Control_Groups is Unsigned32, and a delay past 2^32-1
/// milliseconds (about 49 days) is VALUE_OUT_OF_RANGE.
fn unsigned_element(value: PropertyValue) -> Result<u32, Error> {
    let PropertyValue::Unsigned(raw) = value else {
        return Err(common::invalid_data_type_error());
    };
    u32::try_from(raw).map_err(|_| common::value_out_of_range_error())
}

impl ChannelObject {
    /// Make both parallel arrays, and the datatypes learned for the members,
    /// `size` long. The members a resize keeps keep what was learned for
    /// them.
    fn resize_members(&mut self, size: usize) {
        self.members.resize_with(size, empty_reference);
        self.learned.resize(size, None);
        self.execution_delay.resize(size, 0);
    }

    /// Write List_Of_Object_Property_References. A member replaced, by an
    /// element write or a whole one, loses the datatype learned for it.
    pub(super) fn write_members(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        match array_index {
            None => {
                let members = decode_members(&value)?;
                let size = members.len();
                self.members = members;
                self.learned.clear();
                self.resize_members(size);
            }
            Some(0) => {
                let size = new_size(value, MAX_CHANNEL_MEMBERS)?;
                self.resize_members(size);
            }
            Some(index) => {
                let at = slot(index, self.members.len())?;
                let member: BACnetDeviceObjectPropertyReference =
                    device_reference::decode_reference(&value)?;
                device_reference::check_device_member(member.device_identifier)?;
                self.members[at] = member;
                self.learned[at] = None;
            }
        }
        Ok(())
    }

    pub(super) fn write_execution_delay(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        match array_index {
            None => {
                let delays = unsigned_elements(value)?;
                if delays.len() != self.members.len() {
                    return Err(common::value_out_of_range_error());
                }
                self.execution_delay = delays;
            }
            Some(0) => {
                let size = new_size(value, MAX_CHANNEL_MEMBERS)?;
                self.resize_members(size);
            }
            Some(index) => {
                let at = slot(index, self.execution_delay.len())?;
                self.execution_delay[at] = unsigned_element(value)?;
            }
        }
        Ok(())
    }

    pub(super) fn write_control_groups(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        match array_index {
            None => {
                let groups = unsigned_elements(value)?;
                check_group_count(groups.len())?;
                self.control_groups = groups;
            }
            Some(0) => {
                let size = new_size(value, MAX_CONTROL_GROUPS)?;
                check_group_count(size)?;
                self.control_groups.resize(size, 0);
            }
            Some(index) => {
                let at = slot(index, self.control_groups.len())?;
                self.control_groups[at] = unsigned_element(value)?;
            }
        }
        Ok(())
    }
}

/// Control_Groups holds at least one entry (Clause 12.53.15) and at most
/// [`MAX_CONTROL_GROUPS`].
pub(super) fn check_group_count(count: usize) -> Result<(), Error> {
    match count {
        0 => Err(common::value_out_of_range_error()),
        count if count > MAX_CONTROL_GROUPS => Err(no_space_error()),
        _ => Ok(()),
    }
}
