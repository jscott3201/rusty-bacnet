//! Door_Status, Lock_Status, Door_Alarm_State and Reliability while
//! Out_Of_Service is TRUE (Clauses 12.26.8, 12.26.9, 12.26.12, 12.26.13 and
//! 12.26.20, Table 12-30 footnote 1, #1131, #1149).
//!
//! Footnote 1 of Table 12-30 marks the first three rows, so a client can
//! simulate the door by writing them. Clause 12.26.9 adds Reliability for a
//! door whose Reliability can leave NO_FAULT_DETECTED, and this one can: the
//! FAULT_STATE check on Fault_Values (`door_alarm`, #1149) moves it to
//! MULTI_STATE_FAULT. The door takes WriteProperty and WritePropertyMultiple
//! of the four only while Out_Of_Service is TRUE and refuses them with
//! WRITE_ACCESS_DENIED in service. Each write has to be an Enumerated inside
//! its Clause 21 production: a named BACnetDoorStatus or one from the
//! proprietary range 1024..=65535, a named BACnetLockStatus (the production
//! is closed and Table 23-1 doesn't list it, so it has no proprietary range),
//! a named BACnetDoorAlarmState or one from 256..=65535, or a
//! BACnetReliability, its proprietary range included. Any other number is
//! VALUE_OUT_OF_RANGE and any other datatype INVALID_DATA_TYPE, with the
//! property left as it was.
//!
//! A simulated Door_Alarm_State also has to be one the door may hold (Clause
//! 12.26.20): NORMAL or a member of Alarm_Values or Fault_Values, and no
//! member of Masked_Alarm_Values. Any other state is VALUE_OUT_OF_RANGE, as
//! it is from the application.
//!
//! Out of service these values stop following the device, so the door keeps
//! the device's own values to one side:
//!
//! - On the FALSE-to-TRUE edge it puts aside the values it serves
//!   (`common::write_out_of_service_with_restore`). The device has no
//!   Reliability of its own to put aside: in service the FAULT_STATE check
//!   decides it.
//! - Meanwhile a value the application reports (`set_door_status`,
//!   `set_lock_status`, `set_door_alarm_state`) replaces the value put aside,
//!   not the one served.
//! - On the TRUE-to-FALSE edge the values put aside come back, so the
//!   device's state is served again at once and the simulated values, a
//!   simulated Reliability included, are gone.
//!
//! How a simulated value reaches the rest of the door and the server:
//!
//! - COV: Door_Alarm_State is a trigger of the Access Door row of Table 13-1
//!   (#1061), and the server compares the row's values around every committed
//!   write, so a simulated Door_Alarm_State sends a SubscribeCOV report, as the
//!   return to service does when it restores a different value. Door_Status
//!   and Lock_Status aren't in that row and send none by themselves. A
//!   simulated fault reports through the FAULT flag of Status_Flags.
//! - Pulse relock: Door_Pulse_Time and Door_Extended_Pulse_Time relinquish a
//!   pulse from the priority array on a timer armed by the command (#1073).
//!   Nothing reads Door_Status, Lock_Status or Door_Alarm_State to relinquish
//!   early or to hold the slot, so a simulated value neither drives nor delays
//!   the relock, and the relock leaves the simulated values alone.
//! - Event reporting: the CHANGE_OF_STATE algorithm watches the
//!   Door_Alarm_State and Reliability served (#1149), and while no
//!   Reliability is simulated the FAULT_STATE check reads the
//!   Door_Alarm_State served. A simulated alarm state therefore raises an
//!   offnormal event, and a simulated fault value or Reliability a fault one,
//!   the way Clause 12.26.9 has anything depending on these values follow
//!   the simulation.
//! - Secured_Status: each read derives it from the served Door_Status and
//!   Lock_Status, among other inputs (`AccessDoorObject::secured_status`,
//!   #1148). A simulated value moves it as a device report would, and the
//!   return to service moves it back with the device's values.

use bacnet_types::enums::{
    DoorAlarmState, DoorStatus, LockStatus, PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::{simulated_reliability, DoorStateReport};
use crate::common;

/// The door state a client can simulate while Out_Of_Service is TRUE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DoorState {
    pub(super) door_status: DoorStatus,
    pub(super) lock_status: LockStatus,
    pub(super) door_alarm_state: DoorAlarmState,
    /// A client's simulated Reliability; `None` leaves Reliability to the
    /// FAULT_STATE check, as the device's own state always does.
    pub(super) reliability: Option<Reliability>,
}

impl DoorState {
    /// A closed, locked door with no alarm.
    pub(super) const SECURE: Self = Self {
        door_status: DoorStatus::CLOSED,
        lock_status: LockStatus::LOCKED,
        door_alarm_state: DoorAlarmState::NORMAL,
        reliability: None,
    };

    /// Apply a client's write of Door_Status, Lock_Status, Door_Alarm_State
    /// or Reliability, taken only while `out_of_service`; `None` for any
    /// other property. `admits` says which Door_Alarm_State values the
    /// door's lists allow.
    pub(super) fn write(
        &mut self,
        out_of_service: bool,
        admits: impl Fn(DoorAlarmState) -> bool,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        const SIMULATED: [PropertyIdentifier; 4] = [
            PropertyIdentifier::DOOR_STATUS,
            PropertyIdentifier::LOCK_STATUS,
            PropertyIdentifier::DOOR_ALARM_STATE,
            PropertyIdentifier::RELIABILITY,
        ];
        if !SIMULATED.contains(&property) {
            return None;
        }
        if !out_of_service {
            return Some(Err(common::write_access_denied_error()));
        }
        if property == PropertyIdentifier::RELIABILITY {
            return Some(simulated_reliability(value).map(|reliability| {
                self.reliability = Some(reliability);
            }));
        }
        let PropertyValue::Enumerated(raw) = *value else {
            return Some(Err(common::invalid_data_type_error()));
        };
        Some(match property {
            p if p == PropertyIdentifier::DOOR_STATUS => {
                checked(DoorStatus::from_raw(raw), door_status_in_range)
                    .map(|status| self.door_status = status)
            }
            p if p == PropertyIdentifier::LOCK_STATUS => {
                checked(LockStatus::from_raw(raw), lock_status_in_range)
                    .map(|status| self.lock_status = status)
            }
            _ => checked(DoorAlarmState::from_raw(raw), door_alarm_state_in_range)
                .and_then(|state| checked(state, &admits))
                .map(|state| self.door_alarm_state = state),
        })
    }
}

impl DoorState {
    /// Apply a report from the door's hardware (#1132): each value given,
    /// checked against its production as a client's simulated write is, and
    /// the alarm state against `admits`. Any value refused is
    /// VALUE_OUT_OF_RANGE and none of them changes.
    pub(super) fn report(
        &mut self,
        report: DoorStateReport,
        admits: impl Fn(DoorAlarmState) -> bool,
    ) -> Result<(), Error> {
        let door_status = report
            .door_status
            .map(|status| checked(status, door_status_in_range))
            .transpose()?;
        let lock_status = report
            .lock_status
            .map(|status| checked(status, lock_status_in_range))
            .transpose()?;
        let door_alarm_state = report
            .door_alarm_state
            .map(|state| {
                checked(state, door_alarm_state_in_range).and_then(|state| checked(state, &admits))
            })
            .transpose()?;
        self.door_status = door_status.unwrap_or(self.door_status);
        self.lock_status = lock_status.unwrap_or(self.lock_status);
        self.door_alarm_state = door_alarm_state.unwrap_or(self.door_alarm_state);
        Ok(())
    }
}

/// `value`, or VALUE_OUT_OF_RANGE when `in_range` refuses it.
fn checked<T: Copy>(value: T, in_range: impl Fn(T) -> bool) -> Result<T, Error> {
    if in_range(value) {
        Ok(value)
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// Whether `status` is a named BACnetDoorStatus or a proprietary one
/// (1024..=65535, Clause 23.1).
fn door_status_in_range(status: DoorStatus) -> bool {
    DoorStatus::ALL_NAMED
        .iter()
        .any(|&(_, named)| named == status)
        || (1024..=65_535).contains(&status.to_raw())
}

/// Whether `status` is a named BACnetLockStatus, a closed production.
fn lock_status_in_range(status: LockStatus) -> bool {
    LockStatus::ALL_NAMED
        .iter()
        .any(|&(_, named)| named == status)
}

/// Whether `state` is a named BACnetDoorAlarmState or a proprietary one
/// (256..=65535, Clause 23.1).
pub(super) fn door_alarm_state_in_range(state: DoorAlarmState) -> bool {
    DoorAlarmState::ALL_NAMED
        .iter()
        .any(|&(_, named)| named == state)
        || (256..=65_535).contains(&state.to_raw())
}
