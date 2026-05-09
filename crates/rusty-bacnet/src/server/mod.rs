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
    // Passwords
    dcc_password: Option<String>,
    reinit_password: Option<String>,
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

mod server_methods {
    mod lifecycle;
    mod registration;
}
