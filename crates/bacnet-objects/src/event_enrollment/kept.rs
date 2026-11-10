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
//!
//! A value a peer writes arrives in the framed encoding and has been
//! through the framed decoder. An in-process caller can hand
//! `write_property` the flat list form instead, which is only checked to
//! have an encoding, and some such encodings the framed decoder refuses (a
//! bit string claiming more than seven unused bits). An enrollment with
//! storage holds every form to what a load will hold its row to
//! ([`keeps`]), so nothing is saved that wouldn't reopen; one without
//! storage takes the flat form as it always has.

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::EventEnrollmentObject;
use super::parameters::{self, MAX_KEPT_OCTETS, NotKept};
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
/// runs. Only an enrollment with storage is asked, so Event_Parameters and
/// Fault_Parameters are also held to what storage hands back ([`keeps`]).
pub(super) fn parameter_after(
    rows: &mut AlgorithmWriteConfig,
    write: &PendingWrite,
) -> Result<(), Error> {
    match write.property {
        P::EVENT_PARAMETERS => {
            let value = parameters::decode_event_parameters(write.value.clone())?;
            keeps(
                &value,
                parameters::event_parameter_octets,
                parameters::framed_event_parameters,
            )?;
            rows.event_parameters = Some(value);
        }
        P::FAULT_PARAMETERS => {
            let value = parameters::decode_fault_parameters(write.value.clone())?;
            keeps(
                &value,
                parameters::fault_parameter_octets,
                parameters::framed_fault_parameters,
            )?;
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

/// Refuse a value storage wouldn't hand back: one whose encoding doesn't
/// read back by the rule a load holds a file's row to
/// ([`parameters::reloaded`]). It is refused as the wire refuses octets that
/// are no value of the property (INVALID_DATA_TYPE, or the decoder's own
/// refusal). One whose encoding is past [`MAX_KEPT_OCTETS`] is refused with
/// RESOURCES / NO_SPACE_TO_WRITE_PROPERTY, as a message text past its cap
/// is.
fn keeps<T>(
    value: &T,
    octets_of: fn(&T) -> Result<BytesMut, Error>,
    framed: fn(&[u8]) -> Result<T, Error>,
) -> Result<(), Error> {
    // The write's decoder has encoded the value once, so it has an
    // encoding.
    let octets = octets_of(value).map_err(|_| common::invalid_data_type_error())?;
    parameters::reloaded(&octets, framed, octets_of).map_err(|why| match why {
        NotKept::Undecodable(refusal) => refusal,
        NotKept::Recoded => common::invalid_data_type_error(),
    })?;
    if octets.len() > MAX_KEPT_OCTETS {
        return Err(common::protocol_error(
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        ));
    }
    Ok(())
}
