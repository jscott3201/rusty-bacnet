//! Tracking_Value and Reliability while Out_Of_Service is TRUE (Clauses
//! 12.15.11 and 12.16.11, Tables 12-18 and 12-19, #1108).
//!
//! Footnote 1 of both tables marks Tracking_Value and Reliability, so a client
//! has to be able to simulate either one by writing it. A Point or Zone takes
//! WriteProperty and WritePropertyMultiple of the two only while
//! Out_Of_Service is TRUE, and refuses them with WRITE_ACCESS_DENIED in
//! service. A Tracking_Value has to
//! be an Enumerated holding a standard BACnetLifeSafetyState or one from the
//! proprietary range 256..=65535; a Reliability has to pass
//! `common::is_reliability_value_valid`. Any other number is
//! VALUE_OUT_OF_RANGE and any other datatype INVALID_DATA_TYPE, with the
//! property left as it was.
//!
//! Out of service the two values no longer follow the device, so the object
//! keeps the device's own values to one side:
//!
//! - On the FALSE-to-TRUE edge it puts aside the Tracking_Value and
//!   Reliability it serves.
//! - Meanwhile a Tracking_Value the application reports (`set_tracking_value`,
//!   the `LocalInput` Tracking_Value or a reset commit) replaces the value put
//!   aside, not the one served, and `set_reliability_internal` is refused, as
//!   on the other Reliability carriers.
//! - On the TRUE-to-FALSE edge both go back to the values put aside, so the
//!   device's state is served again at once and the simulated values are gone.
//!
//! Anything that reads the two properties sees a simulated value the way it
//! would see one from the device:
//!
//! - COV: the server snapshots the Life Safety COV surface around every write
//!   (`life_safety_cov` in bacnet-server). A simulated Tracking_Value reaches
//!   its property subscribers, and a simulated Reliability that flips the
//!   FAULT bit changes Status_Flags, which reaches every subscriber. The
//!   return to service is a write too, so restoring the device's values
//!   notifies the same way.
//! - Present_Value: the object never computes Present_Value from
//!   Tracking_Value (Clauses 12.15.4 and 12.16.4 leave the derivation and any
//!   latching to the implementation, here the application), so a simulated
//!   Tracking_Value does not latch it. The tables give Present_Value no
//!   out-of-service footnote, so it stays the application's in both states:
//!   `set_present_value`, the `LocalInput` Present_Value and a reset commit set
//!   it as usual (`application.rs`). A reset executor's context carries the
//!   Tracking_Value the object serves, so a latching rule in the executor
//!   reacts to a simulated value as to a real one.
//! - Silenced and Operation_Expected don't depend on Tracking_Value here and
//!   are left alone; LifeSafetyOperation works the same out of service.
//! - Event reporting: CHANGE_OF_LIFE_SAFETY (Clause 13.3.8) takes
//!   Present_Value as its monitored value, plus Mode, Status_Flags and
//!   Operation_Expected, and never Tracking_Value. These objects run no
//!   intrinsic reporting either (Event_State stays NORMAL, Clauses 12.15.9 and
//!   12.16.9), so neither simulated value raises an event; a simulated
//!   Reliability shows only as the FAULT bit.

use bacnet_types::enums::{LifeSafetyState, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::common;

/// The device's Tracking_Value and Reliability, put aside while a client
/// simulates them. Both are `Some` only while Out_Of_Service is TRUE.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct DeviceValues {
    reliability: Option<Reliability>,
    tracking_value: Option<LifeSafetyState>,
}

/// The fields of a Point or Zone that an out-of-service simulation touches,
/// borrowed together.
pub(super) struct Simulation<'a> {
    pub(super) out_of_service: &'a mut bool,
    pub(super) reliability: &'a mut Reliability,
    pub(super) tracking_value: &'a mut LifeSafetyState,
    pub(super) device_values: &'a mut DeviceValues,
}

impl Simulation<'_> {
    /// Apply a network write of Out_Of_Service, Tracking_Value or
    /// Reliability; `None` for any other property.
    pub(super) fn write(
        &mut self,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if property == PropertyIdentifier::TRACKING_VALUE {
            return Some(self.write_tracking_value(value));
        }
        if property == PropertyIdentifier::RELIABILITY {
            return Some(self.write_reliability(value));
        }
        let was_out_of_service = *self.out_of_service;
        let result = common::write_out_of_service_with_reliability_restore(
            self.out_of_service,
            self.reliability,
            &mut self.device_values.reliability,
            property,
            value,
        )?;
        match (was_out_of_service, *self.out_of_service) {
            (false, true) => self.device_values.tracking_value = Some(*self.tracking_value),
            (true, false) => {
                if let Some(state) = self.device_values.tracking_value.take() {
                    *self.tracking_value = state;
                }
            }
            _ => {}
        }
        Some(result)
    }

    fn write_tracking_value(&mut self, value: &PropertyValue) -> Result<(), Error> {
        if !*self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        *self.tracking_value = super::application::life_safety_state(value)?;
        Ok(())
    }

    fn write_reliability(&mut self, value: &PropertyValue) -> Result<(), Error> {
        if !*self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        let PropertyValue::Enumerated(raw) = value else {
            return Err(common::invalid_data_type_error());
        };
        let reliability = Reliability::from_raw(*raw);
        if !common::is_reliability_value_valid(reliability) {
            return Err(common::value_out_of_range_error());
        }
        *self.reliability = reliability;
        Ok(())
    }

    /// Take a Tracking_Value from the application: served at once in service,
    /// put aside for the return to service while a client simulates.
    pub(super) fn track(&mut self, state: LifeSafetyState) {
        if *self.out_of_service {
            self.device_values.tracking_value = Some(state);
        } else {
            *self.tracking_value = state;
        }
    }

    /// The application's Reliability: refused while a client simulates it.
    pub(super) fn set_reliability(&mut self, reliability: Reliability) -> Result<(), Error> {
        if *self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        if !common::is_reliability_value_valid(reliability) {
            return Err(common::value_out_of_range_error());
        }
        *self.reliability = reliability;
        Ok(())
    }
}
