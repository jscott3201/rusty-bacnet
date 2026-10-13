//! Channel object (type 53) per ASHRAE 135-2020 Clause 12.53.
//!
//! A Channel forwards one value to a set of object properties and keeps no
//! state of its own besides that value: no priority array, no algorithm. A
//! Present_Value write is passed on to every member of
//! List_Of_Object_Property_References at the priority it came with.
//!
//! The object owns the bookkeeping: which values Present_Value takes (a
//! BACnetChannelValue), Last_Priority, Write_Status and the busy interlock
//! while a distribution runs. The member writes are the bundled server's,
//! made the way a Command object's are: it takes the queued [`CommandRun`]
//! under the guard that committed the Present_Value write, then, once that
//! guard is released, coerces the value to each member's datatype
//! ([`coerce_channel_value`]) and writes it through the local write path,
//! honouring each member's Execution_Delay. The outcome comes back through
//! [`BACnetObject::complete_command_run_internal`].
//!
//! A member may name another Device (Clause 12.53.11): the server writes it
//! there as a confirmed WriteProperty, from its device bindings, coerced to
//! the datatype a ReadProperty there returned. The object keeps that datatype
//! for each member ([`BACnetObject::remember_member_datatype_internal`]) until
//! the member is written again, or the server forgets it (#1342).
//!
//! Reliability reports how the last distribution ended (Clause 12.53.9). A
//! SUCCESSFUL one sets NO_FAULT_DETECTED; a FAILED one sets the kind of its
//! first failure ([`WriteFailure::reliability`]): CONFIGURATION_ERROR,
//! PROCESS_ERROR or COMMUNICATION_FAILURE. It keeps its value while a
//! distribution is in progress, and Status_Flags shows FAULT whenever it isn't
//! NO_FAULT_DETECTED. Out of service, Reliability is decoupled from the
//! distributions (Clause 12.53.10): it holds what it read when the object
//! went out of service, a client may write any Reliability value to it, and
//! back in service it reads the last distribution's verdict again.
//!
//! Channel_Number and Control_Groups are what the bundled server's WriteGroup
//! execution matches on (Clause 15.11). Allow_Group_Delay_Inhibit is served
//! for it too: the object only stores the flag, and the server reads it to
//! decide whether a WriteGroup asking for no delays gets them.

use std::borrow::Cow;
use std::sync::Arc;

use bacnet_encoding::constructed::constructed_channel_value;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier, Reliability, WriteStatus,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::command::{next_generation, CommandRun, RunPlan, WriteFailure};
use crate::common::{self, read_identity_properties};
use crate::property_metadata::{property_list_from_metadata, PropertyMetadata};
use crate::traits::BACnetObject;

mod arrays;
mod coercion;
mod metadata;

pub use arrays::{MAX_CHANNEL_MEMBERS, MAX_CONTROL_GROUPS};
pub use coercion::{coerce_channel_value, CoercionFailure, MemberDatatype};

/// The priority Last_Priority holds when a write carried none, and its
/// initial value (Clause 12.53.6).
const DEFAULT_PRIORITY: u8 = 16;

/// One member a distribution writes.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelMember {
    /// The property written; never an empty reference. One naming a Device
    /// other than this one is written in that device.
    pub reference: BACnetDeviceObjectPropertyReference,
    /// Milliseconds after the distribution starts before this member is
    /// written: its Execution_Delay element.
    pub delay_ms: u32,
    /// The member's zero-based slot in List_Of_Object_Property_References.
    pub slot: usize,
    /// For a member in another device, the datatype an earlier distribution
    /// learned for it, if any (#1342).
    pub learned: Option<MemberDatatype>,
}

/// A Channel's Present_Value write, for the server to pass on.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelDistribution {
    /// The value written, before any coercion.
    pub value: PropertyValue,
    /// The priority the write carried, which each member write carries too.
    pub priority: Option<u8>,
    /// The members to write, in list order.
    pub members: Vec<ChannelMember>,
}

/// BACnet Channel object: forwards each Present_Value write to a list of
/// member properties.
pub struct ChannelObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: PropertyValue,
    last_priority: u8,
    write_status: WriteStatus,
    /// The verdict of the last distribution to end, or of a Present_Value
    /// write that had nothing to distribute.
    reliability: Reliability,
    /// What Reliability reads while out of service: the value it read when
    /// the object went out of service, or one a client wrote since. `None`
    /// in service.
    simulated_reliability: Option<Reliability>,
    out_of_service: bool,
    members: Vec<BACnetDeviceObjectPropertyReference>,
    /// The datatype the server learned for each member in another device,
    /// slot by slot with `members`; a slot whose member is replaced forgets
    /// it (#1342).
    learned: Vec<Option<MemberDatatype>>,
    execution_delay: Vec<u32>,
    channel_number: u16,
    control_groups: Vec<u32>,
    allow_group_delay_inhibit: bool,
    /// A fresh `next_generation` value at every Present_Value write that
    /// starts a distribution, unique across the process.
    generation: u64,
    pending_run: Option<CommandRun>,
}

impl ChannelObject {
    /// Create a Channel with no members, in control group 0 alone (no
    /// assignment), Present_Value NULL, Write_Status IDLE, Reliability
    /// NO_FAULT_DETECTED and Allow_Group_Delay_Inhibit FALSE.
    pub fn new(instance: u32, name: impl Into<String>, channel_number: u16) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::CHANNEL, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: PropertyValue::Null,
            last_priority: DEFAULT_PRIORITY,
            write_status: WriteStatus::IDLE,
            reliability: Reliability::NO_FAULT_DETECTED,
            simulated_reliability: None,
            out_of_service: false,
            members: Vec::new(),
            learned: Vec::new(),
            execution_delay: Vec::new(),
            channel_number,
            control_groups: vec![0],
            allow_group_delay_inhibit: false,
            generation: next_generation(),
            pending_run: None,
        })
    }

    /// Replace List_Of_Object_Property_References.
    ///
    /// Execution_Delay follows the new size, keeping its leading delays and
    /// adding zeros. A member may name another Device, which the server then
    /// writes over the network. Refused, leaving the list as it was, with
    /// PROPERTY / VALUE_OUT_OF_RANGE for a member whose device identifier
    /// isn't a Device object (#1285), and with RESOURCES /
    /// NO_SPACE_TO_WRITE_PROPERTY past [`MAX_CHANNEL_MEMBERS`].
    pub fn set_members(
        &mut self,
        members: Vec<BACnetDeviceObjectPropertyReference>,
    ) -> Result<(), Error> {
        if members.len() > MAX_CHANNEL_MEMBERS {
            return Err(common::protocol_error(
                ErrorClass::RESOURCES,
                ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
            ));
        }
        crate::device_reference::check_device_members(&members)?;
        self.execution_delay.resize(members.len(), 0);
        self.learned = vec![None; members.len()];
        self.members = members;
        Ok(())
    }

    /// Replace Execution_Delay, one delay in milliseconds per member.
    ///
    /// Refused with PROPERTY / VALUE_OUT_OF_RANGE, leaving the delays as they
    /// were, unless there's exactly one per member, so set the members first.
    pub fn set_execution_delay(&mut self, delays: Vec<u32>) -> Result<(), Error> {
        if delays.len() != self.members.len() {
            return Err(common::value_out_of_range_error());
        }
        self.execution_delay = delays;
        Ok(())
    }

    /// Replace Control_Groups.
    ///
    /// Refused, leaving the groups as they were, with PROPERTY /
    /// VALUE_OUT_OF_RANGE for an empty list (the array holds at least one
    /// entry, zero meaning no assignment) and with RESOURCES /
    /// NO_SPACE_TO_WRITE_PROPERTY past [`MAX_CONTROL_GROUPS`].
    pub fn set_control_groups(&mut self, groups: Vec<u32>) -> Result<(), Error> {
        arrays::check_group_count(groups.len())?;
        self.control_groups = groups;
        Ok(())
    }

    /// Set Allow_Group_Delay_Inhibit: whether a WriteGroup that asks for no
    /// execution delays skips this Channel's Execution_Delay (Clause
    /// 12.53.13). WriteProperty and WritePropertyMultiple keep the delays
    /// either way.
    pub fn set_allow_group_delay_inhibit(&mut self, allow: bool) {
        self.allow_group_delay_inhibit = allow;
    }

    /// Take a Present_Value write (Clauses 12.53.5 to 12.53.7).
    fn write_present_value(
        &mut self,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        if self.write_status == WriteStatus::IN_PROGRESS {
            return Err(common::protocol_error(ErrorClass::OBJECT, ErrorCode::BUSY));
        }
        check_channel_value(&value)?;
        let last_priority = match priority {
            None => DEFAULT_PRIORITY,
            Some(priority @ 1..=16) => priority,
            Some(_) => return Err(common::value_out_of_range_error()),
        };
        self.present_value = value;
        self.last_priority = last_priority;
        // Out of service the value stays here (Clause 12.53.10).
        if self.out_of_service {
            return Ok(());
        }
        if self.members.is_empty() {
            self.write_status = WriteStatus::IDLE;
            self.reliability = Reliability::NO_FAULT_DETECTED;
            return Ok(());
        }
        self.generation = next_generation();
        let members: Vec<ChannelMember> = self
            .members
            .iter()
            .zip(&self.execution_delay)
            .zip(&self.learned)
            .enumerate()
            .filter(|(_, ((member, _), _))| !arrays::is_empty(member))
            .map(|(slot, ((member, delay), learned))| ChannelMember {
                reference: member.clone(),
                delay_ms: *delay,
                slot,
                learned: *learned,
            })
            .collect();
        if members.is_empty() {
            // Every member is an empty reference: nothing to write, nothing
            // to fail.
            self.write_status = WriteStatus::SUCCESSFUL;
            self.reliability = Reliability::NO_FAULT_DETECTED;
            return Ok(());
        }
        self.write_status = WriteStatus::IN_PROGRESS;
        self.pending_run = Some(CommandRun {
            source: self.oid,
            generation: self.generation,
            plan: RunPlan::Channel(ChannelDistribution {
                value: self.present_value.clone(),
                priority,
                members,
            }),
            chain: Arc::from([]),
        });
        Ok(())
    }

    /// What Reliability reads: a client's simulated value while out of
    /// service, otherwise the last distribution's verdict.
    fn served_reliability(&self) -> Reliability {
        self.simulated_reliability.unwrap_or(self.reliability)
    }

    fn write_out_of_service(&mut self, value: PropertyValue) -> Result<(), Error> {
        let PropertyValue::Boolean(value) = value else {
            return Err(common::invalid_data_type_error());
        };
        if value != self.out_of_service {
            // Out of service a client simulates Reliability, starting from
            // what it reads now; back in service the verdict shows again.
            self.simulated_reliability = value.then_some(self.reliability);
        }
        self.out_of_service = value;
        Ok(())
    }

    /// A client's simulated Reliability, taken only while out of service
    /// (Clause 12.53.10).
    fn write_reliability(&mut self, value: PropertyValue) -> Result<(), Error> {
        if !self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        let PropertyValue::Enumerated(raw) = value else {
            return Err(common::invalid_data_type_error());
        };
        let reliability = Reliability::from_raw(raw);
        if !common::is_reliability_value_valid(reliability) {
            return Err(common::value_out_of_range_error());
        }
        self.simulated_reliability = Some(reliability);
        Ok(())
    }
}

/// Refuse a Present_Value that isn't a BACnetChannelValue: a primitive, or
/// one of the constructed alternatives, a lighting command in context tag
/// `[0]` or Addendum 135-2020ca's xy colour in `[1]` and colour command in
/// `[2]` (#1474). Octets that open one of those tags but don't hold one are
/// INVALID_DATA_ENCODING.
fn check_channel_value(value: &PropertyValue) -> Result<(), Error> {
    match value {
        value if value.is_primitive() => Ok(()),
        PropertyValue::ApplicationData(octets)
            if matches!(octets.first(), Some(0x0E | 0x1E | 0x2E)) =>
        {
            if constructed_channel_value(octets).is_some() {
                Ok(())
            } else {
                Err(common::invalid_data_encoding_error())
            }
        }
        _ => Err(common::invalid_data_type_error()),
    }
}

impl BACnetObject for ChannelObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if let Some(result) = read_identity_properties!(self, property, array_index) {
            return result;
        }
        match property {
            PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::CHANNEL.to_raw()))
            }
            PropertyIdentifier::PRESENT_VALUE => Ok(self.present_value.clone()),
            PropertyIdentifier::LAST_PRIORITY => {
                Ok(PropertyValue::Unsigned(self.last_priority.into()))
            }
            PropertyIdentifier::WRITE_STATUS => {
                Ok(PropertyValue::Enumerated(self.write_status.to_raw()))
            }
            // FAULT follows Reliability; with no intrinsic reporting and
            // nothing overriding the value, IN_ALARM and OVERRIDDEN stay
            // clear (Clause 12.53.8).
            PropertyIdentifier::STATUS_FLAGS => Ok(common::compute_status_flags(
                StatusFlags::empty(),
                self.served_reliability(),
                self.out_of_service,
                EventState::NORMAL,
            )),
            PropertyIdentifier::RELIABILITY => Ok(PropertyValue::Enumerated(
                self.served_reliability().to_raw(),
            )),
            PropertyIdentifier::OUT_OF_SERVICE => Ok(PropertyValue::Boolean(self.out_of_service)),
            PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES => common::read_array(
                crate::device_reference::reference_elements(&self.members),
                array_index,
            ),
            PropertyIdentifier::EXECUTION_DELAY => common::read_array(
                self.execution_delay
                    .iter()
                    .map(|delay| PropertyValue::Unsigned((*delay).into()))
                    .collect(),
                array_index,
            ),
            PropertyIdentifier::ALLOW_GROUP_DELAY_INHIBIT => {
                Ok(PropertyValue::Boolean(self.allow_group_delay_inhibit))
            }
            PropertyIdentifier::CHANNEL_NUMBER => {
                Ok(PropertyValue::Unsigned(self.channel_number.into()))
            }
            PropertyIdentifier::CONTROL_GROUPS => common::read_array(
                self.control_groups
                    .iter()
                    .map(|group| PropertyValue::Unsigned((*group).into()))
                    .collect(),
                array_index,
            ),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        if array_index.is_some() && !self.is_array_property(property) {
            return Err(common::property_is_not_an_array_error());
        }
        if let Some(result) = common::write_object_name(&mut self.name, property, &value) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            PropertyIdentifier::PRESENT_VALUE => self.write_present_value(value, priority),
            PropertyIdentifier::OUT_OF_SERVICE => self.write_out_of_service(value),
            PropertyIdentifier::RELIABILITY => self.write_reliability(value),
            PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES => {
                self.write_members(array_index, value)
            }
            PropertyIdentifier::EXECUTION_DELAY => self.write_execution_delay(array_index, value),
            PropertyIdentifier::ALLOW_GROUP_DELAY_INHIBIT => {
                let PropertyValue::Boolean(allow) = value else {
                    return Err(common::invalid_data_type_error());
                };
                self.allow_group_delay_inhibit = allow;
                Ok(())
            }
            PropertyIdentifier::CHANNEL_NUMBER => {
                let PropertyValue::Unsigned(raw) = value else {
                    return Err(common::invalid_data_type_error());
                };
                self.channel_number =
                    u16::try_from(raw).map_err(|_| common::value_out_of_range_error())?;
                Ok(())
            }
            PropertyIdentifier::CONTROL_GROUPS => self.write_control_groups(array_index, value),
            _ => Err(common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
                array_index,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        Cow::Borrowed(metadata::CHANNEL_PROPERTY_METADATA)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        property_list_from_metadata(metadata::CHANNEL_PROPERTY_METADATA)
    }

    fn take_command_run_internal(&mut self) -> Option<CommandRun> {
        self.pending_run.take()
    }

    fn command_generation_internal(&self) -> Option<u64> {
        Some(self.generation)
    }

    fn remember_member_datatype_internal(
        &mut self,
        slot: usize,
        reference: &BACnetDeviceObjectPropertyReference,
        datatype: Option<MemberDatatype>,
    ) {
        if self.members.get(slot) == Some(reference) {
            self.learned[slot] = datatype;
        }
    }

    fn complete_command_run_internal(
        &mut self,
        generation: u64,
        outcome: Result<(), WriteFailure>,
    ) -> bool {
        if generation != self.generation || self.write_status != WriteStatus::IN_PROGRESS {
            return false;
        }
        (self.write_status, self.reliability) = match outcome {
            Ok(()) => (WriteStatus::SUCCESSFUL, Reliability::NO_FAULT_DETECTED),
            Err(failure) => (WriteStatus::FAILED, failure.reliability()),
        };
        true
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod array_tests;

#[cfg(test)]
mod coercion_tests;

#[cfg(test)]
mod reliability_tests;

#[cfg(test)]
mod learned_tests;
