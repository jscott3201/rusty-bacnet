//! The event algorithm parameters an intrinsic reporter keeps beside its
//! reporting settings (#2331), compiled with the `event-reporting` feature;
//! see the [parent module](super).
//!
//! A parameter stays in the object's detector. [`AlgorithmDetector`] lends
//! the detector's parameters to the store and takes saved ones back; a
//! staged write of one is checked by [`after`] with the macros the objects
//! write through, so the step refuses exactly what the object will.

use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;

use crate::common;
use crate::durable::PendingWrite;
use crate::event::{ChangeOfStateDetector, CommandFailureDetector, OutOfRangeDetector};
use crate::snapshot::write_config::parameters as format;
use crate::write_config::AlgorithmWriteConfig;

/// A detector whose parameters a write-config file keeps.
pub(crate) trait AlgorithmDetector {
    /// Every parameter as the detector holds it now. Time_Delay_Normal is
    /// `None` until something sets it.
    fn parameters(&self) -> AlgorithmWriteConfig;

    /// Take the parameters `saved` holds, as a write of each would: the
    /// value goes into the detector and nothing else changes. A delay
    /// already counting keeps its count, and Event_State stays; the next
    /// evaluation uses the new values.
    fn restore_parameters(&mut self, saved: &AlgorithmWriteConfig);
}

impl AlgorithmDetector for OutOfRangeDetector {
    fn parameters(&self) -> AlgorithmWriteConfig {
        AlgorithmWriteConfig {
            time_delay: Some(self.time_delay),
            time_delay_normal: self.time_delay_normal,
            high_limit: Some(self.high_limit),
            low_limit: Some(self.low_limit),
            deadband: Some(self.deadband),
            limit_enable: Some(self.limit_enable),
        }
    }

    fn restore_parameters(&mut self, saved: &AlgorithmWriteConfig) {
        if let Some(delay) = saved.time_delay {
            self.time_delay = delay;
        }
        if saved.time_delay_normal.is_some() {
            self.time_delay_normal = saved.time_delay_normal;
        }
        if let Some(limit) = saved.high_limit {
            self.high_limit = limit;
        }
        if let Some(limit) = saved.low_limit {
            self.low_limit = limit;
        }
        if let Some(deadband) = saved.deadband {
            self.deadband = deadband;
        }
        if let Some(enable) = saved.limit_enable {
            self.limit_enable = enable;
        }
    }
}

/// The detectors whose own parameters are the two time delays.
macro_rules! keeps_time_delays {
    ($($detector:ty),*) => {$(
        impl AlgorithmDetector for $detector {
            fn parameters(&self) -> AlgorithmWriteConfig {
                AlgorithmWriteConfig {
                    time_delay: Some(self.time_delay),
                    time_delay_normal: self.time_delay_normal,
                    ..Default::default()
                }
            }

            fn restore_parameters(&mut self, saved: &AlgorithmWriteConfig) {
                if let Some(delay) = saved.time_delay {
                    self.time_delay = delay;
                }
                if saved.time_delay_normal.is_some() {
                    self.time_delay_normal = saved.time_delay_normal;
                }
            }
        }
    )*};
}

keeps_time_delays!(ChangeOfStateDetector, CommandFailureDetector);

/// The mask bit of `property`, if objects of `object_type` keep it.
pub(super) fn kept_bit(object_type: ObjectType, property: P) -> Option<u32> {
    format::bit_of(property).filter(|bit| format::kept(object_type) & bit != 0)
}

/// The detector the event-row macros write, standing in for the object's.
struct Scratch {
    event_detector: OutOfRangeDetector,
}

/// What `write`, of a kept parameter, leaves of `rows`, or the object's
/// refusal of it.
pub(super) fn after(rows: &mut AlgorithmWriteConfig, write: &PendingWrite) -> Result<(), Error> {
    let mut scratch = Scratch {
        event_detector: OutOfRangeDetector::default(),
    };
    scratch.event_detector.restore_parameters(rows);
    let (property, value) = (write.property, write.value.clone());
    common::write_analog_event_properties!(scratch, property, value)
        .or_else(|| common::write_generic_event_properties!(scratch, property, value))
        .unwrap_or_else(|| Err(common::unknown_property_error()))?;
    format::overlay(rows, &scratch.event_detector.parameters());
    Ok(())
}
