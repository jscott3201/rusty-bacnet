//! B/IP Python endpoint owner: one UDP socket above both roles.
//!
//! A private lifecycle owner orders preparation, session startup and joined
//! teardown. The `client()` and `server()` handles are the standalone
//! `BACnetClient` and `BACnetServer` classes on the session's one socket;
//! the owner stops them.
//!
//! Builder-time config only: the constructor validates the single
//! `DeviceIdentity` (instance/vendor/APDU/segmentation/services/ports/UUID)
//! plus transport addressing. No post-start configuration mutation exists.
//! The default vendor identifier is 555 and flows through this single identity.

use std::net::Ipv4Addr;

use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_endpoint::session::SessionRole;
use bacnet_transport::bip::{BipMode, BipTransport};
use bacnet_transport::port::TransportPort;
use bacnet_types::data_link::DataLink;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::endpoint::common::{
    PendingObject, build_database, build_identity, build_pending_boxes, lifecycle_error,
    parse_device_uuid, parse_ipv4, parse_segmentation, pending_group, with_bip_port,
};
use crate::endpoint::lifecycle::Lifecycle;
use crate::endpoint::running::Running;
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
    apdu_timeout_ms: u64,
    apdu_retries: u8,
    registered_network_port: Option<u32>,
    min_request_interval_ms: u64,
    /// The B/IP mode `bbmd=` or `foreign_device=` asked for (#938).
    mode: BipMode,
    /// The B/IP multicast group `multicast=` asked for (#941).
    multicast: Option<bacnet_transport::bip::BipMulticast>,
    /// The full server's keywords (#2617).
    server: super::server_role::ServerKeywords,
    /// The source Audit Reporter keywords, if any (#2348).
    source_audit: Option<super::source_audit::SourceAuditKeywords>,
}

/// The running session's BBMD table reader, taken under the session lock
/// and used after it is released, so a read waiting on its answer never
/// holds up `close()`.
async fn bvlc_tables(
    session: &tokio::sync::RwLock<Option<Running>>,
) -> PyResult<bacnet_transport::bip::BvlcTableReader> {
    session
        .read()
        .await
        .as_ref()
        .and_then(|session| session.bvlc_tables())
        .ok_or_else(|| PyRuntimeError::new_err("endpoint not started"))
}

/// A table read's failure. One that `close()` cut off, or that met the
/// transport stopping, raises what a read after `close()` does (#2612).
fn table_read_error(error: bacnet_transport::bip::BvlcExchangeError) -> PyErr {
    match &error {
        bacnet_transport::bip::BvlcExchangeError::Transport(
            bacnet_types::error::Error::Transport(io),
        ) if io.kind() == std::io::ErrorKind::NotConnected => {
            PyRuntimeError::new_err("endpoint not started")
        }
        _ => crate::errors::bvlc_exchange_err(error),
    }
}

impl BipEndpointConfig {
    async fn prepare(self, objects: Vec<PendingObject>) -> PyResult<Running> {
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
        let mut session = builder
            .build_session_wrapped(crate::app_transport::AppTransport::from)
            .map_err(to_py_err)?;
        if let Some(source) = &config.source_audit {
            session = source.select(session);
        }
        Ok(Running::new(config.server.install(session)))
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
    lifecycle: Lifecycle<Running, PendingObject>,
    frames: std::sync::Arc<crate::frames::FrameSubscriptions>,
    config: BipEndpointConfig,
    /// The BBMD table reads started, for `read_bdt` and `read_fdt` (#2612).
    table_reads: super::table_reads::TableReads,
}

impl PyBipEndpoint {
    fn push_pending(&self, obj: PendingObject) -> PyResult<()> {
        super::common::register_pending(&self.lifecycle, &self.config.identity, obj)
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
    ///     segmentation: `Segmentation` override (default SEGMENTED_BOTH). The
    ///         Device states it, and the endpoint's server and client both
    ///         segment only as it says.
    ///     apdu_segment_timeout_ms: Keyword-only (#2617). The Device's
    ///         APDU_Segment_Timeout (default 5000), which the server and
    ///         `client()` use while they segment; positive when segmenting.
    ///     device_uuid: Optional 16-byte UUID (zeros allowed for BIP-only;
    ///         stored into DEVICE_UUID, single identity source).
    ///     queue_capacity: Bounded queue capacity (must be >0, default 256).
    ///         A full queue drops and counts what it has no room for
    ///         (`status()`'s `*_full_drops`); it never ends the session.
    ///     apdu_timeout_ms / apdu_retries: The client role's APDU timeout
    ///         and retries (default 6000 ms and 3).
    ///     read_work_limit: Keyword-only. The server's
    ///         ReadPropertyMultiple budget of result rows (must be >0,
    ///         default 256), `BACnetServer`'s `rpm_max_result_elements`: a
    ///         ReadProperty counts its own row plus, for a Group's
    ///         Present_Value, one per member property, and a read past it
    ///         is aborted with OUT_OF_RESOURCES.
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
    ///     mutation_policy: Keyword-only (default None). Who may change the
    ///         endpoint's objects (#2617): `"permissive"`, `"deny_all"` or a
    ///         callable, as `BACnetServer`'s keyword of the same name takes.
    ///         Without it the server is read-only: every write, create and
    ///         delete is denied, and reads, discovery and COV subscriptions
    ///         are answered.
    ///     cov_policy: Keyword-only (default None, the Rust defaults). The
    ///         server's COV policy, as `BACnetServer`'s keyword of the same
    ///         name reads it.
    ///     time_sync_policy: Keyword-only (default None: off). A dict read
    ///         as `BACnetServer`'s keyword of the same name reads it turns
    ///         TimeSynchronization and UTCTimeSynchronization on (`{}` takes
    ///         every valid request). Unlike `BACnetServer()`'s default,
    ///         time synchronization is off on an endpoint unless asked for,
    ///         whatever its `mutation_policy` (#2617).
    ///     source_audit_reporter / audit_recipient / audit_device_bindings:
    ///         Keyword-only (#2348). `source_audit_reporter` is the
    ///         `ObjectIdentifier` of an Audit Reporter added with
    ///         `add_audit_reporter`; the session reports through it the
    ///         ReinitializeDevice resets, backups and restores it carries
    ///         out, its server's `report_device_reset_local()` calls, and
    ///         the reads and writes of its `client()` (#2617). It needs
    ///         `audit_recipient`, the Device's Audit recipient (an
    ///         `AuditRecipientInput`), and a `mutation_policy` that decides
    ///         writes: none (read-only), `"deny_all"` or a callable, never
    ///         `"permissive"`. `audit_device_bindings` is a list of
    ///         `(ObjectIdentifier, "ipv4:port")` routes to recipient
    ///         Devices, or `"network:0xMAC[:one-station]"` for one behind a router, or a
    ///         `RoutedTarget` with no router, which can mark the address
    ///         `one_station=True` (#2371, #2738). A Reporter identifier of
    ///         another type, and other shapes construction can check, raise
    ///         ValueError here; a Reporter that wasn't added raises
    ///         BacnetError at start.
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
        device_uuid=None,
        queue_capacity=256,
        apdu_timeout_ms=6000,
        apdu_retries=3,
        registered_network_port=None,
        *,
        apdu_segment_timeout_ms=5000,
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
        mutation_policy=None,
        cov_policy=None,
        time_sync_policy=None,
        source_audit_reporter=None,
        audit_recipient=None,
        audit_device_bindings=None
    ))]
    fn new(
        py: Python<'_>,
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
        device_uuid: Option<Vec<u8>>,
        queue_capacity: usize,
        apdu_timeout_ms: u64,
        apdu_retries: u8,
        registered_network_port: Option<u32>,
        apdu_segment_timeout_ms: u64,
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
        mutation_policy: Option<&Bound<'_, PyAny>>,
        cov_policy: Option<&Bound<'_, PyDict>>,
        time_sync_policy: Option<&Bound<'_, PyDict>>,
        source_audit_reporter: Option<crate::types::PyObjectIdentifier>,
        audit_recipient: Option<&Bound<'_, PyAny>>,
        audit_device_bindings: Option<
            Vec<(
                crate::types::PyObjectIdentifier,
                super::source_audit::BindingAddress,
            )>,
        >,
    ) -> PyResult<Self> {
        let server = super::server_role::ServerKeywords::parse(
            mutation_policy,
            read_work_limit,
            cov_policy,
            time_sync_policy,
            on_reinitialize,
            after_reinitialize,
            reinit_password,
        )?;
        let mode = crate::bip_options::bip_mode("bip", bbmd, foreign_device)?;
        let interface_ip = crate::bip_options::interface_ipv4(py, interface)?;
        let multicast = crate::bip_options::bip_multicast(
            "bip",
            interface_ip,
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
        let min_request_interval_ms =
            crate::client::parse_min_request_interval_ms(min_request_interval_ms)?;
        let broadcast = parse_ipv4(broadcast_address, "broadcast_address")?;
        let source_audit = super::source_audit::SourceAuditKeywords::parse(
            source_audit_reporter,
            audit_recipient,
            audit_device_bindings,
            device_instance,
            super::source_audit::AuditLink::Bip(broadcast),
            server.decides_mutations(),
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
        let uuid = parse_device_uuid(device_uuid, false, "device_uuid")?;
        let identity = build_identity(
            device_instance,
            device_name,
            vendor_id,
            max_apdu,
            segmentation,
            apdu_segment_timeout_ms,
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
        let frames = std::sync::Arc::new(crate::frames::FrameSubscriptions::default());
        let observer = frames.clone();
        Ok(Self {
            lifecycle: Lifecycle::new().with_close_observer(move || observer.close()),
            frames,
            table_reads: super::table_reads::TableReads::default(),
            config: BipEndpointConfig {
                interface: interface_ip,
                port,
                broadcast,
                share_port_by_address,
                share_wildcard_port,
                identity,
                queue_capacity,
                apdu_timeout_ms,
                apdu_retries,
                registered_network_port,
                min_request_interval_ms,
                mode,
                multicast,
                server,
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
        self.push_pending(pending_group(instance, name, members))
    }

    /// Start after lifecycle admission. A second explicit start fails.
    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        crate::logging_bridge::refresh_at_start(py);
        let lifecycle = self.lifecycle.clone();
        let config = self.config.clone();
        config.server.bind_event_loop(py)?;
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
        config.server.bind_event_loop(py)?;
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

    /// Subscribe to raw frames on the endpoint's existing socket, including
    /// malformed received datagrams. A full queue drops new frames; the next
    /// frame queued reports the gap in `Frame.missed_before`. Close ends
    /// every subscription, waking pending reads.
    #[pyo3(signature = (capacity=1024))]
    fn frames<'py>(&self, py: Python<'py>, capacity: usize) -> PyResult<Bound<'py, PyAny>> {
        let capacity = crate::frames::capacity(capacity)?;
        let clock = crate::frames::Clock::get(py)?;
        let lifecycle = self.lifecycle.clone();
        let frames = self.frames.clone();
        crate::py_async::future_into_py(py, async move {
            let guard = lifecycle.session.read().await;
            let session = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("endpoint not started"))?;
            let receiver = session
                .frames(capacity)
                .ok_or_else(|| PyRuntimeError::new_err("endpoint not started"))?;
            let address = session
                .bip_local_address()
                .and_then(|socket| {
                    crate::frames::local_address(DataLink::Bip, &[], Some(socket.into()))
                })
                .ok_or_else(|| PyRuntimeError::new_err("endpoint not started"))?;
            Ok(frames.subscribe(receiver, address, clock))
        })
    }

    /// Subscribe to a record of every I-Am the endpoint's socket receives,
    /// as `BACnetClient.i_am_arrivals` does, leaving out and counting its
    /// own I-Am come back to it: from its own address, of its own device
    /// instance. Close ends every subscription, waking pending reads.
    #[pyo3(signature = (capacity=1024))]
    fn i_am_arrivals<'py>(&self, py: Python<'py>, capacity: usize) -> PyResult<Bound<'py, PyAny>> {
        let capacity = crate::frames::capacity(capacity)?;
        let clock = crate::frames::Clock::get(py)?;
        let lifecycle = self.lifecycle.clone();
        let frames = self.frames.clone();
        let instance = self.config.identity.instance();
        crate::py_async::future_into_py(py, async move {
            let guard = lifecycle.session.read().await;
            let not_started = || PyRuntimeError::new_err("endpoint not started");
            let session = guard.as_ref().ok_or_else(not_started)?;
            let receiver = session.frames(capacity).ok_or_else(not_started)?;
            let own = session.bip_local_address().ok_or_else(not_started)?;
            Ok(crate::i_am_arrivals::subscribe(
                &frames,
                receiver,
                Some((own.into(), instance)),
                clock,
            ))
        })
    }

    /// The `BACnetClient` on this endpoint's socket (#2617): the standalone
    /// class, with all its calls, on the endpoint's one address. While it
    /// runs, each call returns the same object; once its `stop()` has
    /// finished, a fresh one. Its `stop()` ends the client role alone, and
    /// the endpoint's close stops it too. Raises RuntimeError before start,
    /// after close, and while the last client is still stopping or its stop
    /// failed (naming the cause). With `source_audit_reporter`, its reads
    /// and writes are audited (#2617).
    fn client<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let client = super::running::client(py, self.lifecycle.session.clone(), "bip")?;
        crate::py_async::future_into_py(py, client)
    }

    /// The `BACnetServer` the endpoint's session runs on its socket
    /// (#2617): the standalone class, with its calls, on the endpoint's one
    /// address and its objects. Each call returns a new handle on the one
    /// server. Its `start()` and `stop()` raise RuntimeError: the endpoint
    /// starts it, and the endpoint's close stops it, after which its calls
    /// raise RuntimeError too. RuntimeError before start and after close.
    fn server<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let server = super::running::server(
            self.lifecycle.session.clone(),
            "bip",
            self.config.identity.instance(),
        );
        crate::py_async::future_into_py(py, server)
    }

    /// Actual announced IP and bound UDP port while the endpoint is active.
    /// Raises RuntimeError before startup publication and during/after teardown.
    fn local_address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.lifecycle.session.clone();
        crate::py_async::future_into_py(py, async move {
            let guard = inner.read().await;
            guard
                .as_ref()
                .and_then(|session| session.bip_local_address())
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
                let guard = inner.read().await;
                let session = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("endpoint not started"))?;
                let address = session
                    .bip_local_address()
                    .ok_or_else(|| PyRuntimeError::new_err("endpoint is not active"))?;
                let counters = session.policy_counters().await;
                let leases = session.active_leases();
                let running = session.is_running();
                let seat = session.seat_counts();
                (counters, leases, running, address, seat)
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
                dict.set_item("unconfirmed_full_drops", snapshot.0.unconfirmed_full_drops)?;
                dict.set_item("confirmed_full_drops", snapshot.0.confirmed_full_drops)?;
                dict.set_item("answer_full_drops", snapshot.0.answer_full_drops)?;
                dict.set_item("policy_full_drops", snapshot.0.policy_full_drops)?;
                snapshot.4.add_to(&dict)?;
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
                let guard = inner.read().await;
                guard
                    .as_ref()
                    .and_then(|session| session.foreign_registration())
                    .ok_or_else(|| PyRuntimeError::new_err("endpoint not started"))?
            };
            crate::registration_dicts::registration_dict(registration)
        })
    }

    /// Read the Broadcast Distribution Table of the BBMD at `address`
    /// (`"ip"` or `"ip:port"`) through this endpoint's own socket, beside the
    /// roles' traffic (#2612), as `BACnetClient.read_bdt` does: the same
    /// `("ip:port", "mask")` rows and the same errors. Raises RuntimeError
    /// before start and after close, and for a read that close cuts off.
    fn read_bdt<'py>(&self, py: Python<'py>, address: String) -> PyResult<Bound<'py, PyAny>> {
        let target = crate::bip_options::bip_mac(&address, "address")?;
        let inner = self.lifecycle.session.clone();
        // A cancelled read gives its request up before the same read
        // starts again (#2612).
        let key = (super::table_reads::Table::Bdt, target);
        let (id, give_up) = self.table_reads.begin(py)?;
        let reads = give_up.clone();
        let read = async move {
            let rows = bvlc_tables(&inner)
                .await?
                .with_give_up(&reads)
                .read_bdt(&target)
                .await
                .map_err(table_read_error)?;
            Ok(rows
                .iter()
                .map(crate::bip_options::bdt_row)
                .collect::<Vec<_>>())
        };
        let on_done = self.table_reads.on_done(key, id, give_up.clone());
        let future = crate::py_async::future_into_py_on_done(py, read, on_done)?;
        self.table_reads.started(key, id, &future, give_up);
        Ok(future)
    }

    /// Read the Foreign Device Table of the BBMD at `address` through this
    /// endpoint's own socket (#2612), as `BACnetClient.read_fdt` does: a
    /// dict per entry with `address`, `ttl` and `seconds_remaining`, and the
    /// same errors. Raises RuntimeError before start and after close, and
    /// for a read that close cuts off.
    fn read_fdt<'py>(&self, py: Python<'py>, address: String) -> PyResult<Bound<'py, PyAny>> {
        let target = crate::bip_options::bip_mac(&address, "address")?;
        let inner = self.lifecycle.session.clone();
        let key = (super::table_reads::Table::Fdt, target);
        let (id, give_up) = self.table_reads.begin(py)?;
        let reads = give_up.clone();
        let read = async move {
            let entries = bvlc_tables(&inner)
                .await?
                .with_give_up(&reads)
                .read_fdt(&target)
                .await
                .map_err(table_read_error)?;
            crate::py_async::attach(|py| crate::bip_options::fdt_rows(py, &entries))
        };
        let on_done = self.table_reads.on_done(key, id, give_up.clone());
        let future = crate::py_async::future_into_py_on_done(py, read, on_done)?;
        self.table_reads.started(key, id, &future, give_up);
        Ok(future)
    }

    /// Broadcast one I-Am consistent with the composed identity.
    ///
    /// Fails with BacnetError when not running or without an identity
    /// (identity is always composed here).
    fn broadcast_i_am<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.lifecycle.session.clone();
        let future = async move {
            let guard = inner.read().await;
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
