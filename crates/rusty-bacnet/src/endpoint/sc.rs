//! SC Python endpoint owner: one hub connection above both roles.
//!
//! `ScEndpoint` holds `Mutex<Option<EndpointSession<ScTransport<TlsWebSocket>>>>`.
//! The single `device_uuid` parameter feeds both the transport dial and the
//! composed `DeviceIdentity` — no split identities.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_endpoint::sc::ScEndpointBuilder;
use bacnet_endpoint::session::{EndpointSession, SessionRole};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use tokio::sync::Mutex;

use crate::endpoint::common::{
    build_database, build_identity, build_pending_boxes, make_analog_input, make_analog_value,
    make_binary_input, make_binary_value, parse_device_uuid, parse_segmentation, parse_services,
    with_sc_port, PendingObject, PendingRestoreGuard,
};
use crate::endpoint::roles::{PyEndpointClient, PyEndpointServer};
use crate::errors::to_py_err;
use crate::types::PySegmentation;

type ScSession =
    EndpointSession<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>;

/// Owned, validated SC endpoint startup configuration (no I/O here).
#[derive(Clone)]
struct ScEndpointConfig {
    hub_url: String,
    vmac: [u8; 6],
    device_uuid: [u8; 16],
    ca_cert: String,
    client_cert: String,
    client_key: String,
    heartbeat_interval_ms: u64,
    heartbeat_timeout_ms: u64,
    identity: DeviceIdentity,
    queue_capacity: usize,
}

/// SC endpoint: one hub connection that both initiates and executes.
///
/// The `device_uuid` is the single durable lifetime identity (Annex AB.1.5.3):
/// it dials the hub AND syncs into DEVICE_UUID. Provision once, reuse for the
/// device lifetime. No generation or persistence is provided.
///
/// Lifecycle mirrors `BipEndpoint` (start-once, idempotent close, context
/// manager). BIPv6/Ethernet have no endpoint owner.
#[pyclass(name = "ScEndpoint")]
pub struct PyScEndpoint {
    inner: Arc<Mutex<Option<ScSession>>>,
    config: ScEndpointConfig,
    pending: Arc<std::sync::Mutex<Vec<PendingObject>>>,
    started: Arc<AtomicBool>,
}

impl PyScEndpoint {
    fn lock_pending(&self) -> PyResult<std::sync::MutexGuard<'_, Vec<PendingObject>>> {
        self.pending
            .lock()
            .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))
    }

    fn push_pending(&self, obj: PendingObject) -> PyResult<()> {
        let mut guard = self.lock_pending()?;
        if self.started.load(Ordering::Acquire) {
            return Err(PyRuntimeError::new_err(
                "cannot add objects after start() — endpoint is already running",
            ));
        }
        guard.push(obj);
        Ok(())
    }

    fn vmac_hex(&self) -> String {
        self.config
            .vmac
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":")
    }
}

#[pymethods]
impl PyScEndpoint {
    /// Create an SC endpoint (no I/O; validated before dial).
    ///
    /// Args:
    ///     device_instance / device_name / vendor_id: Single identity source
    ///         (vendor default 555 preserves the old hardcoded value).
    ///     sc_hub: Hub `wss://` URL (required).
    ///     sc_vmac: 6-byte VMAC, neither all zero nor all ff.
    ///     sc_device_uuid: Required keyword-only nonzero 16-byte durable UUID
    ///         (single source for transport + DEVICE_UUID).
    ///     sc_ca_cert / sc_client_cert / sc_client_key: Required nonempty
    ///         paths for mutual TLS (presence validated now, files read at start).
    ///     sc_heartbeat_interval_ms / sc_heartbeat_timeout_ms: Heartbeat timing
    ///         (defaults 30000/60000; interval 3000..=300000, timeout>interval).
    ///     network_number / network_port_instance: SC port entry (instance 2
    ///         by convention, VIRTUAL type).
    ///     max_apdu / segmentation / services / device name: Identity knobs
    ///         (same narrow defaults as BIP).
    ///     queue_capacity: Bounded queue capacity (must be >0). Client timers
    ///         stay at the session default; the SC builder exposes no timer
    ///         override by design.
    #[new]
    #[pyo3(signature = (
        device_instance,
        sc_hub,
        sc_vmac,
        sc_ca_cert,
        sc_client_cert,
        sc_client_key,
        *,
        sc_device_uuid,
        device_name="BACnet Device",
        vendor_id=555,
        sc_heartbeat_interval_ms=30000,
        sc_heartbeat_timeout_ms=60000,
        network_number=0,
        network_port_instance=2,
        max_apdu=1476,
        segmentation=None,
        services=None,
        queue_capacity=16
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        device_instance: u32,
        sc_hub: &str,
        sc_vmac: Vec<u8>,
        sc_ca_cert: &str,
        sc_client_cert: &str,
        sc_client_key: &str,
        sc_device_uuid: Vec<u8>,
        device_name: &str,
        vendor_id: u16,
        sc_heartbeat_interval_ms: u64,
        sc_heartbeat_timeout_ms: u64,
        network_number: u32,
        network_port_instance: u32,
        max_apdu: u16,
        segmentation: Option<PySegmentation>,
        services: Option<Vec<u8>>,
        queue_capacity: usize,
    ) -> PyResult<Self> {
        if queue_capacity == 0 {
            return Err(PyValueError::new_err(
                "queue_capacity must be greater than zero",
            ));
        }
        // Credential presence first (mirrors hub/client ordering).
        crate::tls::required_sc_credentials(
            (!sc_ca_cert.is_empty()).then_some(sc_ca_cert),
            (!sc_client_cert.is_empty()).then_some(sc_client_cert),
            (!sc_client_key.is_empty()).then_some(sc_client_key),
        )
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
        // VMAC: length is RuntimeError (hub parity), reservation ValueError.
        if sc_vmac.len() != 6 {
            return Err(PyRuntimeError::new_err("sc_vmac must be exactly 6 bytes"));
        }
        let mut vmac = [0u8; 6];
        vmac.copy_from_slice(&sc_vmac);
        if vmac == [0; 6] || vmac == [0xff; 6] {
            return Err(PyValueError::new_err(
                "sc_vmac must not be UNKNOWN (all zero) or BROADCAST (all ff)",
            ));
        }
        let uuid = parse_device_uuid(Some(sc_device_uuid), true, "sc_device_uuid")?;
        // Heartbeat range without hub I/O (mirrors ScTransport validation).
        if !(3_000..=300_000).contains(&sc_heartbeat_interval_ms) {
            return Err(PyValueError::new_err(format!(
                "sc_heartbeat_interval_ms must be 3000..=300000, got {sc_heartbeat_interval_ms}"
            )));
        }
        if sc_heartbeat_timeout_ms <= sc_heartbeat_interval_ms {
            return Err(PyValueError::new_err(format!(
                "sc_heartbeat_timeout_ms must exceed interval, got interval={sc_heartbeat_interval_ms} timeout={sc_heartbeat_timeout_ms}"
            )));
        }
        let segmentation = parse_segmentation(segmentation);
        let service_list = parse_services(services);
        let identity = build_identity(
            device_instance,
            device_name,
            vendor_id,
            max_apdu,
            segmentation,
            &service_list,
            uuid,
        )?;
        let identity = with_sc_port(identity, network_port_instance, network_number, vmac)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(None)),
            config: ScEndpointConfig {
                hub_url: sc_hub.to_string(),
                vmac,
                device_uuid: uuid,
                ca_cert: sc_ca_cert.to_string(),
                client_cert: sc_client_cert.to_string(),
                client_key: sc_client_key.to_string(),
                heartbeat_interval_ms: sc_heartbeat_interval_ms,
                heartbeat_timeout_ms: sc_heartbeat_timeout_ms,
                identity,
                queue_capacity,
            },
            pending: Arc::new(std::sync::Mutex::new(Vec::new())),
            started: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Test seam: pending registration count (drained by successful start;
    /// restored on failed/cancelled start).
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
        make_analog_input(instance, name, units, present_value)?;
        self.push_pending(PendingObject::AnalogInput {
            instance,
            name: name.to_string(),
            units,
            present_value,
        })
    }

    /// Add an Analog Value object (before start).
    #[pyo3(signature = (instance, name, units=62))]
    fn add_analog_value(&self, instance: u32, name: &str, units: u32) -> PyResult<()> {
        make_analog_value(instance, name, units)?;
        self.push_pending(PendingObject::AnalogValue {
            instance,
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
    #[pyo3(signature = (instance, name))]
    fn add_binary_value(&self, instance: u32, name: &str) -> PyResult<()> {
        make_binary_value(instance, name)?;
        self.push_pending(PendingObject::BinaryValue {
            instance,
            name: name.to_string(),
        })
    }

    /// Start the endpoint: dial hub, compose one SC session. Start-once.
    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        // Local TLS file errors precede pending drain so repaired files can
        // retry with registrations intact (server parity).
        let tls_config = crate::tls::build_client_tls_config(
            Some(self.config.ca_cert.as_str()),
            Some(self.config.client_cert.as_str()),
            Some(self.config.client_key.as_str()),
        )
        .map_err(|e| PyRuntimeError::new_err(format!("TLS config error: {e}")))?;
        // Dial before draining (RUN-1): dial failures and cancellation while
        // dialing leave pending untouched, so retry needs no re-registration.
        let pending = self.pending.clone();
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
            let ws = bacnet_transport::sc_tls::TlsWebSocket::connect(&config.hub_url, tls_config)
                .await
                .map_err(to_py_err)?;
            let mut restore = PendingRestoreGuard::new(pending.clone(), {
                let mut guard = pending
                    .lock()
                    .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))?;
                guard.drain(..).collect()
            });
            let boxes = build_pending_boxes(restore.objects())?;
            let db = build_database(&config.identity, boxes)?;
            let mut session = ScEndpointBuilder::new(config.vmac, config.device_uuid)
                .role(SessionRole::Both)
                .queue_capacity(config.queue_capacity)
                .heartbeat(config.heartbeat_interval_ms, config.heartbeat_timeout_ms)
                .database(db)
                .identity(config.identity.clone())
                .build_hub_session(ws)
                .map_err(to_py_err)?;
            session.start().await.map_err(to_py_err)?;
            *inner.lock().await = Some(session);
            restore.take();
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
        let (tls_config, pending, inner, started, config) = {
            let borrowed = slf.borrow();
            let tls_config = crate::tls::build_client_tls_config(
                Some(borrowed.config.ca_cert.as_str()),
                Some(borrowed.config.client_cert.as_str()),
                Some(borrowed.config.client_key.as_str()),
            )
            .map_err(|e| PyRuntimeError::new_err(format!("TLS config error: {e}")))?;
            (
                tls_config,
                borrowed.pending.clone(),
                borrowed.inner.clone(),
                borrowed.started.clone(),
                borrowed.config.clone(),
            )
        };
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            {
                let guard = inner.lock().await;
                if guard.is_some() {
                    return Ok(self_ref);
                }
            }
            let ws = bacnet_transport::sc_tls::TlsWebSocket::connect(&config.hub_url, tls_config)
                .await
                .map_err(to_py_err)?;
            let mut restore = PendingRestoreGuard::new(pending.clone(), {
                let mut guard = pending
                    .lock()
                    .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))?;
                guard.drain(..).collect()
            });
            let boxes = build_pending_boxes(restore.objects())?;
            let db = build_database(&config.identity, boxes)?;
            let mut session = ScEndpointBuilder::new(config.vmac, config.device_uuid)
                .role(SessionRole::Both)
                .queue_capacity(config.queue_capacity)
                .heartbeat(config.heartbeat_interval_ms, config.heartbeat_timeout_ms)
                .database(db)
                .identity(config.identity.clone())
                .build_hub_session(ws)
                .map_err(to_py_err)?;
            session.start().await.map_err(to_py_err)?;
            *inner.lock().await = Some(session);
            restore.take();
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

    /// VMAC hex for this SC node (validated startup config).
    fn local_address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let addr = self.vmac_hex();
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(addr) })
    }

    /// Bounded snapshot (same keys as BIP; transport "sc").
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
                dict.set_item("transport", "sc")?;
                dict.set_item(
                    "local_address",
                    config
                        .vmac
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<Vec<_>>()
                        .join(":"),
                )?;
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

    /// Broadcast one I-Am via the hub relay.
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
            "ScEndpoint(device={}, vmac={})",
            self.config.identity.instance(),
            self.vmac_hex()
        )
    }
}
