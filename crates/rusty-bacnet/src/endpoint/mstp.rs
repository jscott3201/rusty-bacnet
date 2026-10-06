//! MS/TP Python endpoint owner: one serial owner above both roles.
//!
//! A private lifecycle owner admits serial acquisition exactly once per session
//! and retains startup/teardown across cancelled Python waiters.

use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_endpoint::mstp::MstpEndpointBuilder;
use bacnet_endpoint::session::{EndpointSession, SessionRole};
use bacnet_transport::mstp::MstpTransport;
use bacnet_transport::mstp_serial::{SerialConfig, TokioSerialPort};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::endpoint::common::{
    build_database, build_identity, build_pending_boxes, lifecycle_error, make_analog_input,
    make_analog_value, make_binary_input, make_binary_value, parse_device_uuid,
    parse_read_work_limit, parse_segmentation, parse_services, pending_group, PendingObject,
};
use crate::endpoint::lifecycle::Lifecycle;
use crate::endpoint::roles::{PyEndpointClient, PyEndpointServer};
use crate::errors::to_py_err;
use crate::types::{PyReadAccessSpec, PySegmentation};

/// MS/TP APDU bound enforced at composition (standard-frame transport).
const MSTP_MAX_APDU: u16 = 480;

/// Owned, validated MS/TP endpoint startup configuration (no I/O here).
#[derive(Clone)]
struct MstpEndpointConfig {
    serial_port: String,
    #[cfg(test)]
    serial_opener: Option<lifecycle_tests::SerialOpener>,
    baud: u32,
    mac: u8,
    max_master: u8,
    max_info_frames: u8,
    identity: DeviceIdentity,
    queue_capacity: usize,
    read_work_limit: usize,
    apdu_timeout_ms: u64,
    apdu_retries: u8,
    min_request_interval_ms: u64,
}

#[cfg(test)]
#[path = "mstp_lifecycle_tests.rs"]
mod lifecycle_tests;
#[cfg(test)]
type EndpointSerial = lifecycle_tests::TestSerial;
#[cfg(not(test))]
type EndpointSerial = TokioSerialPort;
type MstpSession = EndpointSession<MstpTransport<EndpointSerial>>;

impl MstpEndpointConfig {
    async fn prepare(self, objects: Vec<PendingObject>) -> PyResult<MstpSession> {
        let config = self;
        let serial_config = SerialConfig {
            port_name: config.serial_port.clone(),
            baud_rate: config.baud,
        };
        #[cfg(not(test))]
        let serial = TokioSerialPort::open(&serial_config)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        #[cfg(test)]
        let serial = match &config.serial_opener {
            Some(open) => open(&serial_config)?,
            None => lifecycle_tests::TestSerial::Real(
                TokioSerialPort::open(&serial_config)
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?,
            ),
        };
        let boxes = build_pending_boxes(&objects)?;
        let db = build_database(&config.identity, boxes)?;
        let session = MstpEndpointBuilder::new(serial, config.mac)
            .max_master(config.max_master)
            .max_info_frames(config.max_info_frames)
            .baud_rate(config.baud)
            .role(SessionRole::Both)
            .queue_capacity(config.queue_capacity)
            .read_work_limit(config.read_work_limit)
            .client_timers(config.apdu_timeout_ms, config.apdu_retries)
            .min_request_interval_ms(config.min_request_interval_ms)
            .database(db)
            .identity(config.identity.clone())
            .build_session()
            .map_err(to_py_err)?;
        Ok(session)
    }
}

/// MS/TP endpoint: one serial owner that both initiates and executes.
///
/// Simulator + bench guidance from the Rust builder applies: proofs run over
/// loopback serial in Rust; this binding opens a real serial device via
/// `TokioSerialPort` like the current wrappers. Timing qualification is RB-26.
///
/// Lifecycle mirrors `BipEndpoint`. BIPv6/Ethernet have no endpoint owner.
#[pyclass(name = "MstpEndpoint", module = "rusty_bacnet")]
pub struct PyMstpEndpoint {
    lifecycle: Lifecycle<MstpSession, PendingObject>,
    config: MstpEndpointConfig,
}

impl PyMstpEndpoint {
    fn push_pending(&self, obj: PendingObject) -> PyResult<()> {
        self.lifecycle.push(obj).map_err(|()| {
            PyRuntimeError::new_err(
                "cannot add objects while endpoint is starting, running, or stopping",
            )
        })
    }
}

#[pymethods]
impl PyMstpEndpoint {
    /// Create an MS/TP endpoint (no I/O; validated before open).
    ///
    /// Args:
    ///     device_instance / device_name / vendor_id: Single identity source.
    ///     serial_port: Serial device path (required).
    ///     mstp_baud: One of 9600/19200/38400/57600/76800/115200.
    ///     mstp_mac / mstp_max_master / mstp_max_info_frames: Addressing
    ///         (mac <= max_master, both <=127, info frames >=1).
    ///     max_apdu: Wire-legal APDU within the 480 MS/TP bound.
    ///     segmentation / services / device_uuid: Identity knobs (UUID zeros
    ///         allowed; stored into DEVICE_UUID).
    ///     queue_capacity / apdu_timeout_ms / apdu_retries: Session tuning.
    ///     read_work_limit: Keyword-only. Result rows one ReadProperty served
    ///         by the server role may expand (must be >0, default 256): its
    ///         own row plus, for a Group's Present_Value, one per member
    ///         property. A read past it is aborted with OUT_OF_RESOURCES.
    ///     min_request_interval_ms: Keyword-only (default 0, no pacing; at
    ///         most 3600000). Least time between the client role's confirmed
    ///         requests to one destination, as for `BACnetClient`.
    #[new]
    #[pyo3(signature = (
        device_instance,
        serial_port,
        device_name="BACnet Device",
        vendor_id=555,
        mstp_baud=38400,
        mstp_mac=1,
        mstp_max_master=127,
        mstp_max_info_frames=1,
        max_apdu=480,
        segmentation=None,
        services=None,
        device_uuid=None,
        queue_capacity=16,
        apdu_timeout_ms=6000,
        apdu_retries=0,
        *,
        read_work_limit=256,
        min_request_interval_ms=0
    ))]
    fn new(
        device_instance: u32,
        serial_port: &str,
        device_name: &str,
        vendor_id: u16,
        mstp_baud: u32,
        mstp_mac: u8,
        mstp_max_master: u8,
        mstp_max_info_frames: u8,
        max_apdu: u16,
        segmentation: Option<PySegmentation>,
        services: Option<Vec<u8>>,
        device_uuid: Option<Vec<u8>>,
        queue_capacity: usize,
        apdu_timeout_ms: u64,
        apdu_retries: u8,
        read_work_limit: usize,
        min_request_interval_ms: u64,
    ) -> PyResult<Self> {
        if queue_capacity == 0 {
            return Err(PyValueError::new_err(
                "queue_capacity must be greater than zero",
            ));
        }
        let read_work_limit = parse_read_work_limit(read_work_limit)?;
        let min_request_interval_ms =
            crate::client::parse_min_request_interval_ms(min_request_interval_ms)?;
        // Early addressing/baud validation without opening (ValueError).
        crate::mstp_py::validate_mstp_config(
            Some(serial_port),
            mstp_baud,
            mstp_mac,
            mstp_max_master,
            mstp_max_info_frames,
        )?;
        if max_apdu > MSTP_MAX_APDU {
            return Err(PyValueError::new_err(format!(
                "max_apdu {max_apdu} exceeds MS/TP transport bound {MSTP_MAX_APDU}"
            )));
        }
        let segmentation = parse_segmentation(segmentation);
        let service_list = parse_services(services);
        let uuid = parse_device_uuid(device_uuid, false, "device_uuid")?;
        let identity = build_identity(
            device_instance,
            device_name,
            vendor_id,
            max_apdu,
            segmentation,
            &service_list,
            uuid,
        )?;
        Ok(Self {
            lifecycle: Lifecycle::new(),
            config: MstpEndpointConfig {
                serial_port: serial_port.to_string(),
                #[cfg(test)]
                serial_opener: None,
                baud: mstp_baud,
                mac: mstp_mac,
                max_master: mstp_max_master,
                max_info_frames: mstp_max_info_frames,
                identity,
                queue_capacity,
                read_work_limit,
                apdu_timeout_ms,
                apdu_retries,
                min_request_interval_ms,
            },
        })
    }

    /// Test seam: pending registration count (drained by successful start;
    /// restored on failed/cancelled start).
    #[doc(hidden)]
    fn _pending_registration_count(&self) -> PyResult<usize> {
        Ok(self.lifecycle.pending_count())
    }

    /// Add an Analog Input object (before start).
    #[pyo3(signature = (instance, name, units=62, present_value=0.0))]
    fn add_analog_input(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        present_value: f32,
    ) -> PyResult<()> {
        make_analog_input(instance, name, units, present_value)?;
        self.push_pending(PendingObject::AnalogInput {
            instance,
            name: name.to_string(),
            units,
            present_value,
        })
    }

    /// Add an Analog Value object (before start).
    #[pyo3(signature = (instance, name, units=62, *, audit_level=None, auditable_operations=None, audit_priority_filter=None))]
    fn add_analog_value(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        audit_level: Option<&str>,
        auditable_operations: Option<&Bound<'_, PyAny>>,
        audit_priority_filter: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let audit_policy = crate::object_audit_policy::parse(
            audit_level,
            auditable_operations,
            audit_priority_filter,
        )?;
        make_analog_value(instance, name, units, audit_policy)?;
        self.push_pending(PendingObject::AnalogValue {
            instance,
            audit_policy,
            name: name.to_string(),
            units,
        })
    }

    /// Add a Binary Input object (before start).
    #[pyo3(signature = (instance, name))]
    fn add_binary_input(&self, instance: u32, name: &str) -> PyResult<()> {
        make_binary_input(instance, name)?;
        self.push_pending(PendingObject::BinaryInput {
            instance,
            name: name.to_string(),
        })
    }

    /// Add a Binary Value object (before start).
    #[pyo3(signature = (instance, name, *, audit_level=None, auditable_operations=None, audit_priority_filter=None))]
    fn add_binary_value(
        &self,
        instance: u32,
        name: &str,
        audit_level: Option<&str>,
        auditable_operations: Option<&Bound<'_, PyAny>>,
        audit_priority_filter: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let audit_policy = crate::object_audit_policy::parse(
            audit_level,
            auditable_operations,
            audit_priority_filter,
        )?;
        make_binary_value(instance, name, audit_policy)?;
        self.push_pending(PendingObject::BinaryValue {
            instance,
            audit_policy,
            name: name.to_string(),
        })
    }

    /// Add a Group object (before start); its Present_Value is rebuilt from
    /// `members` on each read, one result per member, in order.
    ///
    /// `members` takes the `read_property_multiple` spec shape,
    /// `(object_id, [(property_id, array_index), ...])`. A member listing no
    /// properties, or a group's Present_Value, is a ValueError naming its
    /// position and the rule; any property identifier is taken.
    #[pyo3(signature = (instance, name, members=None))]
    fn add_group(
        &self,
        instance: u32,
        name: &str,
        members: Option<Vec<PyReadAccessSpec>>,
    ) -> PyResult<()> {
        self.push_pending(pending_group(instance, name, members)?)
    }

    /// Start after lifecycle admission. A second explicit start fails.
    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let lifecycle = self.lifecycle.clone();
        let config = self.config.clone();
        crate::py_async::future_into_py(py, async move {
            lifecycle
                .start(false, |objects| config.prepare(objects))
                .await
                .map_err(lifecycle_error)?;
            crate::py_async::attach(|py| Ok(py.None()))
        })
    }

    /// Join earlier admitted startup and teardown; safe before start and twice.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let lifecycle = self.lifecycle.clone();
        crate::py_async::future_into_py(py, async move {
            lifecycle.close().await.map_err(lifecycle_error)?;
            crate::py_async::attach(|py| Ok(py.None()))
        })
    }

    /// Start on context entry, or reuse the already running session.
    fn __aenter__<'py>(slf: Bound<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let self_ref = slf.clone().unbind();
        let (lifecycle, config) = {
            let borrowed = slf.borrow();
            (borrowed.lifecycle.clone(), borrowed.config.clone())
        };
        crate::py_async::future_into_py(py, async move {
            lifecycle
                .start(true, |objects| config.prepare(objects))
                .await
                .map_err(lifecycle_error)?;
            Ok(self_ref)
        })
    }

    /// Await joined cleanup without suppressing the context body exception.
    #[pyo3(signature = (_exc_type=None, _exc_val=None, _exc_tb=None))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _exc_type: Option<Bound<'py, PyAny>>,
        _exc_val: Option<Bound<'py, PyAny>>,
        _exc_tb: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.close(py)
    }

    /// Clone the client role.
    fn client<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.lifecycle.session.clone();
        crate::py_async::future_into_py(py, async move {
            let handle = {
                let guard = inner.lock().await;
                let session = guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err(
                        "endpoint not started — use 'async with' or await start()",
                    )
                })?;
                session.cloned_client_handle().ok_or_else(|| {
                    PyRuntimeError::new_err(
                        "endpoint not started — use 'async with' or await start()",
                    )
                })?
            };
            Ok(PyEndpointClient::new(handle))
        })
    }

    /// Clone the server role.
    fn server<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.lifecycle.session.clone();
        crate::py_async::future_into_py(py, async move {
            let handle = {
                let guard = inner.lock().await;
                let session = guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err(
                        "endpoint not started — use 'async with' or await start()",
                    )
                })?;
                session.cloned_server_handle().ok_or_else(|| {
                    PyRuntimeError::new_err(
                        "endpoint not started — use 'async with' or await start()",
                    )
                })?
            };
            Ok(PyEndpointServer::new(handle))
        })
    }

    /// Station MAC as a decimal string (validated startup config).
    fn local_address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let addr = self.config.mac.to_string();
        crate::py_async::future_into_py(py, async move { Ok(addr) })
    }

    /// Bounded snapshot (same keys as BIP; transport "mstp").
    fn status<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.lifecycle.session.clone();
        let config = self.config.clone();
        crate::py_async::future_into_py(py, async move {
            let snapshot = {
                let guard = inner.lock().await;
                let session = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("endpoint not started"))?;
                let counters = session.policy_counters().await;
                let leases = session.active_leases();
                let running = session.is_running();
                (counters, leases, running)
            };
            crate::py_async::attach(|py| {
                let dict = PyDict::new(py);
                dict.set_item("is_running", snapshot.2)?;
                dict.set_item("device_instance", config.identity.instance())?;
                dict.set_item("vendor_id", config.identity.vendor_id())?;
                dict.set_item("max_apdu", config.identity.max_apdu_length())?;
                dict.set_item("transport", "mstp")?;
                dict.set_item("local_address", config.mac.to_string())?;
                dict.set_item("active_leases", snapshot.1)?;
                dict.set_item("ingress_policy", snapshot.0.ingress_policy)?;
                dict.set_item("no_server_role", snapshot.0.no_server_role)?;
                dict.set_item("no_client_role", snapshot.0.no_client_role)?;
                dict.set_item("unclaimed_terminal", snapshot.0.unclaimed_terminal)?;
                dict.set_item("responder_declined", snapshot.0.responder_declined)?;
                Ok(dict.into_any().unbind())
            })
        })
    }

    /// Broadcast one I-Am (MS/TP local broadcast).
    fn broadcast_i_am<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.lifecycle.session.clone();
        let future = async move {
            let guard = inner.lock().await;
            let session = guard.as_ref().ok_or_else(|| {
                PyRuntimeError::new_err("endpoint not started — use 'async with' or await start()")
            })?;
            session.broadcast_i_am().await.map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Device instance from the single identity (no I/O).
    #[getter]
    fn device_instance(&self) -> u32 {
        self.config.identity.instance()
    }

    /// Vendor identifier from the single identity (no I/O).
    #[getter]
    fn vendor_id(&self) -> u16 {
        self.config.identity.vendor_id()
    }

    fn __repr__(&self) -> String {
        format!(
            "MstpEndpoint(device={}, station={})",
            self.config.identity.instance(),
            self.config.mac
        )
    }
}
