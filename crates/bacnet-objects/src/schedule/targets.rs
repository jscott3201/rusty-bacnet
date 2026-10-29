//! Network writes of List_Of_Object_Property_References and
//! Priority_For_Writing, and what a change to either does to the targets
//! (#1088).
//!
//! Table 12-28 leaves the writability of both properties to the
//! implementation. This object accepts them through WriteProperty and
//! WritePropertyMultiple with the checks the local setters apply; the
//! server's AddListElement and RemoveListElement edit the reference list and
//! write the result back whole through the same path (#1121).
//! Priority_For_Writing takes an Unsigned from 1 to 16 (VALUE_OUT_OF_RANGE
//! otherwise). The reference list arrives as the raw bytes of its
//! BACnetDeviceObjectPropertyReference elements and is decoded by the shared
//! helpers in `device_reference.rs` (#1313). Each refusal names its member by
//! position (`common::at_list_element`), so AddListElement can report the
//! request element behind it. The checks run in three passes, and the first
//! refusal of the first pass that refuses anything is the answer: the whole
//! list is decoded (a member whose first tag is not the context `[0]` object
//! identifier is INVALID_DATA_TYPE, one that starts right but doesn't decode
//! INVALID_DATA_ENCODING), then its length is held to [`MAX_REFERENCES`]
//! (NO_SPACE_TO_WRITE_PROPERTY, naming the first member past it), then each
//! member's Device member is checked in order. So a malformed member, or the
//! cap, wins over a Device refusal of an earlier member.
//!
//! A Device member that isn't a Device identifier is VALUE_OUT_OF_RANGE
//! (#1308). Any other Device member is OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
//! the error Clause 12.24.10 names for a Schedule that serves only objects of
//! its own device. That is this object's profile: it writes local targets
//! only and stores references without a Device member. The object can't tell
//! which Device holds it, so the bundled server removes a Device identifier
//! that names its own Device before the value gets here (#1122); what reaches
//! this check names another device, or comes from a caller outside the
//! server. A refused write changes nothing.
//!
//! What a change does is left to the implementation; the choices here:
//!
//! - The new list, at the new priority, gets the current Present_Value as
//!   soon as the object writes at all. In service, the next calculation
//!   inside Effective_Period sends it, as entering the period does, and
//!   outside the period nothing is sent. Out of service it goes out as a
//!   value a client writes would. The bundled server runs that pass at once
//!   after a write commits.
//! - The slots the change leaves behind are relinquished with a NULL at the
//!   priority they hold: on each dropped member, or on every member when the
//!   priority moves. Otherwise the targets would keep a command at a priority
//!   no writer owns any more.
//! - Only slots this Schedule still holds are relinquished: those filled by
//!   its last write, of a value other than NULL, while it has stayed active
//!   since. Leaving Effective_Period gives them up. Clause 12.24.6 has
//!   seasonal Schedules share references and a priority, each writing only in
//!   its own period, so an out-of-season Schedule whose list or priority is
//!   edited must not clear the command of the one in season.
//! - Every accepted write counts as a change, even of the value already
//!   held; sending the current value again is harmless.

use crate::prelude::*;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetObjectPropertyReference,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::{ScheduleObject, ScheduleWrite, writes};
use crate::{common, device_reference};

/// Resource cap on List_Of_Object_Property_References members, the bound
/// Exception_Schedule has.
pub(crate) const MAX_REFERENCES: usize = 1024;

/// The slots a Schedule's value holds: the priority and the references of
/// the write that filled them.
#[derive(Debug, Clone)]
pub(super) struct HeldCommand {
    priority: u8,
    references: Vec<BACnetObjectPropertyReference>,
}

/// The local references in a written list. A refusal names its member's
/// position in the list (#1121).
fn decode_references(value: &PropertyValue) -> Result<Vec<BACnetObjectPropertyReference>, Error> {
    let members: Vec<BACnetDeviceObjectPropertyReference> =
        device_reference::decode_references_at(value)?;
    if members.len() > MAX_REFERENCES {
        return Err(common::at_list_element(
            writes::no_space_error(),
            MAX_REFERENCES,
        ));
    }
    members
        .into_iter()
        .enumerate()
        .map(|(index, member)| {
            device_reference::into_local_property_reference(member)
                .map_err(|error| common::at_list_element(error, index))
        })
        .collect()
}

impl ScheduleObject {
    /// WriteProperty of List_Of_Object_Property_References: the whole list.
    pub(super) fn write_object_property_references(
        &mut self,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let references = decode_references(&value)?;
        self.set_object_property_references(references)
    }

    /// WriteProperty of Priority_For_Writing, through the local setter.
    pub(super) fn write_priority_for_writing(&mut self, value: PropertyValue) -> Result<(), Error> {
        let PropertyValue::Unsigned(priority) = value else {
            return Err(common::invalid_data_type_error());
        };
        let priority = u8::try_from(priority).map_err(|_| common::value_out_of_range_error())?;
        self.set_priority_for_writing(priority)
    }

    /// List_Of_Object_Property_References or Priority_For_Writing changed:
    /// owe a NULL to each held slot the change left behind and the current
    /// value to the new targets, and drop refusals by members that are gone.
    pub(super) fn targets_changed(&mut self) {
        if let Some(held) = self.held.take() {
            let priority = held.priority;
            let still_written = priority == self.priority_for_writing;
            let list = &self.list_of_object_property_references;
            let (kept, left): (Vec<_>, Vec<_>) = held
                .references
                .into_iter()
                .partition(|reference| still_written && list.contains(reference));
            if !left.is_empty() {
                self.relinquish_owed.push(ScheduleWrite {
                    value: PropertyValue::Null,
                    priority,
                    references: left,
                    retry: false,
                });
            }
            if !kept.is_empty() {
                self.held = Some(HeldCommand {
                    priority,
                    references: kept,
                });
            }
        }
        self.rewrite_owed = true;
        let list = &self.list_of_object_property_references;
        self.refusing_references
            .retain(|reference| list.contains(reference));
        let _ = self.recompute_reliability();
    }

    /// The write of `value` to every reference at Priority_For_Writing, or
    /// `None` without references; notes the slots it fills.
    pub(super) fn command(&mut self, value: PropertyValue) -> Option<ScheduleWrite> {
        self.rewrite_owed = false;
        let references = &self.list_of_object_property_references;
        if references.is_empty() {
            self.held = None;
            return None;
        }
        self.held = (value != PropertyValue::Null).then(|| HeldCommand {
            priority: self.priority_for_writing,
            references: references.clone(),
        });
        Some(ScheduleWrite {
            value,
            priority: self.priority_for_writing,
            references: references.clone(),
            retry: false,
        })
    }

    /// The writes owed apart from the calculation, in order: the NULLs a
    /// change of the targets owes, then the Present_Value owed out of service,
    /// for a client's write or a change of the targets (`out_of_service.rs`).
    pub(super) fn take_owed_writes(&mut self) -> Vec<ScheduleWrite> {
        let mut writes = core::mem::take(&mut self.relinquish_owed);
        let owed = match self.simulated_write.take() {
            Some(value) => Some(value),
            None if self.out_of_service && self.rewrite_owed => Some(self.present_value.clone()),
            None => None,
        };
        if let Some(value) = owed {
            writes.extend(self.command(value));
        }
        writes
    }
}
