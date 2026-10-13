//! The inputs an application reports for its access-control objects while
//! the server holds them (#1132).
//!
//! An Access Point's access events, a Credential Data Input's reads and an
//! Access Door's hardware state come from outside BACnet: the application's
//! access logic, its card readers, its door contacts and lock monitors. None
//! of them takes a network write in service. Each object takes its own
//! record through [`BACnetObject::report_access_input_internal`], whose
//! values change together. The server's `report_access_event_local`,
//! `report_credential_read_local` and `report_door_state_local` hand the
//! record over as one local write, so the COV report and the event pass
//! follow it as they follow any other.
//!
//! While Out_Of_Service is TRUE each object stands apart from what it
//! represents (Clauses 12.26.9, 12.31.8 and 12.36.8), and the route follows
//! the object's own rule for that time:
//!
//! - The Access Point performs no authentication or authorization, so an
//!   access event then has no meaning: the route refuses it with
//!   WRITE_ACCESS_DENIED and changes nothing. It does the same while its
//!   Reliability isn't NO_FAULT_DETECTED (Clause 12.31.7).
//! - An Access Door or Credential Data Input keeps the device's values to one
//!   side while a client may simulate the served ones (#1131, #1168). A
//!   report replaces the values put aside, not the ones served, so it sends
//!   no COV report and moves no event algorithm; the return to service
//!   serves the latest values reported. The objects' setters follow the
//!   same rule.
//!
//! [`BACnetObject::report_access_input_internal`]: crate::traits::BACnetObject::report_access_input_internal

use bacnet_types::constructed::{BACnetAuthenticationFactor, BACnetDeviceObjectReference};
use bacnet_types::enums::{AccessEvent, DoorAlarmState, DoorStatus, LockStatus};
use bacnet_types::primitives::BACnetTimeStamp;

/// An input for one access-control object, as the application reports it.
#[derive(Debug, Clone, PartialEq)]
pub enum AccessControlInput {
    /// A new access event, for an Access Point.
    AccessEvent(AccessEventReport),
    /// A factor a reader has read, for a Credential Data Input.
    CredentialRead(CredentialReadReport),
    /// What the door's hardware reports, for an Access Door.
    DoorState(DoorStateReport),
}

/// One access event at an Access Point: the values of Access_Event,
/// Access_Event_Tag, Access_Event_Time, Access_Event_Credential and
/// Access_Event_Authentication_Factor, which change together for each event
/// (Clause 12.31.27.1).
#[derive(Debug, Clone, PartialEq)]
pub struct AccessEventReport {
    /// The event, a standard BACnetAccessEvent or a proprietary one.
    pub event: AccessEvent,
    /// The access transaction the event belongs to. The application moves
    /// it on for each new transaction and repeats it for further events of
    /// the same one (Clause 12.31.28).
    pub tag: u64,
    /// When the event happened; `None` stamps it strictly after the time
    /// the point serves, from the Device clock or, without a usable clock,
    /// the next sequence number, as the point's Out_Of_Service edges are
    /// stamped, so each event moves Access_Event_Time.
    pub time: Option<BACnetTimeStamp>,
    /// The Access Credential behind the event; `None` stores the
    /// no-credential reference (Clause 12.31.30).
    pub credential: Option<BACnetDeviceObjectReference>,
    /// The factor presented; `None` stores the UNDEFINED factor, for an
    /// event no factor belongs to or one the device keeps back (Clause
    /// 12.31.31).
    pub authentication_factor: Option<BACnetAuthenticationFactor>,
}

impl AccessEventReport {
    /// `event` in transaction `tag`, stamped now, with no credential and no
    /// factor.
    pub fn new(event: AccessEvent, tag: u64) -> Self {
        Self {
            event,
            tag,
            time: None,
            credential: None,
            authentication_factor: None,
        }
    }
}

/// A factor a Credential Data Input's reader has read: its Present_Value and
/// Update_Time, which move together (Clause 12.36.11).
#[derive(Debug, Clone, PartialEq)]
pub struct CredentialReadReport {
    /// The factor read, in one of the reader's declared formats, or the
    /// UNDEFINED or ERROR factor.
    pub factor: BACnetAuthenticationFactor,
    /// When it was read; `None` stamps it from the Device clock, or the
    /// object's next sequence number without a usable clock.
    pub update_time: Option<BACnetTimeStamp>,
}

impl CredentialReadReport {
    /// `factor`, read now.
    pub fn new(factor: BACnetAuthenticationFactor) -> Self {
        Self {
            factor,
            update_time: None,
        }
    }
}

/// What an Access Door's hardware and door logic report: any of
/// Door_Status, Lock_Status and Door_Alarm_State. A field left `None` keeps
/// the value the door holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DoorStateReport {
    /// The open or closed state the door contact reports.
    pub door_status: Option<DoorStatus>,
    /// The state the lock monitor reports.
    pub lock_status: Option<LockStatus>,
    /// The alarm condition the application's door logic works out.
    pub door_alarm_state: Option<DoorAlarmState>,
}
