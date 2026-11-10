//! An Event Enrollment's side of the event algorithm parameters it keeps
//! beside its reporting settings (#2331): Event_Parameters, Fault_Parameters
//! and Time_Delay_Normal. Compiled with the `event-reporting` feature, as
//! the store is.
//!
//! The enrollment has no detector of the intrinsic kind
//! (`AlgorithmDetector`): the server's pass reads the parameters off the
//! object each time it runs. So the three rows are lent, restored and
//! checked here, by the functions the object's own writes go through, and
//! `write_config::parameter_table_tests` walks the format's table against
//! them.
//!
//! Time_Delay is no row. On this type it is a member of Event_Parameters
//! (Table 12-15), kept with them, and Time_Delay_Normal is the object's
//! own property (Clause 12.12.19), so neither delay is kept twice.
//!
//! A write of any of the three is checked against the written value alone:
//! Event_Type follows Event_Parameters and Fault_Type follows
//! Fault_Parameters (Clauses 12.12.5 and 12.12.22), and whether the
//! algorithm suits the property Object_Property_Reference names is found by
//! the evaluation, not by the write. So a saved value is one the rebuilt
//! enrollment takes on the wire too, and none is restored unchecked. For
//! the same reason no write earlier in a request changes what a later one
//! of these is checked against, and a request's stage follows nothing
//! through it.

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::EventEnrollmentObject;
use super::parameters::{self, MAX_KEPT_OCTETS};
use crate::common;
use crate::durable::PendingWrite;
use crate::write_config::AlgorithmWriteConfig;

impl EventEnrollmentObject {
    /// The three parameters as the enrollment holds them now.
    /// Fault_Parameters unset is the `none` choice, as it is served;
    /// Time_Delay_Normal is `None` until something sets it.
    pub(super) fn lent_parameters(&self) -> AlgorithmWriteConfig {
        AlgorithmWriteConfig {
            time_delay_normal: self.time_delay_normal,
            event_parameters: Some(self.event_parameters.clone()),
            fault_parameters: Some(self.effective_fault_parameters().clone()),
            ..Default::default()
        }
    }

    /// Take the parameters `saved` holds, as a write of each does: the
    /// value goes where the write puts it, and a delay counting under the
    /// old Event_Parameters or Time_Delay_Normal is dropped. Event_State,
    /// Reliability and the algorithms' baselines stay; the server's next
    /// pass evaluates with the new values.
    pub(super) fn restore_parameters(&mut self, saved: &AlgorithmWriteConfig) {
        if let Some(parameters) = &saved.event_parameters {
            self.install_event_parameters(parameters.clone());
        }
        if let Some(parameters) = &saved.fault_parameters {
            self.install_fault_parameters(parameters.clone());
        }
        if saved.time_delay_normal.is_some() {
            self.install_time_delay_normal(saved.time_delay_normal);
        }
    }
}

/// What `write`, of one of the three kept parameters, leaves of `rows`, or
/// the enrollment's refusal of it, by the decoders its `write_property`
/// runs. Event_Parameters or Fault_Parameters past what storage keeps
/// ([`MAX_KEPT_OCTETS`]) are refused with RESOURCES /
/// NO_SPACE_TO_WRITE_PROPERTY, as a message text past its cap is: only an
/// enrollment with storage is asked.
pub(super) fn parameter_after(
    rows: &mut AlgorithmWriteConfig,
    write: &PendingWrite,
) -> Result<(), Error> {
    match write.property {
        P::EVENT_PARAMETERS => {
            let value = parameters::decode_event_parameters(write.value.clone())?;
            fits(parameters::event_parameter_octets(&value))?;
            rows.event_parameters = Some(value);
        }
        P::FAULT_PARAMETERS => {
            let value = parameters::decode_fault_parameters(write.value.clone())?;
            fits(parameters::fault_parameter_octets(&value))?;
            rows.fault_parameters = Some(value);
        }
        P::TIME_DELAY_NORMAL => {
            let delay = parameters::written_time_delay_normal(&write.value)?;
            rows.time_delay_normal = Some(delay);
        }
        _ => return Err(common::unknown_property_error()),
    }
    Ok(())
}

/// Refuse a value whose encoding storage doesn't keep. The decoders have
/// encoded it once already, so it has an encoding.
fn fits(octets: Result<BytesMut, Error>) -> Result<(), Error> {
    match octets {
        Ok(octets) if octets.len() <= MAX_KEPT_OCTETS => Ok(()),
        _ => Err(common::protocol_error(
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        )),
    }
}
