//! Loop (type 12) object per ASHRAE 135-2020 Clause 12.17.
//!
//! PID control loop. The application is responsible for running the PID
//! algorithm; this object stores configuration and current output.
//! Action and Priority_For_Writing are configuration for that algorithm:
//! the object itself neither computes an output nor commands the property
//! that Manipulated_Variable_Reference names, so nothing here changes with
//! them. While the application runs the algorithm it also feeds
//! Controlled_Variable_Value, the measurement it compares with Setpoint; the
//! object doesn't follow Controlled_Variable_Reference itself.

use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{
    Action, EngineeringUnits, ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier,
    Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_property_list_property};
use crate::reference::{self, ReferenceFrame};
use crate::traits::BACnetObject;

mod metadata;

/// BACnet Loop object — PID control loop configuration and state.
pub struct LoopObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    setpoint: f32,
    controlled_variable_value: f32,
    cov_increment: f32,
    proportional_constant: f32,
    integral_constant: f32,
    derivative_constant: f32,
    output_units: EngineeringUnits,
    controlled_variable_units: EngineeringUnits,
    proportional_constant_units: EngineeringUnits,
    integral_constant_units: EngineeringUnits,
    derivative_constant_units: EngineeringUnits,
    action: Action,
    priority_for_writing: u8,
    update_interval: u32,
    out_of_service: bool,
    reliability: Reliability,
    /// Evaluated Reliability saved while a client simulation owns the property
    /// (Out_Of_Service TRUE); restored on the return to service.
    reliability_before_out_of_service: Option<Reliability>,
    status_flags: StatusFlags,
    controlled_variable_reference: Option<BACnetObjectPropertyReference>,
    manipulated_variable_reference: Option<BACnetObjectPropertyReference>,
    setpoint_reference: Option<BACnetObjectPropertyReference>,
}

impl LoopObject {
    /// Create a new Loop object; `output_units` is a raw BACnetEngineeringUnits value for the
    /// output.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        output_units: EngineeringUnits,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::LOOP, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0.0,
            setpoint: 0.0,
            controlled_variable_value: 0.0,
            cov_increment: 0.0,
            proportional_constant: 1.0,
            integral_constant: 0.0,
            derivative_constant: 0.0,
            output_units,
            controlled_variable_units: EngineeringUnits::NO_UNITS,
            proportional_constant_units: EngineeringUnits::NO_UNITS,
            integral_constant_units: EngineeringUnits::NO_UNITS,
            derivative_constant_units: EngineeringUnits::NO_UNITS,
            action: Action::DIRECT,
            priority_for_writing: 16, // the lowest command priority
            update_interval: 1000,    // milliseconds
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            reliability_before_out_of_service: None,
            status_flags: StatusFlags::empty(),
            controlled_variable_reference: None,
            manipulated_variable_reference: None,
            setpoint_reference: None,
        })
    }

    /// Application sets the current output value after PID computation.
    ///
    /// This direct setter bypasses the server's COV fanout and the
    /// Out_Of_Service guard; a running server's application uses
    /// `BACnetServer::set_present_value_local` instead.
    pub fn set_present_value(&mut self, value: f32) {
        self.present_value = value;
    }

    /// Application sets the measured value of the property that
    /// Controlled_Variable_Reference names (Clause 12.17.14).
    ///
    /// COV notifications carry the latest value, but a change of it alone
    /// sends none (Table 13-1). Like [`Self::set_present_value`] this direct
    /// setter bypasses the server's COV fanout and value checks; a running
    /// server's application uses
    /// `BACnetServer::set_controlled_variable_value_local` instead.
    pub fn set_controlled_variable_value(&mut self, value: f32) {
        self.controlled_variable_value = value;
    }

    /// Validate and store a Present_Value write, without any access check.
    ///
    /// Shared by the network and internal routes, which differ only in the
    /// Out_Of_Service condition each requires.
    fn apply_present_value(&mut self, value: PropertyValue) -> Result<(), Error> {
        write_finite_real(&mut self.present_value, value)
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the controlled variable reference (the object whose present value is
    /// being controlled by this loop). A reference to the reserved instance
    /// 4194303 clears it, as a client's write of the unset form does (#1417).
    pub fn set_controlled_variable_reference(&mut self, r: BACnetObjectPropertyReference) {
        self.controlled_variable_reference = reference::set_or_unset(r);
    }

    /// Set the manipulated variable reference (the object that the loop output
    /// drives to achieve the setpoint). A reference to the reserved instance
    /// 4194303 clears it (#1417).
    pub fn set_manipulated_variable_reference(&mut self, r: BACnetObjectPropertyReference) {
        self.manipulated_variable_reference = reference::set_or_unset(r);
    }

    /// Set the setpoint reference (an alternative way to supply the setpoint
    /// from another object's property instead of the inline `SETPOINT` value).
    pub fn set_setpoint_reference(&mut self, r: BACnetObjectPropertyReference) {
        self.setpoint_reference = Some(r);
    }

    /// Set the units of Controlled_Variable_Value and Setpoint, served as
    /// Controlled_Variable_Units (Clause 12.17.15). A new Loop uses NO_UNITS.
    ///
    /// The property is read-only over the network. A value above 65535,
    /// outside BACnetEngineeringUnits, is refused with VALUE_OUT_OF_RANGE and
    /// the property is left unchanged.
    pub fn set_controlled_variable_units(&mut self, units: EngineeringUnits) -> Result<(), Error> {
        self.controlled_variable_units = checked_units(units)?;
        Ok(())
    }

    /// Set Proportional_Constant_Units (Clause 12.17.20), the units the
    /// algorithm gives its proportional gain. A new Loop uses NO_UNITS.
    /// Read-only over the network; refuses a value above 65535 like
    /// [`Self::set_controlled_variable_units`].
    pub fn set_proportional_constant_units(
        &mut self,
        units: EngineeringUnits,
    ) -> Result<(), Error> {
        self.proportional_constant_units = checked_units(units)?;
        Ok(())
    }

    /// Set Integral_Constant_Units (Clause 12.17.22). A new Loop uses
    /// NO_UNITS. Read-only over the network; refuses a value above 65535.
    pub fn set_integral_constant_units(&mut self, units: EngineeringUnits) -> Result<(), Error> {
        self.integral_constant_units = checked_units(units)?;
        Ok(())
    }

    /// Set Derivative_Constant_Units (Clause 12.17.24). A new Loop uses
    /// NO_UNITS. Read-only over the network; refuses a value above 65535.
    pub fn set_derivative_constant_units(&mut self, units: EngineeringUnits) -> Result<(), Error> {
        self.derivative_constant_units = checked_units(units)?;
        Ok(())
    }

    /// Set Priority_For_Writing (Clause 12.17.28), the command priority the
    /// loop's output holds in the Priority_Array of the property that
    /// Manipulated_Variable_Reference names. A new Loop uses 16, the lowest.
    ///
    /// The property is read-only over the network. A priority outside 1..=16
    /// is refused with VALUE_OUT_OF_RANGE and the property is left unchanged.
    pub fn set_priority_for_writing(&mut self, priority: u8) -> Result<(), Error> {
        if !(1..=16).contains(&priority) {
            return Err(common::value_out_of_range_error());
        }
        self.priority_for_writing = priority;
        Ok(())
    }
}

/// BACnetEngineeringUnits is an enumeration capped at 65535 (Clause 21).
fn checked_units(units: EngineeringUnits) -> Result<EngineeringUnits, Error> {
    if units.to_raw() > 65_535 {
        return Err(common::value_out_of_range_error());
    }
    Ok(units)
}

/// Store a finite REAL write into `target`.
fn write_finite_real(target: &mut f32, value: PropertyValue) -> Result<(), Error> {
    let PropertyValue::Real(v) = value else {
        return Err(common::invalid_data_type_error());
    };
    common::reject_non_finite(v)?;
    *target = v;
    Ok(())
}

impl BACnetObject for LoopObject {
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
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::LOOP.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Real(self.present_value))
            }
            p if p == PropertyIdentifier::SETPOINT => Ok(PropertyValue::Real(self.setpoint)),
            p if p == PropertyIdentifier::CONTROLLED_VARIABLE_VALUE => {
                Ok(PropertyValue::Real(self.controlled_variable_value))
            }
            p if p == PropertyIdentifier::COV_INCREMENT => {
                Ok(PropertyValue::Real(self.cov_increment))
            }
            p if p == PropertyIdentifier::PROPORTIONAL_CONSTANT => {
                Ok(PropertyValue::Real(self.proportional_constant))
            }
            p if p == PropertyIdentifier::INTEGRAL_CONSTANT => {
                Ok(PropertyValue::Real(self.integral_constant))
            }
            p if p == PropertyIdentifier::DERIVATIVE_CONSTANT => {
                Ok(PropertyValue::Real(self.derivative_constant))
            }
            p if p == PropertyIdentifier::OUTPUT_UNITS => {
                Ok(PropertyValue::Enumerated(self.output_units.to_raw()))
            }
            p if p == PropertyIdentifier::CONTROLLED_VARIABLE_UNITS => Ok(
                PropertyValue::Enumerated(self.controlled_variable_units.to_raw()),
            ),
            p if p == PropertyIdentifier::PROPORTIONAL_CONSTANT_UNITS => Ok(
                PropertyValue::Enumerated(self.proportional_constant_units.to_raw()),
            ),
            p if p == PropertyIdentifier::INTEGRAL_CONSTANT_UNITS => Ok(PropertyValue::Enumerated(
                self.integral_constant_units.to_raw(),
            )),
            p if p == PropertyIdentifier::DERIVATIVE_CONSTANT_UNITS => Ok(
                PropertyValue::Enumerated(self.derivative_constant_units.to_raw()),
            ),
            p if p == PropertyIdentifier::ACTION => {
                Ok(PropertyValue::Enumerated(self.action.to_raw()))
            }
            p if p == PropertyIdentifier::PRIORITY_FOR_WRITING => {
                Ok(PropertyValue::Unsigned(self.priority_for_writing.into()))
            }
            p if p == PropertyIdentifier::UPDATE_INTERVAL => {
                Ok(PropertyValue::Unsigned(self.update_interval as u64))
            }
            // FAULT follows Reliability and OUT_OF_SERVICE follows
            // Out_Of_Service; IN_ALARM follows the fixed NORMAL Event_State
            // this object reports.
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(common::compute_status_flags(
                self.status_flags,
                self.reliability,
                self.out_of_service,
                EventState::NORMAL,
            )),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(EventState::NORMAL.to_raw()))
            }
            p if p == PropertyIdentifier::RELIABILITY => {
                Ok(PropertyValue::Enumerated(self.reliability.to_raw()))
            }
            p if p == PropertyIdentifier::OUT_OF_SERVICE => {
                Ok(PropertyValue::Boolean(self.out_of_service))
            }
            // Each reference reads as its Clause 21 encoding (#1312); see
            // reference.rs for the unset forms (#1417): the measured input,
            // or the commanded output, at the reserved instance.
            p if p == PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE => {
                Ok(reference::object_property_reference_value(
                    self.controlled_variable_reference.as_ref(),
                    ObjectType::ANALOG_INPUT,
                ))
            }
            p if p == PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE => {
                Ok(reference::object_property_reference_value(
                    self.manipulated_variable_reference.as_ref(),
                    ObjectType::ANALOG_OUTPUT,
                ))
            }
            p if p == PropertyIdentifier::SETPOINT_REFERENCE => Ok(
                reference::setpoint_reference_value(self.setpoint_reference.as_ref()),
            ),
            p if p == PropertyIdentifier::PROPERTY_LIST => {
                read_property_list_property(&self.property_list(), array_index)
            }
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::UNKNOWN_PROPERTY,
            }),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        // Clause 12.17.9 decouples Present_Value from the algorithm while
        // Out_Of_Service is TRUE and Table 12-20 footnote 7 makes it writable
        // then, for simulation and testing. In service the algorithm owns it;
        // the application route is `set_present_value_internal`.
        if property == PropertyIdentifier::PRESENT_VALUE {
            if !self.out_of_service {
                return Err(common::write_access_denied_error());
            }
            return self.apply_present_value(value);
        }
        if let Some(result) = common::write_out_of_service_with_reliability_restore(
            &mut self.out_of_service,
            &mut self.reliability,
            &mut self.reliability_before_out_of_service,
            property,
            &value,
        ) {
            return result;
        }
        if let Some(result) = common::write_cov_increment(&mut self.cov_increment, property, &value)
        {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::SETPOINT => write_finite_real(&mut self.setpoint, value),
            p if p == PropertyIdentifier::PROPORTIONAL_CONSTANT => {
                write_finite_real(&mut self.proportional_constant, value)
            }
            p if p == PropertyIdentifier::INTEGRAL_CONSTANT => {
                write_finite_real(&mut self.integral_constant, value)
            }
            p if p == PropertyIdentifier::DERIVATIVE_CONSTANT => {
                write_finite_real(&mut self.derivative_constant, value)
            }
            // BACnetAction names only DIRECT and REVERSE (Clause 21).
            p if p == PropertyIdentifier::ACTION => {
                let PropertyValue::Enumerated(raw) = value else {
                    return Err(common::invalid_data_type_error());
                };
                let action = Action::from_raw(raw);
                if action != Action::DIRECT && action != Action::REVERSE {
                    return Err(common::value_out_of_range_error());
                }
                self.action = action;
                Ok(())
            }
            p if p == PropertyIdentifier::UPDATE_INTERVAL => {
                if let PropertyValue::Unsigned(v) = value {
                    self.update_interval = common::u64_to_u32(v)?;
                    return Ok(());
                }
                Err(Error::Protocol {
                    class: ErrorClass::PROPERTY,
                    code: ErrorCode::INVALID_DATA_TYPE,
                })
            }
            // Clause 12.17 Table 12-20 lists Reliability O7; that footnote requires
            // writes to be supported while Out_Of_Service is TRUE. The property's
            // text specifies simulation/test writes to Present_Value and to
            // Reliability when present and capable of values beyond
            // NO_FAULT_DETECTED. In service the property is owned
            // by the algorithm, so a network write is refused here; the internal
            // evaluator route is `set_reliability_internal` with the complementary
            // guard, and Out_Of_Service saves/restores the evaluated value (handled
            // above the match).
            p if p == PropertyIdentifier::RELIABILITY => {
                if !self.out_of_service {
                    return Err(common::write_access_denied_error());
                }
                if let PropertyValue::Enumerated(raw) = value {
                    let v = Reliability::from_raw(raw);
                    if !common::is_reliability_value_valid(v) {
                        return Err(common::value_out_of_range_error());
                    }
                    self.reliability = v;
                    return Ok(());
                }
                Err(Error::Protocol {
                    class: ErrorClass::PROPERTY,
                    code: ErrorCode::INVALID_DATA_TYPE,
                })
            }
            p if p == PropertyIdentifier::DESCRIPTION => {
                if let PropertyValue::CharacterString(s) = value {
                    self.description = s;
                    return Ok(());
                }
                Err(Error::Protocol {
                    class: ErrorClass::PROPERTY,
                    code: ErrorCode::INVALID_DATA_TYPE,
                })
            }
            // Table 12-20 types the two variable references
            // BACnetObjectPropertyReference: a write takes the reference's
            // context-tagged members, the unset form clearing it, and Null
            // is a value of another datatype (reference.rs).
            p if p == PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE => {
                self.controlled_variable_reference =
                    reference::decode_reference_write(&value, ReferenceFrame::Bare)?;
                Ok(())
            }
            p if p == PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE => {
                self.manipulated_variable_reference =
                    reference::decode_reference_write(&value, ReferenceFrame::Bare)?;
                Ok(())
            }
            // Setpoint_Reference is BACnetSetpointReference: the members
            // framed in context tag 0, or the empty value for none.
            p if p == PropertyIdentifier::SETPOINT_REFERENCE => {
                self.setpoint_reference =
                    reference::decode_reference_write(&value, ReferenceFrame::Setpoint)?;
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
        metadata::for_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn cov_increment(&self) -> Option<f64> {
        Some(f64::from(self.cov_increment))
    }

    fn set_present_value_internal(&mut self, value: PropertyValue) -> Result<(), Error> {
        // The complement of the network route: while Out_Of_Service is TRUE
        // a client's simulated output must not be overwritten by the algorithm.
        if self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        self.apply_present_value(value)
    }

    fn set_controlled_variable_value_internal(
        &mut self,
        value: PropertyValue,
    ) -> Result<(), Error> {
        // Out_Of_Service decouples only Present_Value and Reliability from the
        // algorithm (Clause 12.17.9), so the measurement keeps flowing while a
        // client simulates the output.
        write_finite_real(&mut self.controlled_variable_value, value)
    }

    fn set_reliability_internal(&mut self, reliability: Reliability) -> Result<(), Error> {
        // While Out_Of_Service is TRUE the client owns the simulated value;
        // an internal write would clobber the simulation (Clause 12.17
        // Out_Of_Service paragraph separates Reliability from algorithm output),
        // so it is refused until the object returns to service.
        if self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        if !common::is_reliability_value_valid(reliability) {
            return Err(common::value_out_of_range_error());
        }
        self.reliability = reliability;
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod property_set_tests;

#[cfg(test)]
mod reference_tests;
