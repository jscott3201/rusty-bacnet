//! The application's route to the inputs of a running server's access-control
//! objects (#1132): an Access Point's access events, a Credential Data
//! Input's reads and an Access Door's hardware state. Each is one local
//! write, so COV reports and the event pass follow it. While Out_Of_Service
//! is TRUE the point refuses an event, and the door and the reader keep a
//! report aside for the return to service.
use super::super::*;
use crate::types::PyBACnetTimeStamp;
use bacnet_objects::access_control::{
    AccessControlInput, AccessEventReport, CredentialReadReport, DoorStateReport,
};
use bacnet_types::constructed::BACnetAuthenticationFactor;
use bacnet_types::enums::{
    AccessEvent, AuthenticationFactorType, DoorAlarmState, DoorStatus, LockStatus,
};

/// A BACnetAuthenticationFactor as Python gives it: `(format_type,
/// format_class, value)`, the value as `bytes`.
#[derive(FromPyObject)]
struct PyAuthenticationFactor(u32, u32, Vec<u8>);

impl From<PyAuthenticationFactor> for BACnetAuthenticationFactor {
    fn from(factor: PyAuthenticationFactor) -> Self {
        let PyAuthenticationFactor(format_type, format_class, value) = factor;
        Self {
            format_type: AuthenticationFactorType::from_raw(format_type),
            format_class,
            value,
        }
    }
}

impl BACnetServer {
    /// Hand `input` for `object_id` to the started server's route; each
    /// method below turns the future into the awaitable it returns.
    fn report_access_input(
        &self,
        object_id: PyObjectIdentifier,
        input: AccessControlInput,
    ) -> impl std::future::Future<Output = PyResult<()>> + Send + 'static {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            match input {
                AccessControlInput::AccessEvent(report) => {
                    srv.report_access_event_local(&oid, report).await
                }
                AccessControlInput::CredentialRead(report) => {
                    srv.report_credential_read_local(&oid, report).await
                }
                AccessControlInput::DoorState(report) => {
                    srv.report_door_state_local(&oid, report).await
                }
            }
            .map_err(to_py_err)
        }
    }
}

#[pymethods]
impl BACnetServer {
    /// Report an access event at an Access Point the server holds: its
    /// Access_Event, Access_Event_Tag, Access_Event_Time,
    /// Access_Event_Credential and Access_Event_Authentication_Factor change
    /// together.
    ///
    /// `event` is a BACnetAccessEvent number and `tag` the access
    /// transaction. `time` is a `BACnetTimeStamp`, the Device clock's when
    /// omitted. `credential` is the Access Credential behind the event, an
    /// `ObjectIdentifier` or a `(device, object)` pair, the no-credential
    /// reference when omitted. `authentication_factor` is `(format_type,
    /// format_class, value)`, the UNDEFINED factor when omitted. A
    /// credential that isn't an Access Credential, a factor format outside
    /// the closed production, or an event that is neither a named
    /// BACnetAccessEvent nor a proprietary one from 512 to 65535, raises
    /// VALUE_OUT_OF_RANGE. While the point is out of service, or its
    /// Reliability isn't NO_FAULT_DETECTED, the event is refused with
    /// WRITE_ACCESS_DENIED and nothing changes. Any object other than an
    /// Access Point raises OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. Each event
    /// moves Access_Event_Time, so each sends the point's COV report.
    #[pyo3(signature = (
        object_id,
        event,
        tag,
        *,
        time=None,
        credential=None,
        authentication_factor=None
    ))]
    fn report_access_event_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        event: u32,
        tag: u64,
        time: Option<PyBACnetTimeStamp>,
        credential: Option<Bound<'py, PyAny>>,
        authentication_factor: Option<PyAuthenticationFactor>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let credential = credential
            .map(|credential| crate::types::device_object_reference(&credential, "credential"))
            .transpose()?;
        let report = AccessEventReport {
            time: time.map(|time| time.to_rust().clone()),
            credential,
            authentication_factor: authentication_factor.map(Into::into),
            ..AccessEventReport::new(AccessEvent::from_raw(event), tag)
        };
        let future = self.report_access_input(object_id, AccessControlInput::AccessEvent(report));
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Report a factor a Credential Data Input's reader has read: its
    /// Present_Value and Update_Time change together.
    ///
    /// `factor` is `(format_type, format_class, value)` and must name one of
    /// the reader's `supported_formats` with its class, or be the UNDEFINED
    /// (0) or ERROR (1) factor with class 0, else VALUE_OUT_OF_RANGE.
    /// `update_time` is a `BACnetTimeStamp`, the Device clock's when
    /// omitted. While the reader is out of service the read is kept aside,
    /// in place of the reader's earlier one: a client's simulated values
    /// stay served, no COV report goes out, and the return to service
    /// serves the latest read. Any object other than a Credential Data Input
    /// raises OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. A new Update_Time sends
    /// the reader's COV report.
    #[pyo3(signature = (object_id, factor, *, update_time=None))]
    fn report_credential_read_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        factor: PyAuthenticationFactor,
        update_time: Option<PyBACnetTimeStamp>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let report = CredentialReadReport {
            update_time: update_time.map(|time| time.to_rust().clone()),
            ..CredentialReadReport::new(factor.into())
        };
        let future =
            self.report_access_input(object_id, AccessControlInput::CredentialRead(report));
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Report an Access Door's hardware state: whichever of Door_Status,
    /// Lock_Status and Door_Alarm_State is given, as BACnetDoorStatus,
    /// BACnetLockStatus and BACnetDoorAlarmState numbers, change together.
    ///
    /// A number outside its production, or an alarm state the door's
    /// Alarm_Values, Fault_Values and Masked_Alarm_Values don't admit,
    /// raises VALUE_OUT_OF_RANGE and nothing changes. While the door is out
    /// of service the values are kept aside, in place of the device's
    /// earlier ones: a client's simulated values stay served, no COV report
    /// or event follows, and the return to service serves the latest values
    /// reported. Any object other than an Access Door raises
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. In service a new
    /// Door_Alarm_State sends the door's COV report, and the door's event
    /// algorithm sees it at once.
    #[pyo3(signature = (object_id, *, door_status=None, lock_status=None, door_alarm_state=None))]
    fn report_door_state_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        door_status: Option<u32>,
        lock_status: Option<u32>,
        door_alarm_state: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let report = DoorStateReport {
            door_status: door_status.map(DoorStatus::from_raw),
            lock_status: lock_status.map(LockStatus::from_raw),
            door_alarm_state: door_alarm_state.map(DoorAlarmState::from_raw),
        };
        let future = self.report_access_input(object_id, AccessControlInput::DoorState(report));
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}
