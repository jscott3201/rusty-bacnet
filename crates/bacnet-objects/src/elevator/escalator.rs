//! The Escalator object (type 58, Clause 12.60).

use crate::prelude::*;
use alloc::borrow::Cow;
use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::{
    EscalatorFault, EscalatorMode, EscalatorOperationDirection, ObjectType, PropertyIdentifier,
    Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use super::energy_meter::{EnergyMeter, energy_meter_accessors};
use super::membership::{GroupMembership, group_membership_accessors};
use super::{decode_fault_signals, metadata, named_or_proprietary};
use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// BACnet Escalator object — represents an escalator.
///
/// The object serves the rows of its table (Clause 12.60, Table 12-78) that
/// it implements, each with its table datatype.
pub struct EscalatorObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Elevator_Group, Group_ID and Installation_ID.
    membership: GroupMembership,
    /// Escalator mode (BACnetEscalatorMode, Clause 21); proprietary extensions
    /// (Clause 23.1) are preserved as raw values.
    pub(super) escalator_mode: EscalatorMode,
    /// Fault signal set (BACnetEscalatorFault, Clause 21).
    pub(super) fault_signals: Vec<EscalatorFault>,
    /// Energy_Meter and Energy_Meter_Ref.
    energy_meter: EnergyMeter,
    /// Power mode (Boolean).
    power_mode: bool,
    /// Operation direction (BACnetEscalatorOperationDirection, Clause 21);
    /// proprietary extensions (Clause 23.1) are preserved as raw values.
    pub(super) operation_direction: EscalatorOperationDirection,
    /// Passenger alarm state (Boolean).
    passenger_alarm: bool,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: Reliability,
}

impl EscalatorObject {
    /// Create a new Escalator object with default values.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ESCALATOR, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            membership: GroupMembership::new()?,
            escalator_mode: EscalatorMode::UNKNOWN,
            fault_signals: Vec::new(),
            energy_meter: EnergyMeter::new()?,
            power_mode: false,
            operation_direction: EscalatorOperationDirection::UNKNOWN,
            passenger_alarm: false,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
        })
    }

    group_membership_accessors!("escalator");
    energy_meter_accessors!("escalator");
}

impl BACnetObject for EscalatorObject {
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
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        if let Some(value) = self.membership.read(property) {
            return Ok(value);
        }
        if let Some(value) = self.energy_meter.read(property) {
            return Ok(value);
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ESCALATOR.to_raw()))
            }
            p if p == PropertyIdentifier::ESCALATOR_MODE => {
                Ok(PropertyValue::Enumerated(self.escalator_mode.to_raw()))
            }
            p if p == PropertyIdentifier::FAULT_SIGNALS => {
                let items: Vec<PropertyValue> = self
                    .fault_signals
                    .iter()
                    .map(|v| PropertyValue::Enumerated(v.to_raw()))
                    .collect();
                Ok(PropertyValue::List(items))
            }
            p if p == PropertyIdentifier::POWER_MODE => Ok(PropertyValue::Boolean(self.power_mode)),
            p if p == PropertyIdentifier::OPERATION_DIRECTION => {
                Ok(PropertyValue::Enumerated(self.operation_direction.to_raw()))
            }
            p if p == PropertyIdentifier::PASSENGER_ALARM => {
                Ok(PropertyValue::Boolean(self.passenger_alarm))
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
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if let Some(result) = self.energy_meter.write(property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::POWER_MODE => {
                if let PropertyValue::Boolean(v) = value {
                    self.power_mode = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::ESCALATOR_MODE => {
                if let PropertyValue::Enumerated(v) = value {
                    let mode = EscalatorMode::from_raw(v);
                    if !named_or_proprietary(EscalatorMode::ALL_NAMED, mode, v) {
                        return Err(common::value_out_of_range_error());
                    }
                    self.escalator_mode = mode;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::OPERATION_DIRECTION => {
                if let PropertyValue::Enumerated(v) = value {
                    // Validate before mutating so a refused write leaves the
                    // prior value intact.
                    let direction = EscalatorOperationDirection::from_raw(v);
                    if !named_or_proprietary(EscalatorOperationDirection::ALL_NAMED, direction, v) {
                        return Err(common::value_out_of_range_error());
                    }
                    self.operation_direction = direction;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::FAULT_SIGNALS => {
                self.fault_signals = decode_fault_signals(
                    value,
                    EscalatorFault::ALL_NAMED,
                    EscalatorFault::from_raw,
                )?;
                Ok(())
            }
            p if p == PropertyIdentifier::PASSENGER_ALARM => {
                if let PropertyValue::Boolean(v) = value {
                    self.passenger_alarm = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
                _array_index,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_escalator_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }
}
