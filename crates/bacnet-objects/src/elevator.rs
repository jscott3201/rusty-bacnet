//! Elevator Group, Escalator, and Lift objects per ASHRAE 135-2020.
//!
//! - ElevatorGroupObject (type 57) — Clause 12.58
//! - EscalatorObject (type 58) — Clause 12.60
//! - LiftObject (type 59) — Clause 12.59

use crate::prelude::*;
use alloc::borrow::Cow;
use bacnet_types::constructed::BACnetLandingCallStatus;
use bacnet_types::enums::{LiftCarDirection, LiftGroupMode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use core::hash::Hash;

use crate::collections::HashSet;

use crate::common;
use crate::traits::BACnetObject;

mod car_state;
mod door_values;
mod doors;
mod energy_meter;
mod escalator;
mod landing_calls;
mod lift;
mod membership;
mod metadata;

pub use escalator::EscalatorObject;
pub use lift::LiftObject;

/// Whether `raw` is in the domain of one of this family's extensible Clause 21
/// enumerations: one of its named values (`named` is its `ALL_NAMED` table,
/// `value` is `raw` as that type), or the proprietary range 1024..=65535 that
/// Clause 23.1 (Table 23-1) opens for each of them. Every other value is
/// reserved or too large, and a write of it is refused with
/// VALUE_OUT_OF_RANGE.
fn named_or_proprietary<T: Copy + PartialEq>(named: &[(&str, T)], value: T, raw: u32) -> bool {
    named.iter().any(|&(_, named)| named == value) || (1024..=65_535).contains(&raw)
}

/// Whether `direction` is in BACnetLiftCarDirection, the datatype of
/// Car_Moving_Direction, Car_Assigned_Direction and the direction of each
/// landing call.
fn direction_in_range(direction: &LiftCarDirection) -> bool {
    named_or_proprietary(LiftCarDirection::ALL_NAMED, *direction, direction.to_raw())
}

/// `value`, or VALUE_OUT_OF_RANGE when `in_range` refuses it.
fn checked<T>(value: T, in_range: fn(&T) -> bool) -> Result<T, Error> {
    if in_range(&value) {
        Ok(value)
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// Decode a Fault_Signals write, a BACnetLIST of one of this family's fault
/// enumerations (`named` is its `ALL_NAMED` table, `from_raw` its
/// constructor): one Enumerated or a list of them. A value that isn't
/// Enumerated is INVALID_DATA_TYPE; a fault outside the enumeration's domain
/// or a repeated fault (the list holds a set) is VALUE_OUT_OF_RANGE. The
/// caller stores nothing unless every element passes.
fn decode_fault_signals<T: Copy + Eq + Hash>(
    value: PropertyValue,
    named: &[(&str, T)],
    from_raw: fn(u32) -> T,
) -> Result<Vec<T>, Error> {
    let values = match value {
        PropertyValue::Enumerated(v) => vec![PropertyValue::Enumerated(v)],
        PropertyValue::List(values) => values,
        _ => return Err(common::invalid_data_type_error()),
    };
    let mut faults = Vec::with_capacity(values.len());
    let mut seen = HashSet::with_capacity(values.len());
    // A refusal names its element (#1048).
    for (index, value) in values.into_iter().enumerate() {
        let PropertyValue::Enumerated(raw) = value else {
            return Err(common::at_list_element(
                common::invalid_data_type_error(),
                index,
            ));
        };
        let fault = from_raw(raw);
        if !named_or_proprietary(named, fault, raw) || !seen.insert(fault) {
            return Err(common::at_list_element(
                common::value_out_of_range_error(),
                index,
            ));
        }
        faults.push(fault);
    }
    Ok(faults)
}

// ===========================================================================
// ElevatorGroupObject (type 57)
// ===========================================================================

/// BACnet Elevator Group object — manages a group of lifts.
///
/// The object serves only properties its table (Clause 12.58, Table 12-76)
/// defines. That table has no Status_Flags, Out_Of_Service or Reliability, so
/// reads and writes of those return UNKNOWN_PROPERTY.
pub struct ElevatorGroupObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// The Positive Integer Value object holding the machine room's number.
    machine_room_id: ObjectIdentifier,
    /// Group identifier (Unsigned8).
    group_id: u8,
    /// List of lift ObjectIdentifiers in this group.
    group_members: Vec<ObjectIdentifier>,
    /// Group mode.
    group_mode: LiftGroupMode,
    /// Active landing calls, served as the Landing_Calls BACnetLIST.
    landing_calls: Vec<BACnetLandingCallStatus>,
    /// The last call written to Landing_Call_Control.
    landing_call_control: BACnetLandingCallStatus,
}

impl ElevatorGroupObject {
    /// Create a new Elevator Group object with default values.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ELEVATOR_GROUP, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            machine_room_id: ObjectIdentifier::new(
                ObjectType::POSITIVE_INTEGER_VALUE,
                ObjectIdentifier::MAX_INSTANCE,
            )?,
            group_id: 0,
            group_members: Vec::new(),
            group_mode: LiftGroupMode::UNKNOWN,
            landing_calls: Vec::new(),
            landing_call_control: landing_calls::initial_landing_call_control(),
        })
    }

    /// The Positive Integer Value object served as Machine_Room_ID. Until the
    /// application sets one it names instance 4194303, which Clause 12.58
    /// uses when the machine room has no identification number.
    pub fn machine_room_id(&self) -> ObjectIdentifier {
        self.machine_room_id
    }

    /// Choose which Positive Integer Value object supplies this group's
    /// machine-room number; clients read the number from that object's
    /// Present_Value (Clause 12.58).
    ///
    /// Machine_Room_ID is read-only over the network, so this is the only way
    /// to change it. A reference to any other object type is refused with
    /// VALUE_OUT_OF_RANGE and the property is left unchanged.
    pub fn set_machine_room_id(&mut self, oid: ObjectIdentifier) -> Result<(), Error> {
        if oid.object_type() != ObjectType::POSITIVE_INTEGER_VALUE {
            return Err(common::value_out_of_range_error());
        }
        self.machine_room_id = oid;
        Ok(())
    }

    /// Add a lift member to this elevator group.
    pub fn add_member(&mut self, oid: ObjectIdentifier) {
        self.group_members.push(oid);
    }

    /// The last landing call written to Landing_Call_Control, or a placeholder
    /// of floor 0 with direction UNKNOWN before any write.
    pub fn landing_call_control(&self) -> &BACnetLandingCallStatus {
        &self.landing_call_control
    }

    /// The active landing calls served as Landing_Calls.
    pub fn landing_calls(&self) -> &[BACnetLandingCallStatus] {
        &self.landing_calls
    }

    /// Replace the active landing calls served as Landing_Calls.
    ///
    /// The application owns this list: a Landing_Call_Control write doesn't
    /// add to it. A call with a direction outside BACnetLiftCarDirection is
    /// refused with VALUE_OUT_OF_RANGE and the list is left unchanged.
    pub fn set_landing_calls(&mut self, calls: Vec<BACnetLandingCallStatus>) -> Result<(), Error> {
        calls.iter().try_for_each(landing_calls::validate)?;
        self.landing_calls = calls;
        Ok(())
    }
}

impl BACnetObject for ElevatorGroupObject {
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
        // No read_common_properties!: it would serve Status_Flags,
        // Out_Of_Service and Reliability, which Table 12-76 doesn't define.
        match property {
            p if p == PropertyIdentifier::OBJECT_IDENTIFIER => {
                Ok(PropertyValue::ObjectIdentifier(self.oid))
            }
            p if p == PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            p if p == PropertyIdentifier::DESCRIPTION => {
                Ok(PropertyValue::CharacterString(self.description.clone()))
            }
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::ELEVATOR_GROUP.to_raw(),
            )),
            p if p == PropertyIdentifier::PROPERTY_LIST => {
                common::read_property_list_property(&self.property_list(), array_index)
            }
            p if p == PropertyIdentifier::MACHINE_ROOM_ID => {
                Ok(PropertyValue::ObjectIdentifier(self.machine_room_id))
            }
            p if p == PropertyIdentifier::GROUP_ID => {
                Ok(PropertyValue::Unsigned(u64::from(self.group_id)))
            }
            // A BACnetARRAY (Table 12-76): the whole array, its size at
            // index 0, or one member.
            p if p == PropertyIdentifier::GROUP_MEMBERS => common::read_array(
                self.group_members
                    .iter()
                    .copied()
                    .map(PropertyValue::ObjectIdentifier)
                    .collect(),
                array_index,
            ),
            p if p == PropertyIdentifier::GROUP_MODE => {
                Ok(PropertyValue::Enumerated(self.group_mode.to_raw()))
            }
            p if p == PropertyIdentifier::LANDING_CALLS => Ok(PropertyValue::List(
                self.landing_calls
                    .iter()
                    .map(landing_calls::encode)
                    .collect::<Result<Vec<_>, Error>>()?,
            )),
            p if p == PropertyIdentifier::LANDING_CALL_CONTROL => {
                landing_calls::encode(&self.landing_call_control)
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::GROUP_ID => {
                if let PropertyValue::Unsigned(v) = value {
                    // Group_ID is an Unsigned8 (Table 12-76).
                    self.group_id =
                        u8::try_from(v).map_err(|_| common::value_out_of_range_error())?;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::GROUP_MODE => {
                if let PropertyValue::Enumerated(v) = value {
                    self.group_mode = LiftGroupMode::from_raw(v);
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::LANDING_CALL_CONTROL => {
                self.landing_call_control = landing_calls::decode_write(value)?;
                Ok(())
            }
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
                _array_index,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_elevator_group_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
