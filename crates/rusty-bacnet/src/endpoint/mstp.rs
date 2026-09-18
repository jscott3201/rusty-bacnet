//! MS/TP Python endpoint owner: one serial owner above both roles.
//!
//! `MstpEndpoint` holds `Mutex<Option<EndpointSession<MstpTransport<PySerial>>>>`
//! with take-under-lock lifecycle. The serial port is taken once at build;
//! no second serial owner exists.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_endpoint::mstp::MstpEndpointBuilder;
use bacnet_endpoint::session::{EndpointSession, SessionRole};
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::mstp::MstpTransport;
use bacnet_transport::mstp_serial::{SerialConfig, TokioSerialPort};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use tokio::sync::Mutex;

use crate::endpoint::common::{
    build_database, build_identity, make_analog_input, make_analog_value, make_binary_input,
    make_binary_value, parse_device_uuid, parse_segmentation, parse_services,
};
use crate::endpoint::roles::{PyEndpointClient, PyEndpointServer};
use crate::errors::to_py_err;
use crate::types::PySegmentation;

/// MS/TP APDU bound enforced at composition (standard-frame transport).
const MSTP_MAX_APDU: u16 = 480;

/// Owned, validated MS/TP endpoint startup configuration (no I/O here).
#[derive(Clone)]
struct MstpEndpointConfig {
    serial_port: String,
    baud: u32,
    mac: u8,
    max_master: u8,
    max_info_frames: u8,
    identity: DeviceIdentity,
    queue_capacity: usize,
    apdu_timeout_ms: u64,
    apdu_retries: u8,
}

type MstpSession = EndpointSession<MstpTransport<TokioSerialPort>>;

/// MS/TP endpoint: one serial owner that both initiates and executes.
///
/// Simulator + bench guidance from the Rust builder applies: proofs run over
/// loopback serial in Rust; this binding opens a real serial device via
/// `TokioSerialPort` like the current wrappers. Timing qualification is RB-26.
///
/// Lifecycle mirrors `BipEndpoint`. BIPv6/Ethernet have no endpoint owner.
#[pyclass(name = "MstpEndpoint")]
pub struct PyMstpEndpoint {
    inner: Arc<Mutex<Option<MstpSession>>>,
    config: MstpEndpointConfig,
    pending: std::sync::Mutex<Vec<Box<dyn BACnetObject>>>,
    started: Arc<AtomicBool>,
}

impl PyMstpEndpoint {
    fn lock_pending(&self) -> PyResult<std::sync::MutexGuard<'_, Vec<Box<dyn BACnetObject>>>> {
        self.pending
            .lock()
            .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))
    }

    fn push_pending(&self, obj: Box<dyn BACnetObject>) -> PyResult<()> {
        let mut guard = self.lock_pending()?;
        if self.started.load(Ordering::Acquire) {
            return Err(PyRuntimeError::new_err(
                "cannot add objects after start() — endpoint is already running",
            ));
        }
        guard.push(obj);
        Ok(())
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
        apdu_retries=0
    ))]
    #[allow(clippy::too_many_arguments)]
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
    ) -> PyResult<Self> {
        if queue_capacity == 0 {
            return Err(PyValueError::new_err(
                "queue_capacity must be greater than zero",
            ));
        }
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
            inner: Arc::new(Mutex::new(None)),
            config: MstpEndpointConfig {
                serial_port: serial_port.to_string(),
                baud: mstp_baud,
                mac: mstp_mac,
                max_master: mstp_max_master,
                max_info_frames: mstp_max_info_frames,
                identity,
                queue_capacity,
                apdu_timeout_ms,
                apdu_retries,
            },
            pending: std::sync::Mutex::new(Vec::new()),
            started: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Test seam: pending registration count.
    #[doc(hidden)]
    fn _pending_registration_count(&self) -> PyResult<usize> {
        Ok(self.lock_pending()?.len())
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
        self.push_pending(make_analog_input(instance, name, units, present_value)?)
    }

    /// Add an Analog Value object (before start).
    #[pyo3(signature = (instance, name, units=62))]
    fn add_analog_value(&self, instance: u32, name: &str, units: u32) -> PyResult<()> {
        self.push_pending(make_analog_value(instance, name, units)?)
    }

    /// Add a Binary Input object (before start).
    #[pyo3(signature = (instance, name))]
    fn add_binary_input(&self, instance: u32, name: &str) -> PyResult<()> {
        self.push_pending(make_binary_input(instance, name)?)
    }

    /// Add a Binary Value object (before start).
    #[pyo3(signature = (instance, name))]
    fn add_binary_value(&self, instance: u32, name: &str) -> PyResult<()> {
        self.push_pending(make_binary_value(instance, name)?)
    }

    /// Start the endpoint: open serial once, compose one MS/TP session.
    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        // Synchronous serial open before draining pending (server parity:
        // open failures preserve registrations for retry).
        let serial = TokioSerialPort::open(&SerialConfig {
            port_name: self.config.serial_port.clone(),
            baud_rate: self.config.baud,
        })
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        let objects: Vec<Box<dyn BACnetObject>> = {
            let mut guard = self.lock_pending()?;
            guard.drain(..).collect()
        };
        let inner = self.inner.clone();
        let started = self.started.clone();
        let config = self.config.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            {
                let guard = inner.lock().await;
                if guard.is_some() {
                    return Err(to_py_err(bacnet_types::error::Error::Encoding(
                        "endpoint already started".into(),
                    )));
                }
            }
            let db = build_database(&config.identity, objects)?;
            let mut session = MstpEndpointBuilder::new(serial, config.mac)
                .max_master(config.max_master)
                .max_info_frames(config.max_info_frames)
                .baud_rate(config.baud)
                .role(SessionRole::Both)
                .queue_capacity(config.queue_capacity)
                .client_timers(config.apdu_timeout_ms, config.apdu_retries)
                .database(db)
                .identity(config.identity.clone())
                .build_session()
                .map_err(to_py_err)?;
            session.start().await.map_err(to_py_err)?;
            *inner.lock().await = Some(session);
            started.store(true, Ordering::Release);
            Ok(())
        })
    }

    /// Close the endpoint (idempotent).
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let started = self.started.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut session = { inner.lock().await.take() };
            if let Some(session) = session.as_mut() {
                let _ = session.stop().await;
            }
            started.store(false, Ordering::Release);
            Ok(())
        })
    }

    /// Start on context entry (idempotent when already running).
    fn __aenter__<'py>(slf: Bound<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let self_ref = slf.clone().unbind();
        let (serial, inner, started, config, objects) = {
            let borrowed = slf.borrow();
            let serial = TokioSerialPort::open(&SerialConfig {
                port_name: borrowed.config.serial_port.clone(),
                baud_rate: borrowed.config.baud,
            })
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            let mut guard = borrowed.lock_pending()?;
            let objects: Vec<Box<dyn BACnetObject>> = guard.drain(..).collect();
            (
                serial,
                borrowed.inner.clone(),
                borrowed.started.clone(),
                borrowed.config.clone(),
                objects,
            )
        };
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            {
                let guard = inner.lock().await;
                if guard.is_some() {
                    return Ok(self_ref);
                }
            }
            let db = build_database(&config.identity, objects)?;
            let mut session = MstpEndpointBuilder::new(serial, config.mac)
                .max_master(config.max_master)
                .max_info_frames(config.max_info_frames)
                .baud_rate(config.baud)
                .role(SessionRole::Both)
                .queue_capacity(config.queue_capacity)
                .client_timers(config.apdu_timeout_ms, config.apdu_retries)
                .database(db)
                .identity(config.identity.clone())
                .build_session()
                .map_err(to_py_err)?;
            session.start().await.map_err(to_py_err)?;
            *inner.lock().await = Some(session);
            started.store(true, Ordering::Release);
            Ok(self_ref)
        })
    }

    /// Forcefully close on context exit (idempotent).
    #[pyo3(signature = (_exc_type=None, _exc_val=None, _exc_tb=None))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _exc_type: Option<Bound<'py, PyAny>>,
        _exc_val: Option<Bound<'py, PyAny>>,
        _exc_tb: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let started = self.started.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut session = { inner.lock().await.take() };
            if let Some(session) = session.as_mut() {
                let _ = session.stop().await;
            }
            started.store(false, Ordering::Release);
            Ok(())
        })
    }

    /// Clone the client role.
    fn client<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
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
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
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
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(addr) })
    }

    /// Bounded snapshot (same keys as BIP; transport "mstp").
    fn status<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let config = self.config.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
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
            Python::attach(|py| {
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
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = inner.lock().await;
            let session = guard.as_ref().ok_or_else(|| {
                PyRuntimeError::new_err("endpoint not started — use 'async with' or await start()")
            })?;
            session.broadcast_i_am().await.map_err(to_py_err)?;
            Ok(())
        })
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
