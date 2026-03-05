//! Python BACnetServer — async wrapper around the Rust BACnetServer.

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use tokio::sync::Mutex;

use bacnet_objects::access_control::{
    AccessCredentialObject, AccessDoorObject, AccessPointObject, AccessRightsObject,
    AccessUserObject, AccessZoneObject, CredentialDataInputObject,
};
use bacnet_objects::accumulator::{AccumulatorObject, PulseConverterObject};
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::audit::{AuditLogObject, AuditReporterObject};
use bacnet_objects::averaging::AveragingObject;
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::command::CommandObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::elevator::{ElevatorGroupObject, EscalatorObject, LiftObject};
use bacnet_objects::event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject};
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::file::FileObject;
use bacnet_objects::forwarder::NotificationForwarderObject;
use bacnet_objects::group::{GlobalGroupObject, GroupObject, StructuredViewObject};
use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject};
use bacnet_objects::lighting::{BinaryLightingOutputObject, ChannelObject, LightingOutputObject};
use bacnet_objects::load_control::LoadControlObject;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_objects::network_port::NetworkPortObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::program::ProgramObject;
use bacnet_objects::schedule::{CalendarObject, ScheduleObject};
use bacnet_objects::staging::StagingObject;
use bacnet_objects::timer::TimerObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_objects::value_types::{
    BitStringValueObject, CharacterStringValueObject, DatePatternValueObject,
    DateTimePatternValueObject, DateTimeValueObject, DateValueObject, IntegerValueObject,
    LargeAnalogValueObject, OctetStringValueObject, PositiveIntegerValueObject,
    TimePatternValueObject, TimeValueObject,
};
use bacnet_server::server;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_transport::mstp::NoSerial;
use bacnet_types::primitives::PropertyValue;

use crate::errors::to_py_err;
use crate::types::{PyObjectIdentifier, PyPropertyIdentifier, PyPropertyValue};

/// Async BACnet server that hosts objects and responds to requests.
///
/// Usage:
/// ```python
/// server = BACnetServer(device_instance=1234, device_name="My Device")
/// server.add_analog_input(instance=1, name="Zone Temp", units=62, present_value=72.5)
/// await server.start()
/// # ... server is now responding to BACnet requests ...
/// await server.stop()
/// ```
///
/// Supports multiple transports via the `transport` parameter:
/// - `"bip"` (default): BACnet/IP over UDP
/// - `"ipv6"`: BACnet/IPv6 over UDP multicast
/// - `"sc"`: BACnet/SC over TLS WebSocket (requires `sc_hub`, `sc_vmac`)
#[pyclass(name = "BACnetServer")]
pub struct BACnetServer {
    inner: Arc<Mutex<Option<server::BACnetServer<AnyTransport<NoSerial>>>>>,
    device_instance: u32,
    device_name: String,
    transport_type: String,
    // BIP config
    interface: String,
    port: u16,
    broadcast_address: String,
    // SC config
    sc_hub: Option<String>,
    sc_vmac: Option<Vec<u8>>,
    sc_ca_cert: Option<String>,
    sc_client_cert: Option<String>,
    sc_client_key: Option<String>,
    sc_heartbeat_interval_ms: Option<u64>,
    sc_heartbeat_timeout_ms: Option<u64>,
    // IPv6 config
    ipv6_interface: Option<String>,
    /// Whether the server has been started.
    started: Arc<AtomicBool>,
    /// Objects to add before starting. Cleared after start.
    pending_objects: std::sync::Mutex<Vec<Box<dyn BACnetObject + Send>>>,
}

impl BACnetServer {
    /// Lock the pending_objects mutex, converting poison errors into PyRuntimeError.
    fn lock_pending(
        &self,
    ) -> PyResult<std::sync::MutexGuard<'_, Vec<Box<dyn BACnetObject + Send>>>> {
        self.pending_objects
            .lock()
            .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))
    }

    /// Atomically check that the server is not started and push an object.
    /// Prevents TOCTOU race between checking `started` and modifying `pending_objects`.
    fn push_pending(&self, obj: Box<dyn BACnetObject + Send>) -> PyResult<()> {
        let mut guard = self.lock_pending()?;
        if self.started.load(Ordering::Acquire) {
            return Err(PyRuntimeError::new_err(
                "cannot add objects after start() — server is already running",
            ));
        }
        guard.push(obj);
        Ok(())
    }
}

#[pymethods]
impl BACnetServer {
    #[new]
    #[pyo3(signature = (
        device_instance,
        device_name="BACnet Device",
        interface="0.0.0.0",
        port=0xBAC0,
        broadcast_address="255.255.255.255",
        transport="bip",
        sc_hub=None,
        sc_vmac=None,
        sc_ca_cert=None,
        sc_client_cert=None,
        sc_client_key=None,
        sc_heartbeat_interval_ms=None,
        sc_heartbeat_timeout_ms=None,
        ipv6_interface=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        device_instance: u32,
        device_name: &str,
        interface: &str,
        port: u16,
        broadcast_address: &str,
        transport: &str,
        sc_hub: Option<String>,
        sc_vmac: Option<Vec<u8>>,
        sc_ca_cert: Option<String>,
        sc_client_cert: Option<String>,
        sc_client_key: Option<String>,
        sc_heartbeat_interval_ms: Option<u64>,
        sc_heartbeat_timeout_ms: Option<u64>,
        ipv6_interface: Option<String>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            device_instance,
            device_name: device_name.to_string(),
            transport_type: transport.to_string(),
            interface: interface.to_string(),
            port,
            broadcast_address: broadcast_address.to_string(),
            sc_hub,
            sc_vmac,
            sc_ca_cert,
            sc_client_cert,
            sc_client_key,
            sc_heartbeat_interval_ms,
            sc_heartbeat_timeout_ms,
            ipv6_interface,
            started: Arc::new(AtomicBool::new(false)),
            pending_objects: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Add an Analog Input object to the server (before starting).
    #[pyo3(signature = (instance, name, units=62, present_value=0.0))]
    fn add_analog_input(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        present_value: f32,
    ) -> PyResult<()> {
        let mut ai = AnalogInputObject::new(instance, name, units).map_err(to_py_err)?;
        ai.set_present_value(present_value);
        self.push_pending(Box::new(ai))
    }

    /// Add a Binary Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_binary_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let bv = BinaryValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(bv))
    }

    /// Add an Analog Output object to the server (before starting).
    #[pyo3(signature = (instance, name, units=62))]
    fn add_analog_output(&self, instance: u32, name: &str, units: u32) -> PyResult<()> {
        let ao = AnalogOutputObject::new(instance, name, units).map_err(to_py_err)?;
        self.push_pending(Box::new(ao))
    }

    /// Add a Binary Input object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_binary_input(&self, instance: u32, name: &str) -> PyResult<()> {
        let bi = BinaryInputObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(bi))
    }

    /// Add a Binary Output object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_binary_output(&self, instance: u32, name: &str) -> PyResult<()> {
        let bo = BinaryOutputObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(bo))
    }

    /// Add a Multi-State Input object to the server (before starting).
    #[pyo3(signature = (instance, name, number_of_states))]
    fn add_multistate_input(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
    ) -> PyResult<()> {
        let msi =
            MultiStateInputObject::new(instance, name, number_of_states).map_err(to_py_err)?;
        self.push_pending(Box::new(msi))
    }

    /// Add a Multi-State Output object to the server (before starting).
    #[pyo3(signature = (instance, name, number_of_states))]
    fn add_multistate_output(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
    ) -> PyResult<()> {
        let mso =
            MultiStateOutputObject::new(instance, name, number_of_states).map_err(to_py_err)?;
        self.push_pending(Box::new(mso))
    }

    /// Add a Multi-State Value object to the server (before starting).
    #[pyo3(signature = (instance, name, number_of_states))]
    fn add_multistate_value(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
    ) -> PyResult<()> {
        let msv =
            MultiStateValueObject::new(instance, name, number_of_states).map_err(to_py_err)?;
        self.push_pending(Box::new(msv))
    }

    /// Add a Calendar object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_calendar(&self, instance: u32, name: &str) -> PyResult<()> {
        let cal = CalendarObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(cal))
    }

    /// Add a Schedule object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_schedule(&self, instance: u32, name: &str) -> PyResult<()> {
        let sched = ScheduleObject::new(instance, name, PropertyValue::Null).map_err(to_py_err)?;
        self.push_pending(Box::new(sched))
    }

    /// Add a Notification Class object to the server (before starting).
    #[pyo3(signature = (instance, name, notification_class=0))]
    fn add_notification_class(
        &self,
        instance: u32,
        name: &str,
        notification_class: u32,
    ) -> PyResult<()> {
        let mut nc = NotificationClass::new(instance, name).map_err(to_py_err)?;
        nc.notification_class = notification_class;
        self.push_pending(Box::new(nc))
    }

    /// Add a Trend Log object to the server (before starting).
    #[pyo3(signature = (instance, name, buffer_size=100))]
    fn add_trend_log(&self, instance: u32, name: &str, buffer_size: u32) -> PyResult<()> {
        let tl = TrendLogObject::new(instance, name, buffer_size).map_err(to_py_err)?;
        self.push_pending(Box::new(tl))
    }

    /// Add a Loop (PID) object to the server (before starting).
    #[pyo3(signature = (instance, name, output_units=62))]
    fn add_loop(&self, instance: u32, name: &str, output_units: u32) -> PyResult<()> {
        let lp = LoopObject::new(instance, name, output_units).map_err(to_py_err)?;
        self.push_pending(Box::new(lp))
    }

    /// Add an Audit Log object to the server (before starting).
    #[pyo3(signature = (instance, name, buffer_size=100))]
    fn add_audit_log(&self, instance: u32, name: &str, buffer_size: u32) -> PyResult<()> {
        let al = AuditLogObject::new(instance, name, buffer_size).map_err(to_py_err)?;
        self.push_pending(Box::new(al))
    }

    /// Add an Audit Reporter object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_audit_reporter(&self, instance: u32, name: &str) -> PyResult<()> {
        let ar = AuditReporterObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(ar))
    }

    // -----------------------------------------------------------------------
    // Pattern A: new(instance, name) — simple two-param constructors
    // -----------------------------------------------------------------------

    /// Add an Analog Value object to the server (before starting).
    #[pyo3(signature = (instance, name, units=62))]
    fn add_analog_value(&self, instance: u32, name: &str, units: u32) -> PyResult<()> {
        let obj = AnalogValueObject::new(instance, name, units).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Command object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_command(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = CommandObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Timer object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_timer(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = TimerObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Load Control object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_load_control(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LoadControlObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Program object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_program(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = ProgramObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Lighting Output object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_lighting_output(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LightingOutputObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Binary Lighting Output object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_binary_lighting_output(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = BinaryLightingOutputObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Life Safety Point object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_life_safety_point(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LifeSafetyPointObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Life Safety Zone object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_life_safety_zone(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LifeSafetyZoneObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Group object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_group(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = GroupObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Global Group object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_global_group(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = GlobalGroupObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Structured View object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_structured_view(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = StructuredViewObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Notification Forwarder object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_notification_forwarder(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = NotificationForwarderObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Alert Enrollment object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_alert_enrollment(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AlertEnrollmentObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Door object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_door(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessDoorObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Credential object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_credential(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessCredentialObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Point object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_point(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessPointObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Rights object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_rights(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessRightsObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access User object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_user(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessUserObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Zone object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_zone(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessZoneObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Credential Data Input object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_credential_data_input(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = CredentialDataInputObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Elevator Group object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_elevator_group(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = ElevatorGroupObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Escalator object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_escalator(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = EscalatorObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Averaging object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_averaging(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AveragingObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    // -----------------------------------------------------------------------
    // Value types — all take new(instance, name)
    // -----------------------------------------------------------------------

    /// Add an Integer Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_integer_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = IntegerValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Positive Integer Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_positive_integer_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = PositiveIntegerValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Large Analog Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_large_analog_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LargeAnalogValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Character String Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_character_string_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = CharacterStringValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Octet String Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_octet_string_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = OctetStringValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Bit String Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_bit_string_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = BitStringValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Date Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_date_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = DateValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Time Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_time_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = TimeValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a DateTime Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_date_time_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = DateTimeValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Date Pattern Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_date_pattern_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = DatePatternValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Time Pattern Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_time_pattern_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = TimePatternValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a DateTime Pattern Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_date_time_pattern_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = DateTimePatternValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    // -----------------------------------------------------------------------
    // Pattern B: new(instance, name, extra_param) — three-param constructors
    // -----------------------------------------------------------------------

    /// Add an Accumulator object to the server (before starting).
    #[pyo3(signature = (instance, name, units=62))]
    fn add_accumulator(&self, instance: u32, name: &str, units: u32) -> PyResult<()> {
        let obj = AccumulatorObject::new(instance, name, units).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Pulse Converter object to the server (before starting).
    #[pyo3(signature = (instance, name, units=62))]
    fn add_pulse_converter(&self, instance: u32, name: &str, units: u32) -> PyResult<()> {
        let obj = PulseConverterObject::new(instance, name, units).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a File object to the server (before starting).
    #[pyo3(signature = (instance, name, file_type="application/octet-stream"))]
    fn add_file(&self, instance: u32, name: &str, file_type: &str) -> PyResult<()> {
        let obj = FileObject::new(instance, name, file_type).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Network Port object to the server (before starting).
    #[pyo3(signature = (instance, name, network_type=0))]
    fn add_network_port(&self, instance: u32, name: &str, network_type: u32) -> PyResult<()> {
        let obj = NetworkPortObject::new(instance, name, network_type).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Event Enrollment object to the server (before starting).
    #[pyo3(signature = (instance, name, event_type=0))]
    fn add_event_enrollment(&self, instance: u32, name: &str, event_type: u32) -> PyResult<()> {
        let obj = EventEnrollmentObject::new(instance, name, event_type).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Channel object to the server (before starting).
    #[pyo3(signature = (instance, name, channel_number))]
    fn add_channel(&self, instance: u32, name: &str, channel_number: u32) -> PyResult<()> {
        let obj = ChannelObject::new(instance, name, channel_number).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Staging object to the server (before starting).
    #[pyo3(signature = (instance, name, num_stages))]
    fn add_staging(&self, instance: u32, name: &str, num_stages: usize) -> PyResult<()> {
        let obj = StagingObject::new(instance, name, num_stages).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Lift object to the server (before starting).
    #[pyo3(signature = (instance, name, num_floors))]
    fn add_lift(&self, instance: u32, name: &str, num_floors: usize) -> PyResult<()> {
        let obj = LiftObject::new(instance, name, num_floors).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Event Log object to the server (before starting).
    #[pyo3(signature = (instance, name, buffer_size=100))]
    fn add_event_log(&self, instance: u32, name: &str, buffer_size: u32) -> PyResult<()> {
        let obj = EventLogObject::new(instance, name, buffer_size).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Trend Log Multiple object to the server (before starting).
    #[pyo3(signature = (instance, name, buffer_size=100))]
    fn add_trend_log_multiple(&self, instance: u32, name: &str, buffer_size: u32) -> PyResult<()> {
        let obj = TrendLogMultipleObject::new(instance, name, buffer_size).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Start the server. It will begin responding to BACnet requests.
    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let started = self.started.clone();
        let device_instance = self.device_instance;
        let device_name = self.device_name.clone();
        let transport_type = self.transport_type.clone();
        let interface_str = self.interface.clone();
        let port = self.port;
        let broadcast_str = self.broadcast_address.clone();
        let sc_hub = self.sc_hub.clone();
        let sc_vmac = self.sc_vmac.clone();
        let sc_ca_cert = self.sc_ca_cert.clone();
        let sc_client_cert = self.sc_client_cert.clone();
        let sc_client_key = self.sc_client_key.clone();
        let sc_heartbeat_interval_ms = self.sc_heartbeat_interval_ms;
        let sc_heartbeat_timeout_ms = self.sc_heartbeat_timeout_ms;
        let ipv6_interface = self.ipv6_interface.clone();

        // Take pending objects (synchronous, before async block)
        let objects: Vec<Box<dyn BACnetObject + Send>> = {
            let mut guard = self.lock_pending()?;
            guard.drain(..).collect()
        };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut db = ObjectDatabase::new();

            // Create device object
            let mut device = DeviceObject::new(DeviceConfig {
                instance: device_instance,
                name: device_name,
                vendor_name: "Rusty BACnet".into(),
                vendor_id: 555,
                ..DeviceConfig::default()
            })
            .map_err(to_py_err)?;

            // Collect object identifiers for device object-list
            let dev_oid = device.object_identifier();
            let mut object_list = vec![dev_oid];

            // Move pending objects into the database
            for obj in objects {
                object_list.push(obj.object_identifier());
                db.add(obj).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "duplicate object name: {e}"
                    ))
                })?;
            }

            device.set_object_list(object_list);
            db.add(Box::new(device)).map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "duplicate object name: {e}"
                ))
            })?;

            // Build transport based on type
            let transport: AnyTransport<NoSerial> = match transport_type.as_str() {
                "bip" => {
                    let interface: Ipv4Addr = interface_str
                        .parse()
                        .map_err(|e| PyRuntimeError::new_err(format!("invalid interface: {e}")))?;
                    let broadcast: Ipv4Addr = broadcast_str
                        .parse()
                        .map_err(|e| PyRuntimeError::new_err(format!("invalid broadcast: {e}")))?;
                    AnyTransport::Bip(BipTransport::new(interface, port, broadcast))
                }
                "ipv6" => {
                    let iface_str = ipv6_interface.as_deref().unwrap_or("::");
                    let interface: std::net::Ipv6Addr = iface_str.parse().map_err(|e| {
                        PyRuntimeError::new_err(format!("invalid IPv6 interface: {e}"))
                    })?;
                    AnyTransport::Bip6(Bip6Transport::new(interface, port, None))
                }
                "sc" => {
                    let hub_url = sc_hub.ok_or_else(|| {
                        PyRuntimeError::new_err("sc_hub is required for SC transport")
                    })?;
                    let vmac_bytes = sc_vmac.ok_or_else(|| {
                        PyRuntimeError::new_err("sc_vmac is required for SC transport")
                    })?;
                    if vmac_bytes.len() != 6 {
                        return Err(PyRuntimeError::new_err("sc_vmac must be exactly 6 bytes"));
                    }
                    let mut vmac = [0u8; 6];
                    vmac.copy_from_slice(&vmac_bytes);

                    let tls_config = crate::tls::build_client_tls_config(
                        sc_ca_cert.as_deref(),
                        sc_client_cert.as_deref(),
                        sc_client_key.as_deref(),
                    )
                    .map_err(|e| PyRuntimeError::new_err(format!("TLS config error: {e}")))?;

                    let ws = bacnet_transport::sc_tls::TlsWebSocket::connect(&hub_url, tls_config)
                        .await
                        .map_err(to_py_err)?;

                    let mut sc = bacnet_transport::sc::ScTransport::new(ws, vmac);
                    if let Some(ms) = sc_heartbeat_interval_ms {
                        sc = sc.with_heartbeat_interval_ms(ms);
                    }
                    if let Some(ms) = sc_heartbeat_timeout_ms {
                        sc = sc.with_heartbeat_timeout_ms(ms);
                    }
                    AnyTransport::Sc(Box::new(sc))
                }
                other => {
                    return Err(PyRuntimeError::new_err(format!(
                        "unknown transport: '{other}'. Use 'bip', 'ipv6', or 'sc'"
                    )));
                }
            };

            let srv = server::BACnetServer::generic_builder()
                .database(db)
                .transport(transport)
                .build()
                .await
                .map_err(to_py_err)?;

            *inner.lock().await = Some(srv);
            started.store(true, Ordering::Release);
            Ok(())
        })
    }

    /// Stop the server.
    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let started = self.started.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            if let Some(mut srv) = guard.take() {
                srv.stop().await.map_err(to_py_err)?;
            }
            started.store(false, Ordering::Release);
            Ok(())
        })
    }

    /// Get the server's local address as a string.
    ///
    /// For BIP: "ip:port", for IPv6: "[ip]:port", for SC: hex-encoded VMAC.
    fn local_address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let transport_type = self.transport_type.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            let mac = srv.local_mac();
            match transport_type.as_str() {
                "bip" => {
                    if mac.len() < 6 {
                        return Err(PyRuntimeError::new_err(format!(
                            "unexpected BIP MAC length: {}",
                            mac.len()
                        )));
                    }
                    let ip = Ipv4Addr::new(mac[0], mac[1], mac[2], mac[3]);
                    let port = u16::from_be_bytes([mac[4], mac[5]]);
                    Ok(format!("{ip}:{port}"))
                }
                "ipv6" => {
                    if mac.len() < 18 {
                        return Err(PyRuntimeError::new_err(format!(
                            "unexpected IPv6 MAC length: {}",
                            mac.len()
                        )));
                    }
                    let mut ip_bytes = [0u8; 16];
                    ip_bytes.copy_from_slice(&mac[..16]);
                    let ip = std::net::Ipv6Addr::from(ip_bytes);
                    let port = u16::from_be_bytes([mac[16], mac[17]]);
                    Ok(format!("[{ip}]:{port}"))
                }
                _ => {
                    // SC, Ethernet, or other: hex-encode
                    Ok(mac
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<Vec<_>>()
                        .join(":"))
                }
            }
        })
    }

    // -----------------------------------------------------------------------
    // Runtime object access
    // -----------------------------------------------------------------------

    /// Read a property from a local object in the server's database.
    #[pyo3(signature = (object_id, property_id, array_index=None))]
    fn read_property<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let db_arc = {
                let guard = inner.lock().await;
                let srv = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
                srv.database().clone()
            };
            let db = db_arc.read().await;
            let obj = db
                .get(&oid)
                .ok_or_else(|| PyRuntimeError::new_err(format!("object {oid} not found")))?;
            let value = obj.read_property(pid, array_index).map_err(to_py_err)?;
            Ok(PyPropertyValue::from_rust(value))
        })
    }

    /// Write a property on a local object in the server's database.
    #[pyo3(signature = (object_id, property_id, value, priority=None, array_index=None))]
    #[allow(clippy::too_many_arguments)]
    fn write_property_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        value: PyPropertyValue,
        priority: Option<u8>,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();
        let prop_value = value.inner;

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let db_arc = {
                let guard = inner.lock().await;
                let srv = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
                srv.database().clone()
            };
            let mut db = db_arc.write().await;
            let obj = db
                .get_mut(&oid)
                .ok_or_else(|| PyRuntimeError::new_err(format!("object {oid} not found")))?;
            obj.write_property(pid, array_index, prop_value, priority)
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Get the server's current communication state.
    ///
    /// Returns 0=Enable, 1=Disable, 2=DisableInitiation.
    fn comm_state<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            Ok(srv.comm_state())
        })
    }
}
