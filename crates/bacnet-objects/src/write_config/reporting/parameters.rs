//! The event algorithm parameters an intrinsic reporter keeps beside its
//! reporting settings (#2331), compiled with the `event-reporting` feature;
//! see the [parent module](super).
//!
//! A parameter stays in the object's detector. [`AlgorithmDetector`] lends
//! the detector's parameters to the store, takes saved ones back, and makes
//! a write of one the way the objects holding that kind of detector do. A
//! staged write is checked by [`after`] on a fresh detector of the object's
//! own kind, so each family's step refuses exactly what its objects will.
//!
//! An Access Door and an Access Zone hold the CHANGE_OF_STATE detector
//! inside a `ChangeOfStateReporting`, which lends and restores through the
//! same impl and checks a staged Alarm_Values against the type's own
//! enumeration; a door's Fault_Values and Masked_Alarm_Values are beside
//! it, in the door's `DoorAlarmLists`. An Event Enrollment has no detector
//! of this kind (the server's pass runs its algorithms off the object's
//! properties), so it lends, restores and checks its three rows itself, in
//! `event_enrollment::kept`. The format's table isn't what these
//! impls are made from, so `parameter_table_tests` walks it: every row a
//! type keeps has to come back through the type's object.
//!
//! A saved parameter is checked as the file format checks it and no
//! further. A multi-state object's write refuses an Alarm_Values entry past
//! Number_Of_States (#1429), but a saved list naming one is installed as it
//! was written: the state count and State_Text aren't kept, so a peer that
//! added a state and listed it leaves such a file, and an object must still
//! open on it. The object then holds what a local shrink of its states
//! leaves it holding. A Multi-state Value shows it as CONFIGURATION_ERROR
//! and stays in FAULT, with no OFFNORMAL on any listed state, until the
//! list or the count is repaired; a Multi-state Input's other entries go on
//! alarming.

use bacnet_types::enums::{BinaryPV, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::common;
use crate::durable::PendingWrite;
use crate::event::{ChangeOfStateDetector, CommandFailureDetector, OutOfRangeDetector};
use crate::snapshot::write_config::parameters as format;
use crate::write_config::AlgorithmWriteConfig;

/// A detector whose parameters a write-config file keeps.
pub(crate) trait AlgorithmDetector: Default {
    /// Every parameter as the detector holds it now. Time_Delay_Normal is
    /// `None` until something sets it.
    fn parameters(&self) -> AlgorithmWriteConfig;

    /// Take the parameters `saved` holds, as a write of each would: the
    /// value goes into the detector and nothing else changes. A delay
    /// already counting keeps its count, and Event_State stays; the next
    /// evaluation uses the new values.
    fn restore_parameters(&mut self, saved: &AlgorithmWriteConfig);

    /// Make `write`, of a parameter the objects holding this kind of
    /// detector keep, as they make it: by the code their `write_property`
    /// runs. `states` is the object's Number_Of_States, if it has states.
    fn write_parameter(&mut self, write: &PendingWrite, states: Option<u32>) -> Result<(), Error>;
}

/// A detector where the event-row macros look for one: in the
/// `event_detector` of what they write.
struct Holder<'a, D> {
    event_detector: &'a mut D,
}

/// Write one of the two time delays on `$detector`, as every object does:
/// through the generic event-row macro.
macro_rules! write_delay {
    ($detector:expr, $write:expr) => {{
        let holder = Holder {
            event_detector: $detector,
        };
        common::write_generic_event_properties!(holder, $write.property, $write.value.clone())
            .unwrap_or_else(|| Err(common::unknown_property_error()))
    }};
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
            ..Default::default()
        }
    }

    fn restore_parameters(&mut self, saved: &AlgorithmWriteConfig) {
        restore_delays(&mut self.time_delay, &mut self.time_delay_normal, saved);
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

    fn write_parameter(&mut self, write: &PendingWrite, _: Option<u32>) -> Result<(), Error> {
        let holder = Holder {
            event_detector: self,
        };
        let (property, value) = (write.property, write.value.clone());
        common::write_analog_event_properties!(holder, property, value)
            .or_else(|| common::write_generic_event_properties!(holder, property, value))
            .unwrap_or_else(|| Err(common::unknown_property_error()))
    }
}

/// The binary and multi-state Input and Value objects' detector, and an
/// Access Door's and Access Zone's. Its one list serves a binary object's
/// Alarm_Value, as a list of that value, and the others' Alarm_Values;
/// storage keeps whichever the object's type has.
impl AlgorithmDetector for ChangeOfStateDetector {
    fn parameters(&self) -> AlgorithmWriteConfig {
        AlgorithmWriteConfig {
            time_delay: Some(self.time_delay),
            time_delay_normal: self.time_delay_normal,
            alarm_value: self.alarm_values.first().copied().map(BinaryPV::from_raw),
            alarm_values: Some(self.alarm_values.clone()),
            ..Default::default()
        }
    }

    fn restore_parameters(&mut self, saved: &AlgorithmWriteConfig) {
        restore_delays(&mut self.time_delay, &mut self.time_delay_normal, saved);
        if let Some(value) = saved.alarm_value {
            self.alarm_values = vec![value.to_raw()];
        }
        // Last: beside an Alarm_Value read off the same list, the list is
        // the whole of it.
        if let Some(values) = &saved.alarm_values {
            self.alarm_values.clone_from(values);
        }
    }

    fn write_parameter(&mut self, write: &PendingWrite, states: Option<u32>) -> Result<(), Error> {
        match (write.property, states) {
            (P::ALARM_VALUE, _) => crate::binary::write_alarm_value(self, write.value.clone()),
            (P::ALARM_VALUES, Some(states)) => crate::multistate::write_alarm_values(
                self,
                write.array_index,
                write.value.clone(),
                states,
            ),
            _ => write_delay!(self, write),
        }
    }
}

/// The Binary and Multi-state Output's detector: the two time delays are
/// all a peer writes of it.
impl AlgorithmDetector for CommandFailureDetector {
    fn parameters(&self) -> AlgorithmWriteConfig {
        AlgorithmWriteConfig {
            time_delay: Some(self.time_delay),
            time_delay_normal: self.time_delay_normal,
            ..Default::default()
        }
    }

    fn restore_parameters(&mut self, saved: &AlgorithmWriteConfig) {
        restore_delays(&mut self.time_delay, &mut self.time_delay_normal, saved);
    }

    fn write_parameter(&mut self, write: &PendingWrite, _: Option<u32>) -> Result<(), Error> {
        write_delay!(self, write)
    }
}

/// Take the time delays `saved` holds.
fn restore_delays(delay: &mut u32, delay_normal: &mut Option<u32>, saved: &AlgorithmWriteConfig) {
    if let Some(saved) = saved.time_delay {
        *delay = saved;
    }
    if saved.time_delay_normal.is_some() {
        *delay_normal = saved.time_delay_normal;
    }
}

/// The mask bit of `property`, if objects of `object_type` keep it.
pub(super) fn kept_bit(object_type: ObjectType, property: P) -> Option<u32> {
    format::bit_of(property).filter(|bit| format::kept(object_type) & bit != 0)
}

/// What `write`, of a kept parameter, leaves of `rows`, or the object's
/// refusal of it. The write is made on a fresh detector of the object's own
/// kind, `D`, that holds `rows`; `states` is the object's Number_Of_States,
/// if it has states.
pub(crate) fn after<D: AlgorithmDetector>(
    _kind: &D,
    states: Option<u32>,
    rows: &mut AlgorithmWriteConfig,
    write: &PendingWrite,
) -> Result<(), Error> {
    let mut scratch = D::default();
    scratch.restore_parameters(rows);
    scratch.write_parameter(write, states)?;
    format::overlay(rows, &scratch.parameters());
    Ok(())
}

/// The Number_Of_States `write` leaves an object that has `states` of them,
/// if the object takes it: State_Text written whole has a state for each
/// label, and its size written at index 0 is the new count (#1443). Any
/// other write leaves the count as it is. A stage follows the count through
/// a request with this, so a list naming a state the request itself adds is
/// checked against the count it will meet. A State_Text write the object
/// refuses ends the request, so what was staged past it is released.
pub(super) fn states_after(states: Option<u32>, write: &PendingWrite) -> Option<u32> {
    if write.property != P::STATE_TEXT {
        return states;
    }
    let count = match (write.array_index, &write.value) {
        (None, PropertyValue::List(labels)) => u32::try_from(labels.len()).ok(),
        (None, PropertyValue::CharacterString(_)) => Some(1),
        (Some(0), PropertyValue::Unsigned(count)) => u32::try_from(*count).ok(),
        _ => None,
    };
    states.map(|states| count.unwrap_or(states))
}
