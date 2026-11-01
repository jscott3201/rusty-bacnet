//! Occupancy_Count and Reliability of an Access Zone while Out_Of_Service is
//! TRUE (Clauses 12.32.9, 12.32.10 and 12.32.11, Table 12-37 footnote 1,
//! #1247).
//!
//! Footnote 1 of Table 12-37 marks these two rows, so a client can stand in
//! for the zone's counting and fault detection by writing them. The zone takes
//! WriteProperty and WritePropertyMultiple of either one only while
//! Out_Of_Service is TRUE; in service both are refused with
//! WRITE_ACCESS_DENIED, before the datatype is looked at. Out of service:
//!
//! - an Occupancy_Count write has to be an Unsigned, any value while
//!   counting is on and only zero while it is off (VALUE_OUT_OF_RANGE
//!   otherwise; see `zone_occupancy`); another datatype is INVALID_DATA_TYPE;
//! - a Reliability write has to be an Enumerated inside the BACnetReliability
//!   production, its proprietary range included; another number is
//!   VALUE_OUT_OF_RANGE and another datatype INVALID_DATA_TYPE.
//!
//! A refused write changes nothing.
//!
//! Out of service the two values stop following the zone's own process, so
//! the zone keeps that process's values to one side:
//!
//! - On the FALSE-to-TRUE edge it puts aside the Occupancy_Count and
//!   Reliability it serves (`common::write_out_of_service_with_restore`).
//! - Meanwhile a count the application reports (`set_occupancy_count`)
//!   replaces the count put aside, not the one served, and a Reliability it
//!   works out (`ReliabilityEval::set_reliability`) is refused, as on the other
//!   Reliability carriers.
//! - On the TRUE-to-FALSE edge both go back to the values put aside, so the
//!   zone's own state is served again at once and the simulation is gone.
//!   Clause 12.32.11 leaves the count after the return to service to the
//!   implementation; this is the choice made here.
//!
//! How a simulated value reaches the rest of the device: Status_Flags reads
//! FAULT from the Reliability served, so a simulated fault sets it, and
//! Occupancy_State follows the count served, so a simulated count moves it
//! (12.32.10). An Adjust_Value write is kept but leaves the simulated count
//! alone. The CHANGE_OF_STATE algorithm watches the Occupancy_State and
//! Reliability served (#1305), so a simulated count can raise an offnormal
//! event and a simulated fault a FAULT one, as item (f) of 12.32.10 asks. The
//! zone has no Table 13-1 row, so neither sends a SubscribeCOV report.
//! Anything reading these properties sees the simulation as it would see the
//! zone.

use bacnet_types::enums::{PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::simulated_reliability;
use crate::common;

/// The values a client can simulate on an Access Zone while Out_Of_Service is
/// TRUE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ZoneState {
    pub(super) occupancy_count: u64,
    pub(super) reliability: Reliability,
}

impl ZoneState {
    /// Apply a client's write of Occupancy_Count or Reliability, taken only
    /// while `out_of_service`; while counting is off (`counting` FALSE) the
    /// count takes only zero. `None` for any other property.
    pub(super) fn write(
        &mut self,
        out_of_service: bool,
        counting: bool,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        let simulated = property == PropertyIdentifier::OCCUPANCY_COUNT
            || property == PropertyIdentifier::RELIABILITY;
        if !simulated {
            return None;
        }
        if !out_of_service {
            return Some(Err(common::write_access_denied_error()));
        }
        if property == PropertyIdentifier::RELIABILITY {
            return Some(simulated_reliability(value).map(|reliability| {
                self.reliability = reliability;
            }));
        }
        let PropertyValue::Unsigned(count) = value else {
            return Some(Err(common::invalid_data_type_error()));
        };
        if !counting && *count != 0 {
            return Some(Err(common::value_out_of_range_error()));
        }
        self.occupancy_count = *count;
        Some(Ok(()))
    }
}
