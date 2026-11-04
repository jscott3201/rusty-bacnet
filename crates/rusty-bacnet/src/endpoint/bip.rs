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
use bacnet_transport::bip::{BipMode, BipTransport};
use bacnet_transport::port::TransportPort;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::endpoint::common::{
    PendingObject, build_database, build_identity, build_pending_boxes, lifecycle_error,
    parse_device_uuid, parse_ipv4, parse_read_work_limit, parse_segmentation, parse_services,
    pending_group, with_bip_port,
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
    /// See `BipTransport::set_share_wildcard_port` (#2366).
    share_wildcard_port: bool,
    identity: DeviceIdentity,
    queue_capacity: usize,
    read_work_limit: usize,
    apdu_timeout_ms: u64,
    apdu_retries: u8,
    registered_network_port: Option<u32>,
    min_request_interval_ms: u64,
    /// The B/IP mode `bbmd=` or `foreign_device=` asked for (#938).
    mode: BipMode,
    /// The B/IP multicast group `multicast=` asked for (#941).
    multicast: Option<bacnet_transport::bip::BipMulticast>,
    reinitialize: crate::reinitialize::EndpointReinitialize,
    /// The `device_write_policy` callable, if any (#2033).
    device_writes: crate::mutation_policy::DeviceWritePolicy,
    /// The source Audit Reporter keywords, if any (#2348).
    source_audit: Option<super::source_audit::SourceAuditKeywords>,
}

type BipSession = EndpointSession<BipTransport>;

impl BipEndpointConfig {
    async fn prepare(self, objects: Vec<PendingObject>) -> PyResult<BipSession> {
        let config = self;
        // Fail fast before building objects: probe the port and the
        // interface, so conflicts preserve pending for retry. The real bind
        // stays authoritative (TOCTOU residual: a race loser still restores
        // via the owner). The probe binds exactly what the transport will,
        // shared by address (#1538) or on the wildcard address, claimed or
        // shared (#2366).
        let mut probe = BipTransport::new(config.interface, config.port, config.broadcast);
        probe.set_share_port_by_address(config.share_port_by_address);
        probe.set_share_wildcard_port(config.share_wildcard_port);
        probe.check_bind().map_err(to_py_err)?;
        let boxes = build_pending_boxes(&objects)?;
        let mut db = build_database(&config.identity, boxes)?;
        if let Some(source) = &config.source_audit {
            source.provision(&config.identity, &mut db)?;
        }
        let mut builder = BipEndpointBuilder::new(config.interface, config.port, config.broadcast)
            .share_port_by_address(config.share_port_by_address)
            .share_wildcard_port(config.share_wildcard_port)
            .role(SessionRole::Both)
            .queue_capacity(config.queue_capacity)
            .read_work_limit(config.read_work_limit)
            .client_timers(config.apdu_timeout_ms, config.apdu_retries)
            .min_request_interval_ms(config.min_request_interval_ms)
            .mode(config.mode.clone())
            .multicast(config.multicast)
            .database(db)
            .device(config.identity.clone());
        if let Some(instance) = config.registered_network_port {
            let oid = bacnet_types::primitives::ObjectIdentifier::new(
                bacnet_types::enums::ObjectType::NETWORK_PORT,
                instance,
            )
            .map_err(to_py_err)?;
            builder = builder.registered_network_port(oid);
        }
        if let Some(source) = &config.source_audit {
            builder = source.bind(builder);
        }
        let mut session = builder.build_session().map_err(to_py_err)?;
        if let Some(source) = &config.source_audit {
            session = session.with_source_audit_reporter(source.reporter());
        }
        let session = config.device_writes.install(session);
        Ok(config.reinitialize.install(session))
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

super::builders::endpoint_builders!(PyBipEndpoint);

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
    ///         B/IP transport, in any one mode; None leaves the declaration unbound.
    ///     max_apdu: Raw receive ceiling, at least 50 bytes, resolved against
    ///         the B/IP link before Device construction. Request headers use
    ///         the largest supported code that does not exceed it.
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
    ///         a nonzero port. Broadcasts and unicast are still handled in
    ///         the order they arrived.
    ///     share_wildcard_port: Keyword-only (default False). Share an
    ///         explicitly requested port with other sockets on this host
    ///         that bind it the same way. By default the endpoint claims
    ///         its port, and starting fails when another socket holds it;
    ///         with sharing, unicast to the port reaches only one of the
    ///         sockets on it.
    ///     min_request_interval_ms: Keyword-only (default 0, no pacing; at
    ///         most 3600000). Least time between the client role's confirmed
    ///         requests to one destination, as for `BACnetClient`.
    ///     bbmd / foreign_device: Keyword-only, at most one: a `Bbmd` makes
    ///         the endpoint a BBMD, a `ForeignDevice` registers it with one
    ///         (#938). A ValueError if both are given.
    ///     on_reinitialize / after_reinitialize / reinit_password:
    ///         Keyword-only. Execute ReinitializeDevice, as `BACnetServer`'s
    ///         keywords of the same names do (#1566).
    ///     device_write_policy: Keyword-only (default None). A callable that
    ///         decides each WriteProperty to the local Device's Description
    ///         or installed Audit recipient (#2033). It gets a
    ///         `MutationContext` and returns a bool, with the fail-closed
    ///         rules of `BACnetServer`'s `mutation_policy` callable. Without
    ///         it the endpoint executes no WriteProperty.
    ///     source_audit_reporter / audit_recipient / audit_device_bindings:
    ///         Keyword-only (#2348). `source_audit_reporter` is the
    ///         `ObjectIdentifier` of an Audit Reporter added with
    ///         `add_audit_reporter`; the session reports through it the
    ///         ReinitializeDevice resets, backups and restores it carries
    ///         out, `report_device_reset_local()` calls, and the client
    ///         role's reads. It needs `device_write_policy` and
    ///         `audit_recipient`, the Device's Audit recipient (an
    ///         `AuditRecipientInput`). `audit_device_bindings` is a list of
    ///         `(ObjectIdentifier, "ipv4:port")` routes to recipient
    ///         Devices. A Reporter identifier of another type, and other
    ///         shapes construction can check, raise ValueError here; a
    ///         Reporter that wasn't added raises BacnetError at start.
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
        share_wildcard_port=false,
        min_request_interval_ms=0,
        bbmd=None,
        foreign_device=None,
        multicast=None,
        on_reinitialize=None,
        after_reinitialize=None,
        reinit_password=None,
        device_write_policy=None,
        source_audit_reporter=None,
        audit_recipient=None,
        audit_device_bindings=None
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
        share_wildcard_port: bool,
        min_request_interval_ms: u64,
        bbmd: Option<PyRef<'_, crate::bip_options::PyBbmd>>,
        foreign_device: Option<PyRef<'_, crate::bip_options::PyForeignDevice>>,
        multicast: Option<PyRef<'_, crate::bip_options::PyMulticast>>,
        on_reinitialize: Option<&Bound<'_, PyAny>>,
        after_reinitialize: Option<&Bound<'_, PyAny>>,
        reinit_password: Option<String>,
        device_write_policy: Option<&Bound<'_, PyAny>>,
        source_audit_reporter: Option<crate::types::PyObjectIdentifier>,
        audit_recipient: Option<&Bound<'_, PyAny>>,
        audit_device_bindings: Option<Vec<(crate::types::PyObjectIdentifier, String)>>,
    ) -> PyResult<Self> {
        let reinitialize = crate::reinitialize::EndpointReinitialize::parse(
            on_reinitialize,
            after_reinitialize,
            reinit_password,
        )?;
        let device_writes = crate::mutation_policy::DeviceWritePolicy::parse(device_write_policy)?;
        let mode = crate::bip_options::bip_mode("bip", bbmd, foreign_device)?;
        let multicast = crate::bip_options::bip_multicast(
            "bip",
            interface,
            port,
            share_port_by_address,
            &mode,
            multicast,
        )?;
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
        let source_audit = super::source_audit::SourceAuditKeywords::parse(
            source_audit_reporter,
            audit_recipient,
            audit_device_bindings,
            device_instance,
            broadcast,
            device_writes.enabled(),
        )?;
        if let Some(selected) = registered_network_port
            && (!(1..=255).contains(&selected)
                || selected != network_port_instance
                || interface_ip.is_unspecified()
                || interface_ip.is_multicast()
                || interface_ip.is_broadcast()
                || interface_ip == broadcast)
        {
            return Err(PyValueError::new_err(
                "registered_network_port must select the declared 1..255 port on a concrete unicast interface",
            ));
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
        )?
        .resolve(BipTransport::new(interface_ip, port, broadcast).local_receive_apdu_capacity())
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
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
                share_wildcard_port,
                identity,
                queue_capacity,
                read_work_limit,
                apdu_timeout_ms,
                apdu_retries,
                registered_network_port,
                min_request_interval_ms,
                mode,
                multicast,
                reinitialize,
                device_writes,
                source_audit,
            },
        })
    }

    /// Test seam: pending registration count (drained by successful start;
    /// restored on failed/cancelled start).
    #[doc(hidden)]
    fn _pending_registration_count(&self) -> PyResult<usize> {
        Ok(self.lifecycle.pending_count())
    }

    /// Add an Audit Reporter object (before start), for
    /// `source_audit_reporter` to select (#2348).
    ///
    /// `audit_level` ("none", "audit_config" or "audit_all") and
    /// `auditable_operations` (bit n for AuditOperation n) are required, as
    /// on `BACnetServer.configure_audit_reporters`, and filter what the
    /// session reports. Peers can't change them through an endpoint. A
    /// Reporter that `source_audit_reporter` doesn't select records nothing.
    #[pyo3(signature = (
        instance, name, *,
        audit_level, auditable_operations, issue_confirmed_notifications=false
    ))]
    fn add_audit_reporter(
        &self,
        instance: u32,
        name: &str,
        audit_level: &str,
        auditable_operations: &Bound<'_, PyAny>,
        issue_confirmed_notifications: bool,
    ) -> PyResult<()> {
        use crate::server::{reporter_audit_level, reporter_auditable_operations};
        let pending = PendingObject::AuditReporter {
            instance,
            name: name.to_string(),
            level: reporter_audit_level(audit_level)?,
            operations: reporter_auditable_operations(auditable_operations)?,
            confirmed: issue_confirmed_notifications,
        };
        pending.build()?;
        self.push_pending(pending)
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
        config.reinitialize.bind_event_loop(py)?;
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
        crate::logging_bridge::refresh_at_start(py);
        let self_ref = slf.clone().unbind();
        let (lifecycle, config) = {
            let borrowed = slf.borrow();
            (borrowed.lifecycle.clone(), borrowed.config.clone())
        };
        config.reinitialize.bind_event_loop(py)?;
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

    /// How the endpoint's foreign-device registration is going (#938), as
    /// `BACnetClient.foreign_registration()` reports it. Sends nothing.
    /// Raises RuntimeError before start and after close.
    fn foreign_registration<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.lifecycle.session.clone();
        crate::py_async::future_into_py(py, async move {
            let registration = {
                let guard = inner.lock().await;
                guard
                    .as_ref()
                    .and_then(EndpointSession::foreign_registration)
                    .ok_or_else(|| PyRuntimeError::new_err("endpoint not started"))?
            };
            crate::bip_options::registration_dict(registration)
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
