//! Analog Input (type 0), Analog Output (type 1), and Analog Value (type 2) objects.
//!
//! Per ASHRAE 135-2020 Clauses 12.2 (AI), 12.3 (AO), and 12.4 (AV).

use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::event::{history::EventHistory, OutOfRangeDetector};
use crate::rollback::impl_intrinsic_write_rollback;
use crate::traits::{BACnetObject, ReliabilityEvaluation};

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
    fn reliability(self) -> u32 {
        match self {
            Self::UnderRange => Reliability::UNDER_RANGE.to_raw(),
            Self::OverRange => Reliability::OVER_RANGE.to_raw(),
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

    fn property_list(
        &self,
        base: &'static [PropertyIdentifier],
    ) -> Cow<'static, [PropertyIdentifier]> {
        if self.limits.is_none() {
            return Cow::Borrowed(base);
        }
        let mut properties = Vec::with_capacity(base.len() + 2);
        properties.extend_from_slice(base);
        properties.extend([
            PropertyIdentifier::FAULT_HIGH_LIMIT,
            PropertyIdentifier::FAULT_LOW_LIMIT,
        ]);
        Cow::Owned(properties)
    }

    fn clear_ownership(&mut self) {
        self.owned_fault = None;
    }

    fn evaluate(
        &mut self,
        monitored_value: f32,
        reliability: &mut u32,
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
                    .unwrap_or_else(|| Reliability::NO_FAULT_DETECTED.to_raw()),
                observed_fault,
            )
        } else if *reliability == Reliability::NO_FAULT_DETECTED.to_raw() {
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
