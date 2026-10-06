//! B/IP Python endpoint owner: one UDP socket above both roles.
//!
//! A private lifecycle owner orders preparation, session startup and joined
//! teardown. Role handles hold cloned role values with no lifecycle.
//!
//! Builder-time config only: the constructor validates the single
//! `DeviceIdentity` (instance/vendor/APDU/segmentation/services/ports/UUID)
//! plus transport addressing. No post-start configuration mutation exists.
//! The default vendor identifier is 555 and flows through this single identity.

use std::net::Ipv4Addr;

use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_endpoint::session::{EndpointSession, SessionRole};
use bacnet_transport::bip::BipTransport;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::endpoint::common::{
    build_database, build_identity, build_pending_boxes, lifecycle_error, make_analog_input,
    make_analog_value, make_binary_input, make_binary_value, parse_device_uuid, parse_ipv4,
    parse_read_work_limit, parse_segmentation, parse_services, pending_group, with_bip_port,
    PendingObject,
};
use crate::endpoint::lifecycle::Lifecycle;
use crate::endpoint::roles::{PyEndpointClient, PyEndpointServer};
use crate::errors::to_py_err;
use crate::types::{PyReadAccessSpec, PySegmentation};

/// Owned, validated B/IP endpoint startup configuration.
///
/// Everything is validated in the constructor before any bind: the async
/// start path only reuses these values through the native builder chain.
#[derive(Clone)]
struct BipEndpointConfig {
    interface: Ipv4Addr,
    port: u16,
    broadcast: Ipv4Addr,
    share_port_by_address: bool,
    identity: DeviceIdentity,
    queue_capacity: usize,
    read_work_limit: usize,
    apdu_timeout_ms: u64,
    apdu_retries: u8,
    registered_network_port: Option<u32>,
    min_request_interval_ms: u64,
}

type BipSession = EndpointSession<BipTransport>;

impl BipEndpointConfig {
    async fn prepare(self, objects: Vec<PendingObject>) -> PyResult<BipSession> {
        let config = self;
        // Fail fast before building objects: probe the port and the
        // interface, so conflicts preserve pending for retry. The real bind
        // stays authoritative (TOCTOU residual: a race loser still restores
        // via the owner). A port shared by address checks exactly what the
        // transport will bind (#1538).
        if config.share_port_by_address {
            let mut transport = BipTransport::new(config.interface, config.port, config.broadcast);
            transport.set_share_port_by_address(true);
            transport.check_bind().map_err(to_py_err)?;
        } else {
            let wildcard = std::net::SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, config.port);
            if let Err(e) = std::net::UdpSocket::bind(wildcard) {
                return Err(to_py_err(bacnet_types::error::Error::Transport(e)));
            }
            if !config.interface.is_unspecified() {
                let probe = std::net::SocketAddrV4::new(config.interface, 0);
                if let Err(e) = std::net::UdpSocket::bind(probe) {
                    return Err(to_py_err(bacnet_types::error::Error::Transport(e)));
                }
            }
        }
        let boxes = build_pending_boxes(&objects)?;
        let db = build_database(&config.identity, boxes)?;
        let mut builder = BipEndpointBuilder::new(config.interface, config.port, config.broadcast)
            .share_port_by_address(config.share_port_by_address)
            .role(SessionRole::Both)
            .queue_capacity(config.queue_capacity)
            .read_work_limit(config.read_work_limit)
            .client_timers(config.apdu_timeout_ms, config.apdu_retries)
            .min_request_interval_ms(config.min_request_interval_ms)
            .database(db)
            .identity(config.identity.clone());
        if let Some(instance) = config.registered_network_port {
            let oid = bacnet_types::primitives::ObjectIdentifier::new(
                bacnet_types::enums::ObjectType::NETWORK_PORT,
                instance,
            )
            .map_err(to_py_err)?;
            builder = builder.registered_network_port(oid);
        }
        let session = builder.build_session().map_err(to_py_err)?;
        Ok(session)
    }
}

/// B/IP endpoint: one device that both initiates and executes.
///
/// Usage:
/// ```python
/// endpoint = BipEndpoint(device_instance=1001, vendor_id=42, port=47808)
/// endpoint.add_analog_input(instance=1, name="Zone Temp", present_value=21.5)
/// async with endpoint:
///     client = await endpoint.client()
///     value = await client.read_property("127.0.0.1:47808", oid, pid)
/// ```
///
/// Lifecycle: `start()` builds one `BipTransport` (one UDP socket) above
/// both roles; `close()` is idempotent; `__aenter__` starts (idempotent when
/// already running) and `__aexit__` forcefully closes. Dropping without
/// awaiting close only seals forcefully and cannot guarantee awaited close —
/// always await `close()` or context exit when cleanup matters.
///
/// Two separately constructed objects (`BACnetClient` + `BACnetServer`) are
/// documented as two connections (two sockets/ports). The endpoint is the
/// one-transport path: one socket serves both roles.
///
/// BIPv6/Ethernet have no endpoint owner: keep the standalone
/// `BACnetClient`/`BACnetServer` path there.
#[pyclass(name = "BipEndpoint", module = "rusty_bacnet")]
pub struct PyBipEndpoint {
    lifecycle: Lifecycle<BipSession, PendingObject>,
    config: BipEndpointConfig,
}

impl PyBipEndpoint {
    fn push_pending(&self, obj: PendingObject) -> PyResult<()> {
        self.lifecycle.push(obj).map_err(|()| {
            PyRuntimeError::new_err(
                "cannot add objects while endpoint is starting, running, or stopping",
            )
        })
    }
}

#[pymethods]
impl PyBipEndpoint {
    /// Create a B/IP endpoint (no I/O; validated before bind).
    ///
    /// Args:
    ///     device_instance: BACnet Device instance (validated range).
    ///     device_name: Device object name (default "BACnet Device").
    ///     vendor_id: Vendor identifier (default 555).
    ///     interface: Announced IPv4 (the socket binds INADDR_ANY for broadcast
    ///         unless share_port_by_address is set).
    ///     port: UDP port; zero selects an ephemeral port reported after startup.
    ///     broadcast_address: Local broadcast address.
    ///     network_number: BACnet network number for the Network-Port entry.
    ///     network_port_instance: Declared Network Port instance (default 1).
    ///     registered_network_port: Explicit instance to associate with the live
    ///         NORMAL B/IP transport; None leaves the declaration unbound.
    ///     max_apdu: Wire-legal APDU (50/128/206/480/1024/1476).
    ///     segmentation: `Segmentation` override (default NONE, the proven value).
    ///     services: Optional service-bit list (default `[READ_PROPERTY]`, the
    ///         narrow endpoint reality; wider profiles need the full server).
    ///     device_uuid: Optional 16-byte UUID (zeros allowed for BIP-only;
    ///         stored into DEVICE_UUID, single identity source).
    ///     queue_capacity: Bounded queue capacity (must be >0).
    ///     apdu_timeout_ms / apdu_retries: Client timers.
    ///     read_work_limit: Keyword-only. Result rows one ReadProperty served
    ///         by the server role may expand (must be >0, default 256): its
    ///         own row plus, for a Group's Present_Value, one per member
    ///         property. A read past it is aborted with OUT_OF_RESOURCES.
    ///     share_port_by_address: Keyword-only (default False). Bind the
    ///         interface address itself, so endpoints on other addresses of
    ///         this host can share the port; needs an explicit interface and
    ///         a nonzero port. Broadcasts and unicast then arrive in no fixed
    ///         order.
    ///     min_request_interval_ms: Keyword-only (default 0, no pacing; at
    ///         most 3600000). Least time between the client role's confirmed
    ///         requests to one destination, as for `BACnetClient`.
    #[new]
    #[pyo3(signature = (
        device_instance,
        device_name="BACnet Device",
        vendor_id=555,
        interface="0.0.0.0",
        port=0xBAC0,
        broadcast_address="255.255.255.255",
        network_number=0,
        network_port_instance=1,
        max_apdu=1476,
        segmentation=None,
        services=None,
        device_uuid=None,
        queue_capacity=16,
        apdu_timeout_ms=6000,
        apdu_retries=0,
        registered_network_port=None,
        *,
        read_work_limit=256,
        share_port_by_address=false,
        min_request_interval_ms=0
    ))]
    fn new(
        device_instance: u32,
        device_name: &str,
        vendor_id: u16,
        interface: &str,
        port: u16,
        broadcast_address: &str,
        network_number: u32,
        network_port_instance: u32,
        max_apdu: u16,
        segmentation: Option<PySegmentation>,
        services: Option<Vec<u8>>,
        device_uuid: Option<Vec<u8>>,
        queue_capacity: usize,
        apdu_timeout_ms: u64,
        apdu_retries: u8,
        registered_network_port: Option<u32>,
        read_work_limit: usize,
        share_port_by_address: bool,
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
        let interface_ip = parse_ipv4(interface, "interface")?;
        let broadcast = parse_ipv4(broadcast_address, "broadcast_address")?;
        if let Some(selected) = registered_network_port {
            if !(1..=255).contains(&selected)
                || selected != network_port_instance
                || interface_ip.is_unspecified()
                || interface_ip.is_multicast()
                || interface_ip.is_broadcast()
                || interface_ip == broadcast
            {
                return Err(PyValueError::new_err("registered_network_port must select the declared 1..255 port on a concrete unicast interface"));
            }
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
        let identity = with_bip_port(
            identity,
            network_port_instance,
            network_number,
            interface_ip,
            port,
        )?;
        Ok(Self {
            lifecycle: Lifecycle::new(),
            config: BipEndpointConfig {
                interface: interface_ip,
                port,
                broadcast,
                share_port_by_address,
                identity,
                queue_capacity,
                read_work_limit,
                apdu_timeout_ms,
                apdu_retries,
                registered_network_port,
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
        // Builder-time validation only; params are stored for rebuildable retry.
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

    /// Clone the client role (fails with RuntimeError before start/after close).
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

    /// Clone the server role (fails with RuntimeError before start/after close).
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

    /// Actual announced IP and bound UDP port while the endpoint is active.
    /// Raises RuntimeError before startup publication and during/after teardown.
    fn local_address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.lifecycle.session.clone();
        crate::py_async::future_into_py(py, async move {
            let guard = inner.lock().await;
            guard
                .as_ref()
                .and_then(EndpointSession::bip_local_address)
                .map(|address| address.to_string())
                .ok_or_else(|| PyRuntimeError::new_err("endpoint is not active"))
        })
    }

    /// Bounded snapshot: liveness, identity, transport, leases, policy counts.
    ///
    /// Counts and kind labels only — no keys, certs, or payloads. Raises
    /// RuntimeError before start and after close, like the hub status.
    fn status<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.lifecycle.session.clone();
        let config = self.config.clone();
        crate::py_async::future_into_py(py, async move {
            let snapshot = {
                let guard = inner.lock().await;
                let session = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("endpoint not started"))?;
                let address = session
                    .bip_local_address()
                    .ok_or_else(|| PyRuntimeError::new_err("endpoint is not active"))?;
                let counters = session.policy_counters().await;
                let leases = session.active_leases();
                let running = session.is_running();
                (counters, leases, running, address)
            };
            // Lock released before touching Python.
            crate::py_async::attach(|py| {
                let dict = PyDict::new(py);
                dict.set_item("is_running", snapshot.2)?;
                dict.set_item("device_instance", config.identity.instance())?;
                dict.set_item("vendor_id", config.identity.vendor_id())?;
                dict.set_item("max_apdu", config.identity.max_apdu_length())?;
                dict.set_item("transport", "bip")?;
                dict.set_item("local_address", snapshot.3.to_string())?;
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

    /// Broadcast one I-Am consistent with the composed identity.
    ///
    /// Fails with BacnetError when not running or without an identity
    /// (identity is always composed here).
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
            "BipEndpoint(device={}, {}:{})",
            self.config.identity.instance(),
            self.config.interface,
            self.config.port
        )
    }
}
