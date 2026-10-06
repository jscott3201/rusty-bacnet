//! Access Control objects (ASHRAE 135-2020 Clause 12).
//!
//! This module implements the seven BACnet access control object types:
//! - AccessDoor (type 30)
//! - AccessCredential (type 32)
//! - AccessPoint (type 33)
//! - AccessRights (type 34)
//! - AccessUser (type 35)
//! - AccessZone (type 36)
//! - CredentialDataInput (type 37)
//!
//! [`evaluate_access_rights`] checks a credential against the Access Rights
//! it is assigned, for the application to act on (#1331).

use bacnet_encoding::primitives::encode_timestamp_choice;
use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::{
    AccessEvent, AccessUserType, AuthenticationStatus, BinaryPV, DoorAlarmState, DoorSecuredStatus,
    DoorStatus, DoorValue, EventState, LockStatus, ObjectType, PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{
    BACnetTimeStamp, Date, ObjectIdentifier, PropertyValue, StatusFlags, Time,
};
use bytes::BytesMut;
use std::borrow::Cow;
use std::sync::Arc;

use crate::clock::{current_datetime, ClockReader};
use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// The time stamp an Access Point's Access_Event_Time and a Credential Data
/// Input's Update_Time hold before their first update: the date-and-time form
/// with every octet unspecified (Clauses 12.31.29 and 12.36.11).
fn never_updated() -> BACnetTimeStamp {
    BACnetTimeStamp::DateTime {
        date: Date {
            year: Date::UNSPECIFIED,
            month: Date::UNSPECIFIED,
            day: Date::UNSPECIFIED,
            day_of_week: Date::UNSPECIFIED,
        },
        time: Time {
            hour: Time::UNSPECIFIED,
            minute: Time::UNSPECIFIED,
            second: Time::UNSPECIFIED,
            hundredths: Time::UNSPECIFIED,
        },
    }
}

/// A `BACnetTimeStamp` property in its Clause 21 CHOICE form.
fn timestamp_value(stamp: &BACnetTimeStamp) -> Result<PropertyValue, Error> {
    let mut buf = BytesMut::new();
    encode_timestamp_choice(&mut buf, stamp)?;
    Ok(PropertyValue::ApplicationData(buf.to_vec()))
}

/// The BACnetTimeStamp (Clause 21.6) for an update recorded now: the Device
/// clock's date and time, or `sequence()` in the sequence-number form when
/// there is no usable clock.
///
/// Without a clock every date-and-time stamp would be the unspecified one,
/// so successive updates would carry the same time and a subscriber watching
/// that time (Table 13-1) would hear none of them. A sequence number moves on
/// with each update instead; Clauses 12.31.29 and 12.36.11 both allow an
/// update time in that form.
fn update_stamp(
    clock: Option<&dyn ClockReader>,
    sequence: impl FnOnce() -> u16,
) -> BACnetTimeStamp {
    match current_datetime(clock) {
        Some((date, time)) => BACnetTimeStamp::DateTime { date, time },
        None => BACnetTimeStamp::SequenceNumber(sequence()),
    }
}

/// The sequence number after `previous`: it climbs from 1 to 65535, the top
/// of the production's range, and starts again at 1. It never takes 0,
/// which marks an update time with no update yet (Clauses 12.31.29 and
/// 12.36.11).
fn next_sequence(previous: u16) -> u16 {
    if previous == u16::MAX {
        1
    } else {
        previous + 1
    }
}

/// The Device clock an object stamps its update times from, and the
/// sequence numbers it hands out, in order, while that clock gives no usable
/// reading (`update_stamp`).
#[derive(Default)]
struct UpdateClock {
    clock: Option<Arc<dyn ClockReader>>,
    /// The last sequence number handed out; 0 before the first.
    sequence: u16,
}

impl UpdateClock {
    /// Stamp an update made now.
    fn stamp(&mut self) -> BACnetTimeStamp {
        let sequence = &mut self.sequence;
        update_stamp(self.clock.as_deref(), || {
            *sequence = next_sequence(*sequence);
            *sequence
        })
    }
}

/// A client's simulated Reliability, written while Out_Of_Service is TRUE:
/// an Enumerated inside the BACnetReliability production. Another datatype is
/// INVALID_DATA_TYPE and another number VALUE_OUT_OF_RANGE.
fn simulated_reliability(value: &PropertyValue) -> Result<Reliability, Error> {
    let PropertyValue::Enumerated(raw) = value else {
        return Err(common::invalid_data_type_error());
    };
    checked_reliability(Reliability::from_raw(*raw))
}

/// `reliability`, or VALUE_OUT_OF_RANGE outside the BACnetReliability
/// production.
fn checked_reliability(reliability: Reliability) -> Result<Reliability, Error> {
    if common::is_reliability_value_valid(reliability) {
        Ok(reliability)
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// The references a setter of one of the reference lists or arrays stores,
/// each naming an object of `object_type` in this device or in the Device its
/// device identifier names. A reference to another object type, or one whose
/// device identifier isn't a Device (the shared `check_device_member`), is
/// VALUE_OUT_OF_RANGE, so the setter keeps what it held.
fn references_to(
    object_type: ObjectType,
    references: impl IntoIterator<Item = impl Into<BACnetDeviceObjectReference>>,
) -> Result<Vec<BACnetDeviceObjectReference>, Error> {
    let references: Vec<BACnetDeviceObjectReference> =
        references.into_iter().map(Into::into).collect();
    for reference in &references {
        crate::device_reference::check_device_member(reference.device_identifier)?;
        if reference.object_identifier.object_type() != object_type {
            return Err(common::value_out_of_range_error());
        }
    }
    Ok(references)
}

// ---------------------------------------------------------------------------

mod credential;
mod credential_data_input;
mod credential_data_input_formats;
mod credential_data_input_out_of_service;
mod credential_rules;
mod door;
mod door_alarm;
mod door_out_of_service;
mod input;
mod metadata_identity;
mod metadata_topology;
mod point;
mod point_authorization;
mod point_event_time;
mod rights;
mod rights_evaluation;
mod rights_writes;
mod user;
mod zone;
mod zone_occupancy;
mod zone_out_of_service;
pub use credential::*;
pub use credential_data_input::*;
pub use door::*;
pub use input::*;
pub use point::*;
pub use rights::*;
pub use rights_evaluation::*;
pub use user::*;
pub use zone::*;

#[cfg(test)]
mod array_tests;
#[cfg(test)]
mod constructed_value_tests;
#[cfg(test)]
mod credential_data_input_format_tests;
#[cfg(test)]
mod credential_data_input_out_of_service_tests;
#[cfg(test)]
mod credential_exemption_tests;
#[cfg(test)]
mod credential_tests;
#[cfg(test)]
mod device_reference_tests;
#[cfg(test)]
mod door_alarm_tests;
#[cfg(test)]
mod door_out_of_service_tests;
#[cfg(test)]
mod door_pulse_tests;
#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod point_authorization_tests;
#[cfg(test)]
mod point_out_of_service_tests;
#[cfg(test)]
mod point_policy_tests;
#[cfg(test)]
mod point_status_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod typed_value_tests;
#[cfg(test)]
mod user_references_tests;
#[cfg(test)]
mod zone_event_tests;
#[cfg(test)]
mod zone_occupancy_tests;
#[cfg(test)]
mod zone_out_of_service_tests;
#[cfg(test)]
mod zone_points_tests;
