//! SC Python endpoint owner: one hub connection above both roles.
//!
//! A private lifecycle owner orders connection preparation and session teardown.
//! The single `device_uuid` parameter feeds both the transport dial and the
//! composed `DeviceIdentity` — no split identities. The hub link's events and
//! status come from the session's link monitor (#1946).

use std::sync::Arc;

use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_endpoint::sc::ScEndpointBuilder;
use bacnet_endpoint::session::{EndpointSession, SessionRole};
use bacnet_transport::sc::{ScHubDialer, ScLinkMonitor, ScReconnectConfig};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::endpoint::common::{
    PendingObject, build_database, build_identity, build_pending_boxes, lifecycle_error,
    parse_device_uuid, parse_read_work_limit, parse_segmentation, parse_services, pending_group,
    with_sc_port,
};
use crate::endpoint::lifecycle::Lifecycle;
use crate::endpoint::roles::{PyEndpointClient, PyEndpointServer};
use crate::errors::to_py_err;
use crate::sc_events::{FirstScEvents, PyScEventIterator, PyScLinkStatus};
use crate::types::{PyReadAccessSpec, PySegmentation};

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
    read_work_limit: usize,
    min_request_interval_ms: u64,
    reinitialize: crate::reinitialize::EndpointReinitialize,
    /// The `device_write_policy` callable, if any (#2033).
    device_writes: crate::mutation_policy::DeviceWritePolicy,
    /// The source Audit Reporter keywords, if any (#2357).
    source_audit: Option<super::source_audit::SourceAuditKeywords>,
    /// The `reconnect` keyword's policy, if any (#1946).
    reconnect: Option<ScReconnectConfig>,
    /// The `sc_failover_hub` URL, if any; needs `reconnect`.
    failover_hub: Option<String>,
    /// The `sc_connect_timeout_ms` bound, if given; the dialer's 10 s if not.
    connect_timeout_ms: Option<u64>,
}

impl ScEndpointConfig {
    /// Dial the hub through the shared SC hub dialer and compose the session,
    /// keeping an event receiver subscribed before the lifecycle starts it.
    async fn prepare(
        self,
        objects: Vec<PendingObject>,
        first_events: Arc<FirstScEvents>,
    ) -> PyResult<ScSession> {
        let config = self;
        let tls_config = crate::tls::build_client_tls_config(
            Some(config.ca_cert.as_str()),
            Some(config.client_cert.as_str()),
            Some(config.client_key.as_str()),
        )
        .map_err(|e| PyRuntimeError::new_err(format!("TLS config error: {e}")))?;
        let boxes = build_pending_boxes(&objects)?;
        let mut db = build_database(&config.identity, boxes)?;
        if let Some(source) = &config.source_audit {
            source.provision(&config.identity, &mut db)?;
        }
        let mut hubs = ScHubDialer::new(&config.hub_url, tls_config);
        if let Some(url) = &config.failover_hub {
            hubs = hubs.failover_url(url);
        }
        if let Some(ms) = config.connect_timeout_ms {
            hubs = hubs.connect_timeout_ms(ms);
        }
        let mut builder = ScEndpointBuilder::new(config.vmac, config.device_uuid)
            .role(SessionRole::Both)
            .queue_capacity(config.queue_capacity)
            .read_work_limit(config.read_work_limit)
            .min_request_interval_ms(config.min_request_interval_ms)
            .heartbeat(config.heartbeat_interval_ms, config.heartbeat_timeout_ms)
            .database(db)
            .device(config.identity.clone());
        if let Some(reconnect) = config.reconnect.clone() {
            builder = builder.reconnect(reconnect);
        }
        let session = Box::pin(builder.connect_hub_session(hubs))
            .await
            .map_err(to_py_err)?;
        // Subscribed before the lifecycle starts the session, so the first
        // connection_events() call sees the first connected (#1908).
        if let Some(link) = session.sc_link() {
            first_events.keep(link.connection_events());
        }
        let session = match &config.source_audit {
            Some(source) => source.select(session),
            None => session,
        };
        let session = config.device_writes.install(session);
        Ok(config.reinitialize.install(session))
    }
}

/// SC endpoint: one hub connection that both initiates and executes.
///
/// The `device_uuid` is the single durable lifetime identity (Annex AB.1.5.3):
/// it dials the hub AND syncs into DEVICE_UUID. Provision once, reuse for the
/// device lifetime. No generation or persistence is provided.
///
/// Lifecycle mirrors `BipEndpoint` (one running session, joined close, context
/// manager). BIPv6/Ethernet have no endpoint owner.
#[pyclass(name = "ScEndpoint", module = "rusty_bacnet")]
pub struct PyScEndpoint {
    lifecycle: Lifecycle<ScSession, PendingObject>,
    config: ScEndpointConfig,
    /// Each start's receiver for the first `connection_events()` call; the
    /// next start replaces one nobody took.
    first_events: Arc<FirstScEvents>,
}

/// The started session's hub link; RuntimeError before start or after close.
async fn sc_link(session: &tokio::sync::Mutex<Option<ScSession>>) -> PyResult<ScLinkMonitor> {
    session
        .lock()
        .await
        .as_ref()
        .and_then(EndpointSession::sc_link)
        .ok_or_else(|| {
            PyRuntimeError::new_err("endpoint not started — use 'async with' or await start()")
        })
}

impl PyScEndpoint {
    fn push_pending(&self, obj: PendingObject) -> PyResult<()> {
        self.lifecycle.push(obj).map_err(|()| {
            PyRuntimeError::new_err(
                "cannot add objects while endpoint is starting, running, or stopping",
            )
        })
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

super::builders::endpoint_builders!(PyScEndpoint);

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
    ///         (same narrow defaults as BIP). The raw receive ceiling resolves
    ///         against the SC link before Device construction or dialing.
    ///     queue_capacity: Bounded queue capacity (must be >0). Client timers
    ///         stay at the session default; the SC builder exposes no timer
    ///         override by design.
    ///     read_work_limit: Result rows one ReadProperty served by the server
    ///         role may expand (must be >0, default 256): its own row plus,
    ///         for a Group's Present_Value, one per member property. A read
    ///         past it is aborted with OUT_OF_RESOURCES.
    ///     min_request_interval_ms: Least time between the client role's
    ///         confirmed requests to one destination (default 0, no pacing;
    ///         at most 3600000), as for `BACnetClient`.
    ///     on_reinitialize / after_reinitialize / reinit_password: Execute
    ///         ReinitializeDevice, as `BACnetServer`'s keywords of the same
    ///         names do (#1566).
    ///     device_write_policy: Keyword-only (default None). A callable that
    ///         decides each WriteProperty to the local Device's Description
    ///         or installed Audit recipient (#2033). It gets a
    ///         `MutationContext` and returns a bool, with the fail-closed
    ///         rules of `BACnetServer`'s `mutation_policy` callable. Without
    ///         it the endpoint executes no WriteProperty.
    ///     source_audit_reporter / audit_recipient / audit_device_bindings:
    ///         Keyword-only, as on `BipEndpoint` (#2348, #2357), each binding
    ///         address a VMAC as `local_address()` gives it
    ///         (`"xx:xx:xx:xx:xx:xx"`) or as twelve hex digits, or a routed
    ///         `"network:0xMAC"` (#2371). An address
    ///         `audit_recipient` takes another node's VMAC: never the
    ///         broadcast or all-zero VMAC or this endpoint's own.
    ///     reconnect / sc_failover_hub / sc_connect_timeout_ms: Keyword-only
    ///         (default None), named and checked as on `BACnetClient`
    ///         (#1946): an `ScReconnect` to redial a lost hub, a failover
    ///         hub's URL (needs `reconnect`), and a bound of at least 1 ms
    ///         on each hub dial and Connect-Request (default 10000).
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
        queue_capacity=16,
        read_work_limit=256,
        min_request_interval_ms=0,
        on_reinitialize=None,
        after_reinitialize=None,
        reinit_password=None,
        device_write_policy=None,
        source_audit_reporter=None,
        audit_recipient=None,
        audit_device_bindings=None,
        reconnect=None,
        sc_failover_hub=None,
        sc_connect_timeout_ms=None
    ))]
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
        read_work_limit: usize,
        min_request_interval_ms: u64,
        on_reinitialize: Option<&Bound<'_, PyAny>>,
        after_reinitialize: Option<&Bound<'_, PyAny>>,
        reinit_password: Option<String>,
        device_write_policy: Option<&Bound<'_, PyAny>>,
        source_audit_reporter: Option<crate::types::PyObjectIdentifier>,
        audit_recipient: Option<&Bound<'_, PyAny>>,
        audit_device_bindings: Option<Vec<(crate::types::PyObjectIdentifier, String)>>,
        reconnect: Option<PyRef<'_, crate::sc_reconnect::PyScReconnect>>,
        sc_failover_hub: Option<String>,
        sc_connect_timeout_ms: Option<u64>,
    ) -> PyResult<Self> {
        let reinitialize = crate::reinitialize::EndpointReinitialize::parse(
            on_reinitialize,
            after_reinitialize,
            reinit_password,
        )?;
        let device_writes = crate::mutation_policy::DeviceWritePolicy::parse(device_write_policy)?;
        if queue_capacity == 0 {
            return Err(PyValueError::new_err(
                "queue_capacity must be greater than zero",
            ));
        }
        let read_work_limit = parse_read_work_limit(read_work_limit)?;
        let min_request_interval_ms =
            crate::client::parse_min_request_interval_ms(min_request_interval_ms)?;
        crate::sc_reconnect::hub_options(
            "sc",
            reconnect.is_some(),
            sc_failover_hub.as_deref(),
            sc_connect_timeout_ms,
        )?;
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
        )?
        .resolve(
            bacnet_transport::sc::ScTransport::<
                bacnet_transport::sc_tls::TlsWebSocket,
            >::LOCAL_RECEIVE_APDU_CAPACITY,
        )
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let identity = with_sc_port(identity, network_port_instance, network_number, vmac)?;
        let source_audit = super::source_audit::SourceAuditKeywords::parse(
            source_audit_reporter,
            audit_recipient,
            audit_device_bindings,
            device_instance,
            super::source_audit::AuditLink::Sc(vmac),
            device_writes.enabled(),
        )?;
        Ok(Self {
            lifecycle: Lifecycle::new(),
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
                read_work_limit,
                min_request_interval_ms,
                reinitialize,
                device_writes,
                source_audit,
                reconnect: reconnect.map(|options| options.config.clone()),
                failover_hub: sc_failover_hub,
                connect_timeout_ms: sc_connect_timeout_ms,
            },
            first_events: Arc::default(),
        })
    }

    /// Test seam: pending registration count (drained by successful start;
    /// restored on failed/cancelled start).
    #[doc(hidden)]
    fn _pending_registration_count(&self) -> PyResult<usize> {
        Ok(self.lifecycle.pending_count())
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
        crate::logging_bridge::refresh_at_start(py);
        let lifecycle = self.lifecycle.clone();
        let config = self.config.clone();
        let first_events = Arc::clone(&self.first_events);
        config.reinitialize.bind_event_loop(py)?;
        crate::py_async::future_into_py(py, async move {
            lifecycle
                .start(false, |objects| config.prepare(objects, first_events))
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
        crate::logging_bridge::refresh_at_start(py);
        let self_ref = slf.clone().unbind();
        let (lifecycle, config, first_events) = {
            let borrowed = slf.borrow();
            (
                borrowed.lifecycle.clone(),
                borrowed.config.clone(),
                Arc::clone(&borrowed.first_events),
            )
        };
        config.reinitialize.bind_event_loop(py)?;
        crate::py_async::future_into_py(py, async move {
            lifecycle
                .start(true, |objects| config.prepare(objects, first_events))
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
        let reinitialize = self.config.reinitialize.enabled();
        let device_writes = self.config.device_writes.enabled();
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
            Ok(PyEndpointServer::new(handle, reinitialize, device_writes))
        })
    }

    /// VMAC hex for this SC node (validated startup config).
    fn local_address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let addr = self.vmac_hex();
        crate::py_async::future_into_py(py, async move { Ok(addr) })
    }

    /// Bounded snapshot (same keys as BIP; transport "sc").
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

    /// Subscribe to the hub-connector events (#1946): an async iterator of
    /// `ScEvent`, as `BACnetClient.connection_events()` gives. The first call
    /// after a start gets the events since the session was built, from its
    /// first `connected` (#1908); each later call gets the events from that
    /// call on. Iteration ends when the endpoint closes. Raises RuntimeError
    /// before start or after close.
    fn connection_events<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.lifecycle.session.clone();
        let first_events = Arc::clone(&self.first_events);
        crate::py_async::future_into_py(py, async move {
            let link = sc_link(&inner).await?;
            Ok(PyScEventIterator::new(
                first_events
                    .take()
                    .unwrap_or_else(|| link.connection_events()),
            ))
        })
    }

    /// A snapshot of the hub link (#1946), as `BACnetClient.connection_status()`
    /// gives: state, active hub, hub VMAC and heartbeat timing, without
    /// network I/O. Raises RuntimeError before start or after close.
    fn connection_status<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.lifecycle.session.clone();
        crate::py_async::future_into_py(py, async move {
            Ok(PyScLinkStatus(sc_link(&inner).await?.connection_status()))
        })
    }

    /// Broadcast one I-Am via the hub relay.
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
            "ScEndpoint(device={}, vmac={})",
            self.config.identity.instance(),
            self.vmac_hex()
        )
    }
}
