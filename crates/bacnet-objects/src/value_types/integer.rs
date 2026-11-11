//! The Integer Value object (Clause 12.43, Table 12-50).
//!
//! It is written by hand rather than by the value-type macro so it can take
//! a [`PresentValueAccess`] and a presentation range, which the point-list
//! builder gives it (#2456). Built with [`IntegerValueObject::new`] it is
//! commandable and behaves as the macro's value types do: a write of
//! Present_Value, with or without a command origin, commands the priority
//! slot it names (16 when it names none).

use alloc::borrow::Cow;

use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier as P, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use super::cov_increment::CovIncrement;
use super::metadata::INTEGER_VALUE_BASE;
use crate::common::{self, read_common_properties};
use crate::prelude::*;
use crate::present_value_access::PresentValueAccess;
use crate::property_metadata::{
    PropertyConformance::Optional, PropertyMetadata, PropertyWriteCapability::ReadOnly,
};
use crate::traits::{BACnetObject, LocalInput, LocalInputValue, not_supported};

/// BACnet Integer Value object (type 45): a signed 32-bit Present_Value.
///
/// [`new`](Self::new) makes a commandable one; [`with_access`](Self::with_access)
/// takes any [`PresentValueAccess`]. Priority_Array, Relinquish_Default and
/// Current_Command_Priority exist only while it is commandable. A read-only
/// one takes a peer's Present_Value write only while Out_Of_Service is TRUE;
/// a writable one takes every write as Present_Value, ignoring the priority.
/// Either kind takes the application's Present_Value through
/// [`BACnetObject::local_input`] while in service. It doesn't track
/// Value_Source, so a write's or an input's origin is not kept.
///
/// Min_Pres_Value and Max_Pres_Value are absent until set, and only
/// describe the range: like the analog objects, nothing compares a written
/// value or Relinquish_Default with them.
pub struct IntegerValueObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: i32,
    out_of_service: bool,
    status_flags: StatusFlags,
    reliability: Reliability,
    access: PresentValueAccess,
    /// 16-level priority array. `None` = no command at that level.
    priority_array: [Option<i32>; 16],
    relinquish_default: i32,
    units: EngineeringUnits,
    /// COV_Increment, an Unsigned (Table 12-50).
    cov_increment: u64,
    min_pres_value: Option<i32>,
    max_pres_value: Option<i32>,
}

impl IntegerValueObject {
    /// Create an Integer Value with a commandable Present_Value.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        Self::with_access(instance, name, PresentValueAccess::Commandable)
    }

    /// Create an Integer Value whose Present_Value is written as `access`
    /// says (#2456). It starts at 0, in NO_UNITS, with a COV_Increment of
    /// 0 and no presentation range.
    pub fn with_access(
        instance: u32,
        name: impl Into<String>,
        access: PresentValueAccess,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::INTEGER_VALUE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            reliability: Reliability::NO_FAULT_DETECTED,
            access,
            priority_array: [None; 16],
            relinquish_default: 0,
            units: EngineeringUnits::NO_UNITS,
            cov_increment: <u64 as CovIncrement>::ZERO,
            min_pres_value: None,
            max_pres_value: None,
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the Relinquish_Default (#270), and resolve Present_Value anew
    /// from the priority array, so an empty array falls back to the new
    /// default at once. Every INTEGER is a valid default. One that isn't
    /// commandable has no Relinquish_Default: UNKNOWN_PROPERTY, as a write
    /// of it gets.
    pub fn set_relinquish_default(&mut self, value: i32) -> Result<(), Error> {
        if self.access != PresentValueAccess::Commandable {
            return Err(common::unknown_property_error());
        }
        self.relinquish_default = value;
        self.recalculate_present_value();
        Ok(())
    }

    /// The engineering units of Present_Value, served as Units.
    pub fn units(&self) -> EngineeringUnits {
        self.units
    }

    /// Set the engineering units of Present_Value. A new object uses
    /// NO_UNITS.
    ///
    /// Units is read-only over the network. A value above 65535, outside
    /// BACnetEngineeringUnits, is refused with VALUE_OUT_OF_RANGE and the
    /// property is left unchanged.
    pub fn set_units(&mut self, units: EngineeringUnits) -> Result<(), Error> {
        if units.to_raw() > 65_535 {
            return Err(common::value_out_of_range_error());
        }
        self.units = units;
        Ok(())
    }

    /// Set COV_Increment, the smallest Present_Value change that sends a
    /// COV notification. A new object uses 0, so any change notifies.
    /// WriteProperty goes through the same check, and every Unsigned value
    /// is a valid increment.
    pub fn set_cov_increment(&mut self, increment: u64) -> Result<(), Error> {
        self.cov_increment = increment.validate()?;
        Ok(())
    }

    /// Set Min_Pres_Value, the lowest Present_Value the object reports; the
    /// property is absent until set, and read-only over the network.
    pub fn set_min_pres_value(&mut self, value: i32) {
        self.min_pres_value = Some(value);
    }

    /// Set Max_Pres_Value, the highest Present_Value the object reports; the
    /// property is absent until set, and read-only over the network.
    pub fn set_max_pres_value(&mut self, value: i32) {
        self.max_pres_value = Some(value);
    }

    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
    }

    /// Whether `property` is absent under the object's access: a
    /// commandable row on an object that isn't. Those rows are all in the
    /// base table, so the range rows needn't be looked at.
    fn excludes(&self, property: P) -> bool {
        self.access.excludes(INTEGER_VALUE_BASE, property, false)
    }

    /// The rows before the access projects them: the base table, then the
    /// range rows that are set.
    fn rows(&self) -> Cow<'_, [PropertyMetadata]> {
        let mut rows = Cow::Borrowed(INTEGER_VALUE_BASE);
        for (property, set) in [
            (P::MIN_PRES_VALUE, self.min_pres_value.is_some()),
            (P::MAX_PRES_VALUE, self.max_pres_value.is_some()),
        ] {
            if set {
                rows.to_mut()
                    .push(PropertyMetadata::new(property, Optional, None, ReadOnly));
            }
        }
        rows
    }

    /// A commanded Present_Value at `priority` (16 when none), or its slot
    /// relinquished by a NULL.
    fn command(&mut self, value: PropertyValue, priority: Option<u8>) -> Result<(), Error> {
        let priority = priority.unwrap_or(16);
        if !(1..=16).contains(&priority) {
            return Err(common::value_out_of_range_error());
        }
        let slot = usize::from(priority - 1);
        self.priority_array[slot] = match value {
            PropertyValue::Null => None,
            other => Some(Self::checked_present_value(other)?),
        };
        self.recalculate_present_value();
        Ok(())
    }

    /// A peer's Present_Value write when it isn't commandable: refused
    /// while a read-only one is in service, and a NULL is a no-op, as on
    /// the Analog Value.
    fn write_direct_present_value(&mut self, value: PropertyValue) -> Result<(), Error> {
        if self.access == PresentValueAccess::ReadOnly && !self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        if value == PropertyValue::Null {
            return Ok(());
        }
        self.present_value = Self::checked_present_value(value)?;
        Ok(())
    }

    /// A Present_Value or Relinquish_Default: any INTEGER.
    fn checked_present_value(value: PropertyValue) -> Result<i32, Error> {
        match value {
            PropertyValue::Signed(value) => Ok(value),
            _ => Err(common::invalid_data_type_error()),
        }
    }
}

impl BACnetObject for IntegerValueObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.access.project(self.rows(), false)
    }

    fn read_property(&self, property: P, array_index: Option<u32>) -> Result<PropertyValue, Error> {
        crate::property_metadata::check_read_array_index(self, property, array_index)?;
        if self.excludes(property) {
            return Err(common::unknown_property_error());
        }
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        let range = |value: Option<i32>| value.map(PropertyValue::Signed);
        let value = match property {
            P::OBJECT_TYPE => PropertyValue::Enumerated(ObjectType::INTEGER_VALUE.to_raw()),
            P::PRESENT_VALUE => PropertyValue::Signed(self.present_value),
            P::PRIORITY_ARRAY => {
                return common::read_priority_array!(self, array_index, PropertyValue::Signed);
            }
            P::RELINQUISH_DEFAULT => PropertyValue::Signed(self.relinquish_default),
            P::CURRENT_COMMAND_PRIORITY => common::current_command_priority(&self.priority_array),
            P::UNITS => PropertyValue::Enumerated(self.units.to_raw()),
            P::COV_INCREMENT => self.cov_increment.to_property(),
            P::MIN_PRES_VALUE => {
                range(self.min_pres_value).ok_or_else(common::unknown_property_error)?
            }
            P::MAX_PRES_VALUE => {
                range(self.max_pres_value).ok_or_else(common::unknown_property_error)?
            }
            _ => return Err(common::unknown_property_error()),
        };
        Ok(value)
    }

    fn write_property(
        &mut self,
        property: P,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        if self.excludes(property) {
            return Err(common::unknown_property_error());
        }
        if property == P::PRESENT_VALUE {
            return match self.access {
                PresentValueAccess::Commandable => self.command(value, priority),
                PresentValueAccess::ReadOnly | PresentValueAccess::Writable => {
                    self.write_direct_present_value(value)
                }
            };
        }
        if property == P::RELINQUISH_DEFAULT {
            let default = Self::checked_present_value(value)?;
            return self.set_relinquish_default(default);
        }
        if property == P::COV_INCREMENT {
            let increment = <u64 as CovIncrement>::from_property(value)?;
            return self.set_cov_increment(increment);
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_object_name(&mut self.name, property, &value) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
        ))
    }

    crate::traits::lend_cov_reporting!();

    fn local_input(&mut self) -> Option<&mut dyn LocalInput> {
        Some(self)
    }
}

impl crate::creation::Creatable for IntegerValueObject {
    const CREATION_ONLY_PROPERTIES: &'static [P] = crate::analog::CREATION_ONLY;

    fn initialize_property(&mut self, property: P, value: PropertyValue) -> Result<(), Error> {
        crate::analog::initialize_units(&mut self.units, property, value)
    }
}

impl crate::traits::CovReporting for IntegerValueObject {
    fn cov_increment(&self) -> Option<f64> {
        Some(self.cov_increment.as_f64())
    }
}

/// The Present_Value the application supplies while the object isn't
/// commandable. Its origin isn't kept: the object doesn't track
/// Value_Source.
impl LocalInput for IntegerValueObject {
    fn apply_local_input(&mut self, input: LocalInputValue) -> Result<(), Error> {
        let LocalInputValue::PresentValue { value, .. } = input else {
            return Err(not_supported());
        };
        match self.access {
            PresentValueAccess::Commandable => Err(not_supported()),
            _ if self.out_of_service => Err(common::write_access_denied_error()),
            PresentValueAccess::ReadOnly | PresentValueAccess::Writable => {
                self.present_value = Self::checked_present_value(value)?;
                Ok(())
            }
        }
    }
}
