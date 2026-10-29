//! Energy_Meter and Energy_Meter_Ref, the optional pair the Lift (Clause
//! 12.59, Table 12-77) and Escalator (Clause 12.60, Table 12-78) tables
//! share: the car's or escalator's running energy total in kilowatt-hours,
//! or a reference to a meter object that keeps that total instead.
//!
//! Clauses 12.59 and 12.60 tie the two together: while Energy_Meter_Ref is
//! initialized (its object instance isn't 4194303), Energy_Meter reads 0.0.
//! Setting a reference therefore zeroes the reading, and a write of any other
//! value while the reference is set is refused with VALUE_OUT_OF_RANGE, since
//! 0.0 is then the only value in the property's range.
//!
//! Both tables give Energy_Meter_Ref the O code with no write requirement,
//! and the reference describes how the installation is metered, which the
//! application owns. So it is read-only over the network, like the
//! membership rows: a WriteProperty is refused with WRITE_ACCESS_DENIED, and
//! the application sets it through the setter [`energy_meter_accessors`] adds
//! to each object. Energy_Meter itself takes writes, which item (c) of each
//! Out_Of_Service description requires while the object is out of service.

use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::common;

/// The standard object types Energy_Meter_Ref may name. Clauses 12.59 and
/// 12.60 ask only for the object that indicates the accumulated consumption,
/// so these are the standard object types whose Present_Value is a measured
/// number in engineering units: the Accumulator and Pulse Converter meters,
/// the analog inputs and values, and the integer values. Proprietary object
/// types ([`PROPRIETARY_TYPES`]) are accepted too, since vendor meter objects
/// are common.
const METER_TYPES: [ObjectType; 7] = [
    ObjectType::ACCUMULATOR,
    ObjectType::PULSE_CONVERTER,
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_VALUE,
    ObjectType::LARGE_ANALOG_VALUE,
    ObjectType::INTEGER_VALUE,
    ObjectType::POSITIVE_INTEGER_VALUE,
];

/// The proprietary object type range, open to vendors (Clause 23).
const PROPRIETARY_TYPES: core::ops::RangeInclusive<u32> = 128..=1023;

/// Whether `object_type` can serve as the meter Energy_Meter_Ref names.
fn is_meter_type(object_type: ObjectType) -> bool {
    METER_TYPES.contains(&object_type) || PROPRIETARY_TYPES.contains(&object_type.to_raw())
}

/// The energy metering of one lift or escalator.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct EnergyMeter {
    /// Energy_Meter (REAL, kilowatt-hours); 0.0 while `reference` is
    /// initialized.
    reading: f32,
    /// Energy_Meter_Ref (BACnetDeviceObjectReference).
    reference: BACnetDeviceObjectReference,
}

impl EnergyMeter {
    /// A reading of 0.0 and an uninitialized reference: Accumulator instance
    /// 4194303 with no device.
    pub(super) fn new() -> Result<Self, Error> {
        Ok(Self {
            reading: 0.0,
            reference: BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: ObjectIdentifier::new(
                    ObjectType::ACCUMULATOR,
                    ObjectIdentifier::MAX_INSTANCE,
                )?,
            },
        })
    }

    /// The reference served as Energy_Meter_Ref.
    pub(super) fn reference(&self) -> &BACnetDeviceObjectReference {
        &self.reference
    }

    /// Whether Energy_Meter_Ref is initialized: it names an object instance
    /// other than 4194303.
    fn is_referenced(&self) -> bool {
        self.reference.object_identifier.instance_number() != ObjectIdentifier::MAX_INSTANCE
    }

    /// The value of `property` if it is Energy_Meter or Energy_Meter_Ref.
    pub(super) fn read(&self, property: PropertyIdentifier) -> Option<PropertyValue> {
        match property {
            PropertyIdentifier::ENERGY_METER => Some(PropertyValue::Real(self.reading)),
            PropertyIdentifier::ENERGY_METER_REF => {
                Some(crate::device_reference::reference_value(&self.reference))
            }
            _ => None,
        }
    }

    /// Handle a WriteProperty on Energy_Meter; `None` for any other property.
    ///
    /// A value that isn't a REAL is INVALID_DATA_TYPE. A non-finite value,
    /// or anything but 0.0 while Energy_Meter_Ref is initialized, is
    /// VALUE_OUT_OF_RANGE. Either way the reading is left unchanged.
    pub(super) fn write(
        &mut self,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if property != PropertyIdentifier::ENERGY_METER {
            return None;
        }
        let &PropertyValue::Real(reading) = value else {
            return Some(Err(common::invalid_data_type_error()));
        };
        Some(self.store_reading(reading))
    }

    fn store_reading(&mut self, reading: f32) -> Result<(), Error> {
        common::reject_non_finite(reading)?;
        if !self.is_referenced() {
            self.reading = reading;
        } else if reading != 0.0 {
            return Err(common::value_out_of_range_error());
        }
        Ok(())
    }

    /// Store a new Energy_Meter_Ref, zeroing the reading if it is
    /// initialized.
    ///
    /// The object must be one of [`METER_TYPES`] or a proprietary type, and a
    /// device, when given, must be a Device object; anything else is refused with
    /// VALUE_OUT_OF_RANGE and nothing changes. A reference to instance
    /// 4194303 clears it, and the reading then starts again from 0.0.
    pub(super) fn set_reference(
        &mut self,
        reference: BACnetDeviceObjectReference,
    ) -> Result<(), Error> {
        crate::device_reference::check_device_member(reference.device_identifier)?;
        if !is_meter_type(reference.object_identifier.object_type()) {
            return Err(common::value_out_of_range_error());
        }
        self.reference = reference;
        if self.is_referenced() {
            self.reading = 0.0;
        }
        Ok(())
    }
}

/// The public getter and setter for an object's `energy_meter` field.
/// `$object` names the object type in the docs.
macro_rules! energy_meter_accessors {
    ($object:literal) => {
        /// The reference served as Energy_Meter_Ref. Until the application
        /// sets one it is uninitialized: Accumulator instance 4194303 with no
        /// device.
        pub fn energy_meter_ref(&self) -> &BACnetDeviceObjectReference {
            self.energy_meter.reference()
        }

        /// Point Energy_Meter_Ref at the meter object that totals the energy
        #[doc = concat!("used by this ", $object, ".")]
        ///
        /// Energy_Meter_Ref is read-only over the network, so this is the
        /// only way to change it. The object must be an Accumulator, Pulse
        /// Converter, Analog Input, Analog Value, Large Analog Value, Integer
        /// Value, Positive Integer Value or proprietary object type (128 to
        /// 1023), and a device, when given, a Device object; anything else is
        /// refused with VALUE_OUT_OF_RANGE and the property is left unchanged.
        ///
        /// While the reference names an instance other than 4194303,
        /// Energy_Meter reads 0.0 (Clauses 12.59 and 12.60): setting it
        /// zeroes the reading, and a write of any other value is refused
        /// with VALUE_OUT_OF_RANGE. A reference to instance 4194303 clears
        /// it, and Energy_Meter then takes writes again, starting from 0.0.
        pub fn set_energy_meter_ref(
            &mut self,
            reference: BACnetDeviceObjectReference,
        ) -> Result<(), Error> {
            self.energy_meter.set_reference(reference)
        }
    };
}
pub(super) use energy_meter_accessors;
