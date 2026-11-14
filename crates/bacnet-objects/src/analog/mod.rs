//! Analog Input (type 0), Analog Output (type 1), and Analog Value (type 2) objects.
//!
//! Per ASHRAE 135-2020 Clauses 12.2 (AI), 12.3 (AO), and 12.4 (AV).
//!
//! Each holds the state every I/O object shares (`IoCore`), an
//! OUT_OF_RANGE detector, the rows the three share (Units, COV_Increment
//! and the Present_Value bounds) and its own Present_Value; the Input and
//! the Value add the FAULT_OUT_OF_RANGE algorithm, the Output and the Value
//! a priority array.

use alloc::borrow::Cow;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
#[cfg(test)]
use bacnet_types::{
    enums::{ErrorClass, ErrorCode, EventState},
    primitives::BACnetTimeStamp,
};

use crate::common;
use crate::event::OutOfRangeDetector;
use crate::io_core::IoCore;
use crate::property_metadata::{
    PropertyConformance::Optional, PropertyMetadata, PropertyWriteCapability::ReadOnly,
};
use crate::traits::{BACnetObject, ReliabilityEvaluation};

mod metadata;

#[derive(Clone, Copy)]
struct FaultOutOfRangeLimits {
    low: f32,
    high: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnedRangeFault {
    UnderRange,
    OverRange,
}

impl OwnedRangeFault {
    fn reliability(self) -> Reliability {
        match self {
            Self::UnderRange => Reliability::UNDER_RANGE,
            Self::OverRange => Reliability::OVER_RANGE,
        }
    }
}

#[derive(Default)]
struct FaultOutOfRangeState {
    // Deliberately separate from the intrinsic OutOfRangeDetector: reliability
    // has strict boundaries, no deadband, and first-stage ownership precedence.
    limits: Option<FaultOutOfRangeLimits>,
    owned_fault: Option<OwnedRangeFault>,
}

impl FaultOutOfRangeState {
    fn configure(&mut self, low: f32, high: f32) -> Result<(), Error> {
        if !low.is_finite() || !high.is_finite() || low > high {
            return Err(common::value_out_of_range_error());
        }
        self.limits = Some(FaultOutOfRangeLimits { low, high });
        Ok(())
    }

    fn read_limit(&self, property: PropertyIdentifier) -> Option<PropertyValue> {
        let limits = self.limits?;
        match property {
            PropertyIdentifier::FAULT_LOW_LIMIT => Some(PropertyValue::Real(limits.low)),
            PropertyIdentifier::FAULT_HIGH_LIMIT => Some(PropertyValue::Real(limits.high)),
            _ => None,
        }
    }

    /// Add Fault_High_Limit and Fault_Low_Limit to `rows` once configured:
    /// the limits are paired.
    fn add_rows(&self, rows: &mut Cow<'_, [PropertyMetadata]>) {
        if self.limits.is_some() {
            rows.to_mut().extend([
                PropertyMetadata::new(
                    PropertyIdentifier::FAULT_HIGH_LIMIT,
                    Optional,
                    None,
                    ReadOnly,
                ),
                PropertyMetadata::new(
                    PropertyIdentifier::FAULT_LOW_LIMIT,
                    Optional,
                    None,
                    ReadOnly,
                ),
            ]);
        }
    }

    fn clear_ownership(&mut self) {
        self.owned_fault = None;
    }

    fn evaluate(
        &mut self,
        monitored_value: f32,
        reliability: &mut Reliability,
    ) -> Result<ReliabilityEvaluation, Error> {
        let Some(limits) = self.limits else {
            return Ok(ReliabilityEvaluation::Unchanged);
        };
        if !monitored_value.is_finite() {
            return Err(common::value_out_of_range_error());
        }
        let observed_fault = if monitored_value < limits.low {
            Some(OwnedRangeFault::UnderRange)
        } else if monitored_value > limits.high {
            Some(OwnedRangeFault::OverRange)
        } else {
            None
        };

        let (new_reliability, new_owner) = if self.owned_fault.is_some() {
            (
                observed_fault
                    .map(OwnedRangeFault::reliability)
                    .unwrap_or(Reliability::NO_FAULT_DETECTED),
                observed_fault,
            )
        } else if *reliability == Reliability::NO_FAULT_DETECTED {
            let Some(fault) = observed_fault else {
                return Ok(ReliabilityEvaluation::Unchanged);
            };
            (fault.reliability(), Some(fault))
        } else {
            return Ok(ReliabilityEvaluation::Unchanged);
        };

        if new_reliability == *reliability {
            return Ok(ReliabilityEvaluation::Unchanged);
        }
        let old_reliability = *reliability;
        *reliability = new_reliability;
        self.owned_fault = new_owner;
        Ok(ReliabilityEvaluation::Changed {
            old_reliability,
            new_reliability,
        })
    }
}

/// What a createable analog object takes only from a CreateObject initial
/// value (#1429): Units, which stays read-only to WriteProperty. Integer,
/// Positive Integer and Large Analog Value take it the same way.
pub(crate) const CREATION_ONLY: &[PropertyIdentifier] = &[PropertyIdentifier::UNITS];

/// Set `units` from a Units initial value. An Enumerated above 65535 is
/// outside BACnetEngineeringUnits (Clause 21) and is VALUE_OUT_OF_RANGE;
/// any other property is WRITE_ACCESS_DENIED, as `Creatable`'s default
/// answers.
pub(crate) fn initialize_units(
    units: &mut EngineeringUnits,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    if property != PropertyIdentifier::UNITS {
        return Err(common::write_access_denied_error());
    }
    let PropertyValue::Enumerated(raw) = value else {
        return Err(common::invalid_data_type_error());
    };
    if raw > 65_535 {
        return Err(common::value_out_of_range_error());
    }
    *units = EngineeringUnits::from_raw(raw);
    Ok(())
}

/// Units, COV_Increment and the optional Min_Pres_Value and Max_Pres_Value
/// of an analog object's Present_Value: the rows the three share beyond
/// their [`IoCore`].
struct AnalogProperties {
    units: EngineeringUnits,
    /// COV_Increment: minimum change threshold for COV notifications.
    /// Default 0.0 means notify on any write (including no-change).
    /// Set to a positive value for delta-based filtering.
    cov_increment: f32,
    /// Optional minimum engineering bound metadata for Present_Value.
    min_pres_value: Option<f32>,
    /// Optional maximum engineering bound metadata for Present_Value.
    max_pres_value: Option<f32>,
}

impl AnalogProperties {
    fn new(units: EngineeringUnits) -> Self {
        Self {
            units,
            cov_increment: 0.0,
            min_pres_value: None,
            max_pres_value: None,
        }
    }

    /// Read Units, COV_Increment or a bound; `None` for any other property.
    /// A bound that isn't set is an unknown property.
    fn read(&self, property: PropertyIdentifier) -> Option<Result<PropertyValue, Error>> {
        let bound = |value: Option<f32>| {
            value
                .map(PropertyValue::Real)
                .ok_or_else(common::unknown_property_error)
        };
        Some(match property {
            PropertyIdentifier::UNITS => Ok(PropertyValue::Enumerated(self.units.to_raw())),
            PropertyIdentifier::COV_INCREMENT => Ok(PropertyValue::Real(self.cov_increment)),
            PropertyIdentifier::MIN_PRES_VALUE => bound(self.min_pres_value),
            PropertyIdentifier::MAX_PRES_VALUE => bound(self.max_pres_value),
            _ => return None,
        })
    }

    /// Take a write of COV_Increment; `None` for any other property. Units
    /// is read-only, and the bounds are the application's.
    fn write(
        &mut self,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        common::write_cov_increment(&mut self.cov_increment, property, value)
    }

    /// Add the rows of the bounds that are set to `rows`, Min_Pres_Value
    /// first. They are independent of each other and of the fault limits.
    fn add_rows(&self, rows: &mut Cow<'_, [PropertyMetadata]>) {
        if self.min_pres_value.is_some() {
            rows.to_mut().push(PropertyMetadata::new(
                PropertyIdentifier::MIN_PRES_VALUE,
                Optional,
                None,
                ReadOnly,
            ));
        }
        if self.max_pres_value.is_some() {
            rows.to_mut().push(PropertyMetadata::new(
                PropertyIdentifier::MAX_PRES_VALUE,
                Optional,
                None,
                ReadOnly,
            ));
        }
    }
}

/// Give `$object`, an analog object holding [`AnalogProperties`] in
/// `analog`, what the three share around them: the COV_Increment and bound
/// setters, Units as a CreateObject initial value, and the COV_Increment its
/// COV reporting compares against.
macro_rules! impl_analog_properties {
    ($object:ty) => {
        impl $object {
            /// Set COV_Increment, the smallest Present_Value change that sends a
            /// COV notification (#2445). A new object uses 0, so any change
            /// notifies. As for a write of it, a negative or non-finite increment
            /// is VALUE_OUT_OF_RANGE and the old one stays.
            pub fn set_cov_increment(&mut self, increment: f32) -> Result<(), Error> {
                self.analog.cov_increment = common::checked_cov_increment(increment)?;
                Ok(())
            }

            /// Set minimum engineering-bound metadata; this is not a reliability fault limit.
            pub fn set_min_pres_value(&mut self, value: f32) {
                self.analog.min_pres_value = Some(value);
            }

            /// Set maximum engineering-bound metadata; this is not a reliability fault limit.
            pub fn set_max_pres_value(&mut self, value: f32) {
                self.analog.max_pres_value = Some(value);
            }
        }

        impl $crate::creation::Creatable for $object {
            const CREATION_ONLY_PROPERTIES: &'static [PropertyIdentifier] =
                $crate::analog::CREATION_ONLY;

            fn initialize_property(
                &mut self,
                property: PropertyIdentifier,
                value: PropertyValue,
            ) -> Result<(), Error> {
                $crate::analog::initialize_units(&mut self.analog.units, property, value)
            }
        }

        impl $crate::traits::CovReporting for $object {
            fn cov_increment(&self) -> Option<f64> {
                Some(f64::from(self.analog.cov_increment))
            }
        }
    };
}
use impl_analog_properties;

/// Give `$object`, an Analog Input or Value holding a
/// [`FaultOutOfRangeState`] in `fault_out_of_range`, its configuration
/// setter and its Reliability evaluation: FAULT_OUT_OF_RANGE over
/// Present_Value, while the object owns its Reliability.
macro_rules! impl_fault_out_of_range {
    ($object:ty) => {
        impl $object {
            /// Configure the optional object-owned FAULT_OUT_OF_RANGE algorithm.
            ///
            /// Both limits become readable together. Equal limits are valid; non-finite
            /// limits and a low limit greater than the high limit are rejected without
            /// changing the prior configuration.
            pub fn configure_fault_out_of_range(
                &mut self,
                low: f32,
                high: f32,
            ) -> Result<(), Error> {
                self.fault_out_of_range.configure(low, high)
            }
        }

        impl $crate::traits::ReliabilityEval for $object {
            fn set_reliability(&mut self, reliability: Reliability) -> Result<(), Error> {
                self.core.set_reliability(reliability)?;
                self.fault_out_of_range.clear_ownership();
                Ok(())
            }

            fn evaluate_reliability(&mut self) -> Result<ReliabilityEvaluation, Error> {
                if !self.core.owns_reliability() {
                    return Ok(ReliabilityEvaluation::Unchanged);
                }
                self.fault_out_of_range
                    .evaluate(self.present_value, &mut self.core.reliability)
            }

            fn reliability_evaluation_inhibited(&self) -> bool {
                self.core.reliability_inhibit.enabled()
            }
        }
    };
}
use impl_fault_out_of_range;

mod input;
mod output;
mod value;
pub use input::*;
pub use output::*;
pub use value::*;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
