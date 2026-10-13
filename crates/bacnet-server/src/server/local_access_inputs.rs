//! The application's route to the inputs of the access-control objects the
//! server holds (#1132): an Access Point's access events, a Credential Data
//! Input's reads and an Access Door's hardware state.
//!
//! Each takes a typed record whose values change together, as one local
//! write through `write_local_as`: the object takes the record under the
//! database guard, then the event pass and the COV fanout run as they do
//! after any other local write, and the Tokio-runtime rule of #1367 holds.
//! The records and their rules are in `bacnet_objects::access_control`.

use super::*;
use bacnet_objects::access_control::{
    AccessControlInput, AccessEventReport, CredentialReadReport, DoorStateReport,
};

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Report an access event at an Access Point: Access_Event,
    /// Access_Event_Tag, Access_Event_Time, Access_Event_Credential and
    /// Access_Event_Authentication_Factor change together (Clause
    /// 12.31.27.1), checked as `AccessPointObject::set_access_event` checks
    /// them (VALUE_OUT_OF_RANGE for a credential reference, factor or event
    /// it refuses; an event is a named BACnetAccessEvent or a proprietary
    /// one from 512 to 65535). A report without a time is stamped from the
    /// Device clock.
    ///
    /// Access_Event_Time is the point's Table 13-1 trigger, so each event
    /// with a new time sends the SubscribeCOV report, carrying all five
    /// values. A time the point stamps itself is strictly after the one it
    /// served, so several events of one transaction each move the time and
    /// each report. While Out_Of_Service is TRUE (Clause 12.31.8), or
    /// Reliability isn't NO_FAULT_DETECTED (Clause 12.31.7), the point
    /// performs no authentication, so an event is refused with PROPERTY /
    /// WRITE_ACCESS_DENIED and nothing changes. An unknown object fails
    /// with OBJECT / UNKNOWN_OBJECT and any object other than an Access
    /// Point with OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. The Python
    /// binding exposes this as `BACnetServer.report_access_event_local`.
    ///
    /// Like [`write_local`](Self::write_local), it must be awaited inside a
    /// Tokio runtime, failing before anything is written outside one, and a
    /// caller dropped once the event has committed skips none of the work
    /// it owes (#1367).
    pub async fn report_access_event_local(
        &self,
        oid: &ObjectIdentifier,
        report: AccessEventReport,
    ) -> Result<(), Error> {
        self.report_access_input(oid, AccessControlInput::AccessEvent(report))
            .await
    }

    /// Report a factor a Credential Data Input's reader has read:
    /// Present_Value and Update_Time change together (Clause 12.36.11). The
    /// factor must name one of the reader's declared formats with its class,
    /// or be the UNDEFINED or ERROR factor with class 0, else
    /// VALUE_OUT_OF_RANGE. A report without a time is stamped from the
    /// Device clock, or the object's next sequence number without a usable
    /// clock, so reading the same card again still moves Update_Time.
    ///
    /// Update_Time is the reader's Table 13-1 trigger, so each read with a
    /// new time sends the SubscribeCOV report. While Out_Of_Service is TRUE
    /// a client may be simulating the reader, so the read replaces the
    /// reader's values put aside, as `set_present_value` does (#1168): the
    /// served values, and with them the COV report, stay as they are, and
    /// the return to service serves the latest read. The other errors are
    /// those of [`Self::report_access_event_local`], for a Credential Data
    /// Input. The Python binding exposes this as
    /// `BACnetServer.report_credential_read_local`.
    ///
    /// The Tokio-runtime rule of [`write_local`](Self::write_local) applies.
    pub async fn report_credential_read_local(
        &self,
        oid: &ObjectIdentifier,
        report: CredentialReadReport,
    ) -> Result<(), Error> {
        self.report_access_input(oid, AccessControlInput::CredentialRead(report))
            .await
    }

    /// Report an Access Door's hardware state: whichever of Door_Status,
    /// Lock_Status and Door_Alarm_State the report gives, together. Each
    /// must fall in its production, and the alarm state be NORMAL or a
    /// member of Alarm_Values or Fault_Values and no member of
    /// Masked_Alarm_Values, else VALUE_OUT_OF_RANGE and nothing changes.
    ///
    /// Door_Alarm_State is the door's Table 13-1 trigger, so a change of it
    /// sends the SubscribeCOV report, and the event pass after the write
    /// runs the door's CHANGE_OF_STATE and FAULT_STATE algorithms on it at
    /// once. Secured_Status follows the new values on its next read. While
    /// Out_Of_Service is TRUE a client may be simulating the door, so the
    /// report replaces the device values put aside, as the door's setters do
    /// (#1131): the served values stay, so no COV report or event follows,
    /// and the return to service serves the latest values reported. The
    /// other errors are those of
    /// [`Self::report_access_event_local`], for an Access Door. The Python
    /// binding exposes this as `BACnetServer.report_door_state_local`.
    ///
    /// The Tokio-runtime rule of [`write_local`](Self::write_local) applies.
    pub async fn report_door_state_local(
        &self,
        oid: &ObjectIdentifier,
        report: DoorStateReport,
    ) -> Result<(), Error> {
        self.report_access_input(oid, AccessControlInput::DoorState(report))
            .await
    }

    /// One access-control input, as a local write.
    async fn report_access_input(
        &self,
        oid: &ObjectIdentifier,
        input: AccessControlInput,
    ) -> Result<(), Error> {
        self.write_local_as(
            oid,
            LocalWrite::ApplicationAccessInput(&input),
            PropertyValue::Null,
            None,
        )
        .await
    }
}
