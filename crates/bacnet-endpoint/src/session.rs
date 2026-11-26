//! Single-owner endpoint session: one ingress + shared coordinator.
//!
//! Lifecycle lives here ONLY: the session's client and server stop with it,
//! and neither can stop the link.
//!
//! # Lifecycle
//!
//! [`EndpointSession::new`] owns `transport` without starting it. The owner
//! drives [`start`](EndpointSession::start) once, then
//! [`stop`](EndpointSession::stop) once; both take `&mut self` so only the
//! owner can drive them. `Drop` aborts dispatch/ingress/role work without
//! orphaning, but `stop()` remains the graceful path that joins termination
//! and reports the [`SessionExit`].
//!
//! # Cancellation and drop
//!
//! The session's client ([`client`](EndpointSession::client)) is a
//! [`BACnetClient`](bacnet_client::client::BACnetClient): a request whose
//! caller goes away is cancelled and its invoke ID released, as on any
//! client. A source-audited request is the session's once it is admitted,
//! before anything is sent, so dropping its caller does not cancel it.
//! `stop()` stops the server first, whose own stop settles its
//! objects' saves, or hands that to a task while the database is held (see
//! [`stop`](EndpointSession::stop)), then seals admission, joins dispatch
//! and owned workers, and stops ingress. Undelivered audit records may be
//! lost on shutdown. A client kept past the stop, or past the session's
//! drop, fails each call. A drop ends the link too, the client still alive
//! or not (#2617): it aborts the transport, which lets go of its sockets
//! without a goodbye and without waiting.
//!
//! The session keeps its object database through `stop()` and lets go of it
//! when it drops; a start refused once the link has started hands it back
//! instead ([`start`](EndpointSession::start), #2766). Every holder of the
//! database (the session, its server, the source Audit runtime and the
//! session's tasks) lets go through
//! [`drop_database_off_runtime`], so whichever goes last, in async code the
//! objects and any durable saves they wait for drop on Tokio's blocking pool,
//! not on a runtime worker (#1561).
//!
//! ```compile_fail,E0596
//! // Lifecycle is owner-exclusive: `stop` takes `&mut self`, so a shared
//! // borrow cannot drive shutdown.
//! # use bacnet_transport::loopback::LoopbackTransport;
//! # use bacnet_endpoint::session::EndpointSession;
//! # fn forbidden(session: &EndpointSession<LoopbackTransport>) {
//! session.stop();
//! # }
//! ```
//!
//! # Thread safety
//!
//! [`EndpointSession`] is `Send` when its transport is `Send`: the dispatch
//! task, channels, and shared counters are all `Send`-owned. Shared `&self`
//! borrows (`client`, `server`, counters, `broadcast_i_am`) are safe because
//! lifecycle mutation is owner-exclusive (`&mut`) or via atomics and short
//! synchronous locks.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use bacnet_client::client::ClientConfig;
use bacnet_encoding::apdu::Apdu;
use bacnet_endpoint_core::coordinator::{
    AdmissionOutcome, CoordinatorError, LeaseOwner, OutboundTransactionCoordinator,
};
use bacnet_endpoint_core::endpoint_ingress::{EndpointIngress, PolicyOutcome, PolicyReason};
use bacnet_network::layer::ReceivedApdu;
use bacnet_network::network_number::LocalNetworkNumber;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::{self, Error, Refusal};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::sync::MutexExt;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio::task::JoinHandle;

#[path = "source_profile.rs"]
mod source_profile;
pub(crate) use source_profile::{RoutedBindings, check_link_bindings};
#[path = "session_seat.rs"]
mod seat;
#[path = "session_server_seat.rs"]
mod server_seat;
pub use server_seat::read_only_server;
#[path = "source_reporter.rs"]
pub(crate) mod source_reporter;
#[path = "session_start.rs"]
mod startup;

mod frame_observation;
#[path = "registered_port.rs"]
mod registered_port;

/// The refusal the link builders and [`EndpointSession::start`] return,
/// which hands back the object database (#2766). It is the server
/// builders' type, re-exported so a caller need not name `bacnet_server`.
pub use bacnet_server::server::StartError;
use bacnet_server::server::{
    __endpoint_NotificationTransactions as NotificationTransactions,
    __endpoint_start_refused as start_refused, drop_database_off_runtime,
};

/// How many runtime turns a refused start waits, after its cleanup, for
/// work it began to let go of the database (#2766).
const HAND_BACK_TURNS: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Ready = 0,
    Running = 1,
    Stopped = 2,
    Stopping = 3,
}

/// Which roles the session composes.
///
/// Chosen at [`EndpointSession::new`] time (or via the endpoint builders'
/// `role()` setter) and fixed for the session lifetime. It says what the
/// node serves and declares:
///
/// - [`ClientOnly`](SessionRole::ClientOnly): no server, and the server
///   role's accessors are `None`. A request that reaches the session's own
///   queue is counted (`no_server_role`) and its one-use reply sender
///   released without sending; while a [`client`](EndpointSession::client)
///   is attached, a confirmed request is that client's to answer.
/// - [`ServerOnly`](SessionRole::ServerOnly): the server role is the full
///   `server()` the session builds at start, the read-only one
///   ([`read_only_server`]) unless
///   [`with_server`](EndpointSession::with_server) gave it a builder
///   (#2617). Its Device lists no client services, and a source Audit
///   Reporter is refused.
/// - [`Both`](SessionRole::Both): that server, whose Device also lists the
///   services the node's client executes.
///
/// [`client`](EndpointSession::client) gives a client on a running session
/// of any role, above the one shared coordinator.
///
/// The role also decides how the client and the server share the session's
/// 256 invoke IDs (#2617). Each role the session runs keeps a reserve the
/// other can't take: [`REQUESTER_RESERVE`] (128) for the client on a
/// `ClientOnly` or `Both` session, [`NOTIFICATION_RESERVE`] (64) for the
/// server's confirmed notifications and its runs' requests on a
/// `ServerOnly` or `Both` session. Beyond its reserve either borrows what
/// the other's doesn't keep, so on a one-role session that role may take
/// the whole pool, and with both the client holds at most 192 and the
/// server 128. A client request refused an ID fails at once; the server's
/// work waits for one.
///
/// [`REQUESTER_RESERVE`]: bacnet_endpoint_core::coordinator::REQUESTER_RESERVE
/// [`NOTIFICATION_RESERVE`]: bacnet_endpoint_core::coordinator::NOTIFICATION_RESERVE
///
/// A session with a server role needs a Device identity
/// ([`device`](EndpointSession::device)): its start refuses one without,
/// before the link starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionRole {
    /// No server: the session gives a client and serves nothing.
    ClientOnly,
    /// Server role only: a full server ([`EndpointSession::with_server`],
    /// or [`read_only_server`] without one).
    ServerOnly,
    /// Both sibling roles above one ingress/coordinator.
    Both,
}

impl SessionRole {
    /// The invoke IDs the session's pool keeps for the client and for the
    /// server: each role the session runs keeps its reserve.
    fn invoke_id_reserves(self) -> (usize, usize) {
        use bacnet_endpoint_core::coordinator::{NOTIFICATION_RESERVE, REQUESTER_RESERVE};
        match self {
            Self::ClientOnly => (REQUESTER_RESERVE, 0),
            Self::ServerOnly => (0, NOTIFICATION_RESERVE),
            Self::Both => (REQUESTER_RESERVE, NOTIFICATION_RESERVE),
        }
    }
}

/// Session tuning (bounded queues, client timers).
///
/// Typed at the public boundary: every field is validated where it matters
/// (`queue_capacity == 0` or a `min_request_interval_ms` past an hour fails
/// [`EndpointSession::new`]; APDU/timer values flow into the client role
/// config unchanged).
///
/// ```
/// use bacnet_endpoint::session::SessionConfig;
///
/// let config = SessionConfig::default();
/// assert_eq!(config.queue_capacity, 256);
/// assert_eq!(config.max_apdu_length, 480);
/// assert_eq!(config.min_request_interval_ms, 0);
/// ```
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// Bounded capacity for each ingress queue (requests, answers, policy
    /// outcomes) and the egress channel (default 256, as deep as the
    /// network layer's own receive queue in front of them).
    ///
    /// What finds a queue full is dropped and counted by class in
    /// [`policy_counters`](EndpointSession::policy_counters); a full queue
    /// never ends the session (#2635). Each queue keeps a reserve, a quarter
    /// of it and at least one slot from a capacity of two up: unconfirmed
    /// requests may not take the request queue's, which stays for confirmed
    /// requests, and answers no lease holds may not take the answer
    /// queue's, which stays for answers to the session's own requests. A
    /// capacity of 1 keeps no reserve, so there one Who-Is can shut out a
    /// confirmed request. Under a flood of stray answers, then, the
    /// session's own answers keep 64 slots at the default capacity, and 4
    /// at 16. A [`client`](EndpointSession::client)'s answers don't use
    /// this queue but its seat's, which has tiers of its own.
    ///
    /// A queued item takes a slot of at most 192 octets (176 to 184 on a
    /// 64-bit target today) plus its NPDU or APDU (B/IP copies each NPDU to
    /// its own size; at most about 1.5 KB). At the default the four queues
    /// hold at most about 1.7 MB together, and a request queue full of I-Ams
    /// about 50 KB. Tokio allocates slots in blocks of 32 as a queue first
    /// fills, so an idle session holds next to none of it.
    ///
    /// A request that finds no room gets no answer, and an answer that finds
    /// none ends its request by timeout: peers and clients with long APDU
    /// timeouts or no retries pay one timeout for each. Raise this capacity,
    /// and [`apdu_retries`](Self::apdu_retries), where bursts are expected.
    ///
    /// Must be greater than zero; [`EndpointSession::new`] returns
    /// [`Error::InvalidConfig`] otherwise.
    pub queue_capacity: usize,
    /// The node's APDU timeout in milliseconds (default 6000): how long the
    /// client waits for an answer before it sends a request again,
    /// and, with a full server ([`with_server`](EndpointSession::with_server)), how long
    /// its confirmed notifications wait, and the Device's APDU_Timeout
    /// (Clause 12.11.28).
    pub apdu_timeout_ms: u64,
    /// The node's APDU retries (default 3): how many times the client
    /// sends a request again, and, with a full server
    /// ([`with_server`](EndpointSession::with_server)), its confirmed
    /// notifications, and the Device's Number_Of_APDU_Retries
    /// (Clause 12.11.29).
    pub apdu_retries: u8,
    /// Client max APDU length.
    ///
    /// Standalone default is 480. Composing a
    /// [`DeviceIdentity`](crate::identity::DeviceIdentity) via
    /// [`device`](EndpointSession::device) overrides this at start with the
    /// identity's ceiling resolved against the transport's stable local
    /// receive capacity (480 for standard MS/TP, 1476 for extended MS/TP).
    pub max_apdu_length: u16,
    /// Least time, in milliseconds, between the client role's confirmed
    /// requests to one destination (default 0: no pacing), measured as
    /// [`ClientConfig::min_request_interval_ms`] says for `BACnetClient`
    /// (#1542).
    ///
    /// Only new confirmed requests wait: a retry keeps its request's turn,
    /// and replies, notifications and unconfirmed requests go at once.
    /// Stopping or dropping the session ends a wait at once with the
    /// shutdown error. At
    /// most [`MAX_MIN_REQUEST_INTERVAL_MS`](bacnet_client::client::MAX_MIN_REQUEST_INTERVAL_MS),
    /// an hour; [`EndpointSession::new`] returns [`Error::InvalidConfig`]
    /// past it.
    pub min_request_interval_ms: u64,
    /// Whether the client role takes a Complex-ACK for a service
    /// acknowledged with a Simple-ACK as a success (default `false`), as
    /// [`ClientConfig::accept_complex_ack_for_simple_services`] says for
    /// `BACnetClient` (#2721).
    pub accept_complex_ack_for_simple_services: bool,
}

impl SessionConfig {
    /// The checks [`EndpointSession::new`] makes, without a transport, so a
    /// builder that dials can make them before it does (#1946).
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.queue_capacity == 0 {
            return Err(Error::invalid_config(
                "endpoint session queue capacity must be greater than zero",
            ));
        }
        let interval = self.min_request_interval_ms;
        if interval > bacnet_client::client::MAX_MIN_REQUEST_INTERVAL_MS {
            return Err(Error::InvalidConfig(
                Refusal::new("endpoint session min-request-interval {} ms is more than {} ms")
                    .with_values(interval, bacnet_client::client::MAX_MIN_REQUEST_INTERVAL_MS),
            ));
        }
        Ok(())
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 256,
            apdu_timeout_ms: 6_000,
            apdu_retries: 3,
            max_apdu_length: 480,
            min_request_interval_ms: 0,
            accept_complex_ack_for_simple_services: false,
        }
    }
}

/// Snapshot of policy-outcome ownership (session-owned, bounded).
///
/// The session counts every classifier/policy outcome instead of dropping it
/// silently, and ingress counts by class what a full queue had no room for
/// (#2635). Sampled via [`policy_counters`](EndpointSession::policy_counters).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PolicyCountersSnapshot {
    /// Ingress classifier outcomes owned by the session.
    pub ingress_policy: u64,
    /// Inbound requests with no server role (client-only).
    pub no_server_role: u64,
    /// Answers that matched a lease whose role was not there to take them,
    /// such as one for a client that has just gone; the lease is released.
    /// The session's own queue sees an answer only when no seat takes it.
    pub no_client_role: u64,
    /// Terminal responses no lease claimed (unknown/peer/service/policy).
    pub unclaimed_terminal: u64,
    /// Unconfirmed requests past their share of the full request queue
    /// (see [`SessionConfig::queue_capacity`]).
    pub unconfirmed_full_drops: u64,
    /// Confirmed requests for the session's own roles that its full request
    /// queue had no room for; none was answered, and the requester's retry
    /// is its recovery. One for the client (its COV notification, or any
    /// on a session with no server role) that the client's seat refuses is
    /// counted in [`client_seat_counters`](EndpointSession::client_seat_counters)
    /// instead.
    pub confirmed_full_drops: u64,
    /// Answers and segments the full answer queue had no room for; an
    /// answer to the session's own request then ends by its timeout.
    pub answer_full_drops: u64,
    /// Refused APDUs (malformed, unknown PDU type, a confirmed request from
    /// a group address) the full policy queue had no room for.
    pub policy_full_drops: u64,
}

struct PolicyCounters {
    ingress_policy: u64,
    no_server_role: u64,
    no_client_role: u64,
    unclaimed_terminal: u64,
}

struct SessionShared {
    /// A short synchronous lock, never held across an await, so sampling
    /// never waits on the dispatch task. A Tokio mutex here went to a
    /// sampler that was not polled again, and the dispatch task then waited
    /// for it for good (#2759).
    counters: std::sync::Mutex<PolicyCounters>,
}

impl SessionShared {
    fn counters(&self) -> std::sync::MutexGuard<'_, PolicyCounters> {
        self.counters.locked()
    }
}

/// One owned single-device endpoint session.
///
/// Owns ONE [`EndpointIngress`], the shared [`OutboundTransactionCoordinator`],
/// its server and its client's seat, policy-outcome ownership, timers,
/// egress and termination. Start-once/stop-once; `Drop` aborts without
/// orphaning.
///
/// A [`DeviceIdentity`](crate::identity::DeviceIdentity) is the single
/// source for I-Am + Device readback + role limits when composed via
/// [`device`](Self::device); a session with a server role needs one. A
/// client-only session without an identity keeps the `SessionConfig` 480
/// default untouched.
///
/// ```no_run
/// use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
/// use bacnet_endpoint::DeviceIdentity;
/// use bacnet_transport::loopback::LoopbackTransport;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), bacnet_types::error::Error> {
/// let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
/// let mut session =
///     EndpointSession::new(transport, SessionRole::Both, SessionConfig::default())?
///         .device(DeviceIdentity::new(1001, 42)?);
/// session.start().await?;
/// assert!(session.is_running());
/// session.stop().await?;
/// # Ok(())
/// # }
/// ```
///
/// `T` defaults to [`AnyTransport`](bacnet_transport::any::AnyTransport), so
/// `EndpointSession` alone names a session over any built-in transport;
/// `EndpointSession<BipTransport>` and the like name one over a concrete
/// transport, as the builders return.
pub struct EndpointSession<T: TransportPort + 'static = bacnet_transport::any::AnyTransport> {
    shared: Arc<SessionShared>,
    coordinator: Arc<OutboundTransactionCoordinator>,
    ingress: Option<EndpointIngress<T>>,
    /// The source Audit's pool of notifications on a session with no
    /// server; beside a server, its pool is the server's.
    notifications: Option<Arc<NotificationTransactions>>,
    /// The full server's builder until start ([`Self::with_server`]), then
    /// the server until stop (#2617), and the error its stop gave.
    server_builder: Option<bacnet_server::server::ServerBuilder<T>>,
    server: Option<bacnet_server::server::BACnetServer<T>>,
    server_stop_error: Option<Error>,
    /// Held while a start that built the server is unfinished: the server
    /// takes no request until it goes, at the start's end or at stop.
    server_gate: Option<oneshot::Sender<()>>,
    dispatch_task: Option<JoinHandle<SessionExit>>,
    network_number_task: Option<JoinHandle<()>>,
    /// The checks of the routed source Audit bindings' routers (#2831).
    router_task: Option<JoinHandle<()>>,
    cancel_tx: Option<oneshot::Sender<()>>,
    lifecycle: AtomicU8,
    role: SessionRole,
    #[allow(dead_code)]
    config: SessionConfig,
    client_config: ClientConfig,
    database: Option<Arc<RwLock<ObjectDatabase>>>,
    /// Where the Device the session builds keeps what peers write to it
    /// ([`Self::with_device_storage_path`], #2730).
    device_storage_path: Option<std::path::PathBuf>,
    source_audit_reporter: Option<ObjectIdentifier>,
    source_audit: Option<Arc<crate::source_audit::SourceAudit>>,
    pub(crate) source_audit_bindings: Vec<(ObjectIdentifier, std::net::SocketAddrV4)>,
    pub(crate) source_audit_link_bindings: Vec<(ObjectIdentifier, bacnet_types::MacAddr)>,
    pub(crate) source_audit_routed_bindings: RoutedBindings,
    source_recipient: Option<Arc<crate::source_audit::recipient::SourceRecipient>>,
    stop_exit: Option<Result<SessionExit, Error>>,
    ingress_stopped: bool,
    identity: Option<crate::identity::DeviceIdentity>,
    local_receive_capacity: u16,
    bip_local_address: Option<std::net::SocketAddrV4>,
    registered_network_port: Option<ObjectIdentifier>,
    registered_port_lease: std::sync::Weak<()>,
    egress: Option<bacnet_endpoint_core::endpoint_ingress::EndpointEgress>,
    /// The B/IP transport's own exchanges, when the B/IP builder built it.
    pub(crate) bip_status: Option<bacnet_transport::bip::BvlcClientStatus>,
    /// The B/IP transport's BBMD table reads, when the B/IP builder built it.
    pub(crate) bip_tables: Option<bacnet_transport::bip::BvlcTableReader>,
    pub(crate) frame_tap: Option<bacnet_transport::tap::FrameTap>,
    /// The MS/TP transport's counters, when the MS/TP builder built it.
    pub(crate) mstp_diagnostics: Option<bacnet_transport::mstp::MstpDiagnostics>,
    /// The SC transport's events and link status, when the SC builder built it.
    pub(crate) sc_link: Option<bacnet_transport::sc::ScLinkMonitor>,
}

/// Terminal state of the session dispatch task.
///
/// Returned by [`stop`](EndpointSession::stop): either an explicit stop won
/// the race, or all dispatch receivers closed, as they do when the ingress
/// classifier ends (its input closed).
#[derive(Debug)]
pub enum SessionExit {
    /// Explicit `stop()` won the race.
    Cancelled,
    /// Dispatch receivers all closed.
    ReceiversClosed,
}

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// Creates a session owning `transport` (not yet started).
    ///
    /// Returns [`Error::InvalidConfig`] when `config.queue_capacity` is zero or
    /// `config.min_request_interval_ms` is more than an hour. Normally built
    /// via
    /// [`BipEndpointBuilder`](crate::bip::BipEndpointBuilder),
    /// [`ScEndpointBuilder`](crate::sc::ScEndpointBuilder), or
    /// [`MstpEndpointBuilder`](crate::mstp::MstpEndpointBuilder) instead of
    /// directly.
    ///
    /// ```
    /// use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
    /// use bacnet_transport::loopback::LoopbackTransport;
    ///
    /// let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    /// let bad = SessionConfig {
    ///     queue_capacity: 0,
    ///     ..SessionConfig::default()
    /// };
    /// assert!(EndpointSession::new(transport, SessionRole::Both, bad).is_err());
    /// ```
    pub fn new(transport: T, role: SessionRole, config: SessionConfig) -> Result<Self, Error> {
        config.validate()?;
        let (requester, notification) = role.invoke_id_reserves();
        let coordinator = Arc::new(OutboundTransactionCoordinator::with_reserves(
            requester,
            notification,
        ));
        let client_config = ClientConfig {
            apdu_timeout_ms: config.apdu_timeout_ms,
            apdu_retries: config.apdu_retries,
            max_apdu_length: config.max_apdu_length,
            min_request_interval_ms: config.min_request_interval_ms,
            accept_complex_ack_for_simple_services: config.accept_complex_ack_for_simple_services,
            ..ClientConfig::default()
        };
        let local_receive_capacity = transport.local_receive_apdu_capacity();
        // Answers to the session's own leases come before answers no lease
        // holds when the answer queue runs short (#2635).
        let ingress = EndpointIngress::new(transport, config.queue_capacity)
            .with_coordinator(Arc::clone(&coordinator));
        Ok(Self {
            local_receive_capacity,
            shared: Arc::new(SessionShared {
                counters: std::sync::Mutex::new(PolicyCounters {
                    ingress_policy: 0,
                    no_server_role: 0,
                    no_client_role: 0,
                    unclaimed_terminal: 0,
                }),
            }),
            coordinator,
            ingress: Some(ingress),
            notifications: None,
            server_builder: None,
            server: None,
            server_stop_error: None,
            server_gate: None,
            dispatch_task: None,
            network_number_task: None,
            router_task: None,
            cancel_tx: None,
            lifecycle: AtomicU8::new(Lifecycle::Ready as u8),
            role,
            config,
            client_config,
            database: None,
            device_storage_path: None,
            source_audit_reporter: None,
            source_audit: None,
            source_audit_bindings: Vec::new(),
            source_audit_link_bindings: Vec::new(),
            source_audit_routed_bindings: Vec::new(),
            source_recipient: None,
            stop_exit: None,
            ingress_stopped: false,
            identity: None,
            bip_local_address: None,
            registered_network_port: None,
            registered_port_lease: std::sync::Weak::new(),
            egress: None,
            bip_status: None,
            bip_tables: None,
            frame_tap: None,
            mstp_diagnostics: None,
            sc_link: None,
        })
    }

    /// Attaches the local object database (before start).
    ///
    /// Must be called before [`start`](Self::start). With a
    /// [`DeviceIdentity`](crate::identity::DeviceIdentity), omitting the database
    /// requests stock generation after receive-capacity resolution. Explicitly
    /// supplied databases, including empty ones, are preserved. Their selected
    /// Device must match the resolved identity, its segmentation included.
    /// The session retains ownership even in `ClientOnly` mode; with a
    /// server role it hands the database to its server at start, and shares
    /// it from then on.
    ///
    /// # Panics
    /// Panics if startup has already consumed the session configuration.
    pub fn with_database(mut self, db: ObjectDatabase) -> Self {
        self.assert_configurable();
        self.database = Some(Arc::new(RwLock::new(db)));
        self
    }

    /// Select the sole source Audit Reporter in the attached local database.
    ///
    /// Activates source Audit reporting of what the session's
    /// [`client`](Self::client) asks, which lists the audited services. The built-in
    /// Device must have a typed Audit recipient provision, even at Audit_Level
    /// NONE. Device choices use immutable builder route bindings; direct Address
    /// choices need no binding. Monitored_Objects must be absent. `Both`
    /// requires a server whose policy decides mutations, so that no peer
    /// rewrites the audited Device's recipient unasked: the read-only server
    /// a session runs without a builder does; see
    /// [`with_server`](Self::with_server). `ClientOnly` permits trusted local
    /// changes and `ServerOnly` is rejected.
    /// Standalone BACnetClient source reporting remains unsupported.
    ///
    /// [`start`](Self::start) requires a concrete built-in local Device
    /// ([`ObjectDatabase::local_device`]: the lowest instance when the database
    /// holds several, which preflight requires to be the identity's) matching the optional identity, and one selected Reporter capability with
    /// no conflicting source Reporter. Preflight validation changes no source flags
    /// and leaves configuration retryable; post-ingress failure joins terminal cleanup.
    /// Successful startup installs the Device
    /// recipient mutation owner and source projection without sending traffic.
    /// Device/Reporter membership is protected until owned frames quiesce. The
    /// private adapter forwards the original object behavior when sealed/released.
    /// Repeated calls before start replace the selection.
    ///
    /// ```compile_fail,E0603
    /// use bacnet_endpoint::session::source_reporter::SourceReporter;
    /// ```
    ///
    /// ```no_run
    /// use std::net::{Ipv4Addr, SocketAddrV4};
    /// use bacnet_endpoint::{bip::BipEndpointBuilder, DeviceIdentity, SessionRole};
    /// use bacnet_objects::{audit::AuditReporterObject, traits::BACnetObject};
    /// use bacnet_types::{constructed::BACnetRecipient, enums::ObjectType, primitives::ObjectIdentifier};
    /// # async fn example() -> Result<(), bacnet_types::error::Error> {
    /// let identity = DeviceIdentity::new(123, 42)?;
    /// let mut db = identity.build_database()?;
    /// let logger = ObjectIdentifier::new(ObjectType::DEVICE, 999)?;
    /// db.get_mut(&identity.device_oid()).unwrap().device_authority_internal().unwrap()
    ///     .provision_audit_recipient(BACnetRecipient::Device(logger))?;
    /// let reporter = AuditReporterObject::new(1, "Source configuration")?;
    /// let reporter_oid = reporter.object_identifier();
    /// db.add(Box::new(reporter))?;
    /// let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
    ///     .role(SessionRole::ClientOnly).database(db).device(identity)
    ///     .source_audit_device_binding(logger, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 47808))
    ///     .build_session()?.with_source_audit_reporter(reporter_oid);
    /// session.start().await?; // Startup emits no audit traffic.
    /// session.stop().await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Panics
    /// Panics if startup has already consumed the session configuration.
    pub fn with_source_audit_reporter(mut self, reporter: ObjectIdentifier) -> Self {
        self.assert_configurable();
        self.source_audit_reporter = Some(reporter);
        self
    }

    /// Composes the single Device identity (before start).
    ///
    /// Truth direction: identity overrides the standalone `SessionConfig`
    /// 480 default for the client role max-APDU, and the client follows the
    /// identity's segmentation (see [`client`](Self::client)); timers and
    /// retries stay from `SessionConfig`. Startup resolves the ceiling
    /// against the owned link.
    /// An absent database requests stock generation; an explicitly supplied
    /// database is preserved and its selected Device must match the resolved
    /// identity. Sessions without an identity keep the 480 default.
    ///
    /// # Panics
    /// Panics if startup has already consumed the session configuration.
    pub fn device(mut self, identity: crate::identity::DeviceIdentity) -> Self {
        self.assert_configurable();
        self.client_config.max_apdu_length = identity.max_apdu_length();
        self.config.max_apdu_length = identity.max_apdu_length();
        crate::identity::apply_segmentation(&identity, &mut self.client_config);
        self.identity = Some(identity);
        self
    }

    fn assert_configurable(&self) {
        assert_eq!(
            self.lifecycle.load(Ordering::Acquire),
            Lifecycle::Ready as u8,
            "endpoint configuration must precede startup"
        );
    }

    /// Starts ingress, roles and the single dispatch consumer once.
    ///
    /// Start-once: a second call returns [`Error::State`] with
    /// [`AlreadyStarted`](bacnet_types::error::Lifecycle::AlreadyStarted)
    /// without binding again. Takes `&mut self` so only the owner can start.
    /// Source-profile validation runs before lifecycle consumption or ingress
    /// startup; its errors leave the session ready for correction and retry.
    /// Profile initialization errors after ingress starts cancel and join ingress
    /// before returning, leaving the session terminal. Canceling that cleanup
    /// leaves a stopping session whose teardown can resume with [`stop`](Self::stop).
    ///
    /// A refusal is a [`StartError`], which `?` turns into [`Error`]. One
    /// that ends the session, once the link has started (a link that won't
    /// start, a registered Network Port it can't publish, a server build or
    /// a source Audit installation the started link refuses), hands back
    /// the database (#2766): the one [`with_database`](Self::with_database)
    /// gave, or the stock one the start built from the identity. The
    /// session's cleanup has let go of every other handle on it by then, a
    /// server the start built included. It comes back with what a start
    /// sets and the next start sets again:
    ///
    /// - the identity's Device bound as the local Device;
    /// - a registered Network Port the start published put back as it was
    ///   configured, so the next start reserves it again;
    /// - a source Audit Reporter the start installed taken out again;
    /// - once the start gave it to its server, what [`StartError`] lists for
    ///   a server's start on a seat, with the outputs that start drove
    ///   released: a Load Control's application is told to stop, and the
    ///   object keeps its request.
    ///
    /// A refusal after the server was built (a source Audit installation
    /// the started link refuses) comes once that server has run, until the
    /// cleanup stopped it, with its requests held for the start's end. Its
    /// periodic work may have had its first pass by then, and the database
    /// keeps what that pass did, as a server's start and stop leave it: a
    /// Schedule's value written to its Present_Value and the properties it
    /// references, a Trend Log's records, the event transitions a first
    /// sweep confirmed, whose notifications went out on the link.
    ///
    /// [`can_start`](Self::can_start) tells what a refusal without a
    /// database means. `true`: it came before the link started, and the
    /// session keeps the database, ready to correct and start again.
    /// `false`: the session had already started, or this start ended it
    /// while something it began still held the database a few runtime
    /// turns after its cleanup; the session keeps the database then too,
    /// and lets go of it off the runtime as it drops. A start cancelled
    /// after the link started also leaves the database with the session.
    pub async fn start(&mut self) -> Result<(), StartError> {
        let kept = |error| start_refused(error, None);
        let again = || {
            kept(Error::state(
                error::Lifecycle::AlreadyStarted,
                "endpoint session cannot be started more than once",
            ))
        };
        if self.lifecycle.load(Ordering::Acquire) != Lifecycle::Ready as u8 {
            return Err(again());
        }
        let source_routes = self.prepare_start().await.map_err(kept)?;
        if self.lifecycle.compare_exchange(
            Lifecycle::Ready as u8,
            Lifecycle::Running as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) != Ok(Lifecycle::Ready as u8)
        {
            return Err(again());
        }
        if let Err(error) = self.start_link(source_routes).await {
            // Keep cleanup ownership in self before awaiting. Cancellation leaves
            // Stopping plus intact joins; stop/Drop can still finish teardown.
            let _ = self.stop().await;
            return Err(start_refused(error, self.hand_back_database().await));
        }
        Ok(())
    }

    /// Whether [`start`](Self::start) may run: before the first start, and
    /// after a start refused before its link started, which leaves the
    /// session ready to correct and start again. `false` once a start has
    /// begun the link, through stop and after it, so after a refused start
    /// it tells a refusal the session can retry from one that ended it
    /// (#2766).
    pub fn can_start(&self) -> bool {
        self.lifecycle.load(Ordering::Acquire) == Lifecycle::Ready as u8
    }

    /// The database of a start the session's stop has just ended (#2766):
    /// once every other handle has gone, it comes out of its lock with what
    /// the start began undone. The stop has joined the session's own work,
    /// so it waits at most [`HAND_BACK_TURNS`] runtime turns for anything
    /// that still holds a handle; a handle shared past them stays with the
    /// session, which lets go of it off the runtime as it drops. The
    /// database stays in the session until it is taken, so a start
    /// cancelled here leaves it there too.
    async fn hand_back_database(&mut self) -> Option<ObjectDatabase> {
        for _ in 0..HAND_BACK_TURNS {
            if self
                .database
                .as_ref()
                .is_none_or(|db| Arc::strong_count(db) == 1)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        let db = self.database.take()?;
        match Arc::try_unwrap(db) {
            Ok(db) => {
                let mut db = db.into_inner();
                db.undo_refused_start_internal();
                Some(db)
            }
            Err(shared) => {
                self.database = Some(shared);
                None
            }
        }
    }

    /// Stops dispatch, roles and ingress once; joins termination.
    ///
    /// Returns the dispatch [`SessionExit`]. A canceled stop retains its joins
    /// and sealed membership protection; a later call finishes teardown. Calling before
    /// `start()` returns [`Error::State`] with
    /// [`NotRunning`](bacnet_types::error::Lifecycle::NotRunning), and once
    /// stopped with [`Closed`](bacnet_types::error::Lifecycle::Closed). Takes
    /// `&mut self` so only the owner can stop.
    ///
    /// The session's server ([`with_server`](Self::with_server), or the
    /// read-only one) is stopped first, while the link is still up, as
    /// [`BACnetServer::stop`](bacnet_server::server::BACnetServer::stop)
    /// describes, and its stop error, if any, is this call's. Only the
    /// server stages saves on the session's database. Its stop joins its
    /// work, waits up to 5 s for local writes dropped with their stage still
    /// held to let go of it, then settles the objects' forgotten staged
    /// writes and waits for their saves: storage that stalls keeps this call
    /// pending, and completion does not certify backend success or
    /// power-loss durability.
    ///
    /// Settling needs the database's guard. If any guard is held at that
    /// moment, the application's or even a background reader's brief one,
    /// the server hands the settling to a task that runs once the guard is
    /// free, and nothing waits for that task's saves, so this call can return
    /// before storage holds what the objects serve. It still waits for the
    /// guard where an Audit Reporter is taken out under it (the session's
    /// source Reporter, or the server's own Audit Reporters), but the
    /// settling has been handed off by then. A runtime that shuts down
    /// before that task runs drops it, and storage is then put back only as
    /// the objects are dropped: a process that exits without running
    /// destructors can lose it. Let go of every guard first where storage
    /// must be settled when this returns.
    ///
    /// The stopped server has let go of its database handle when this call
    /// returns (#2766), but for settling handed to a task as above, which
    /// holds one until it has run.
    ///
    /// A [`client`](Self::client) still alive doesn't hold the
    /// stop up (#2617): its tasks end, the link stops, and its requests
    /// after that fail with the transport's stopped error; dropping it later
    /// changes nothing. The database remains attached after termination.
    pub async fn stop(&mut self) -> Result<SessionExit, Error> {
        let state = self.lifecycle.load(Ordering::Acquire);
        if state != Lifecycle::Running as u8 && state != Lifecycle::Stopping as u8 {
            return Err(self.not_running());
        }
        self.lifecycle
            .store(Lifecycle::Stopping as u8, Ordering::Release);
        for slot in [&mut self.network_number_task, &mut self.router_task] {
            if let Some(task) = slot.as_mut() {
                task.abort();
                let _ = task.await;
            }
            *slot = None;
        }
        if let Some(source) = &self.source_audit {
            source.close();
        }
        self.stop_server().await;
        // Closing the pool aborts its workers, an audited request's runner
        // among them; the join below waits for each. Beside a server the
        // pool is the server's, which its stop above closed and joined.
        if let Some(notifications) = &self.notifications {
            notifications.close();
        }
        if let Some(cancel) = self.cancel_tx.take() {
            let _ = cancel.send(());
        }
        if self.stop_exit.is_none() {
            self.stop_exit = Some(match self.dispatch_task.as_mut() {
                // A dispatch task that panicked or was aborted: the session
                // is shut for good, as every stopped one is.
                Some(task) => task.await.map_err(|join| {
                    Error::State(
                        error::Lifecycle::Closed,
                        Refusal::text(format!("endpoint session dispatch failed: {join}")),
                    )
                }),
                None => Ok(SessionExit::ReceiversClosed),
            });
            self.dispatch_task.take();
        }
        if let Some(notifications) = &self.notifications {
            while let Some(result) = notifications.join_next().await {
                NotificationTransactions::observe(Some(result));
            }
        }
        if !self.ingress_stopped {
            if let Some(ingress) = self.ingress.as_mut()
                && let Err(error) = ingress.stop().await
                && self.stop_exit.as_ref().is_some_and(Result::is_ok)
            {
                self.stop_exit = Some(Err(error));
            }
            self.ingress_stopped = true;
        }
        // The owner remains in the session through every await. The DB barrier
        // observes completed synchronous commits; queued source readers recheck
        // the sealed weak sink after taking this guard and cannot reenter it.
        if let Some(runtime) = &self.source_recipient {
            runtime.uninstall(
                &mut *self
                    .database
                    .as_ref()
                    .expect("source database")
                    .write()
                    .await,
            );
        }
        self.source_recipient.take();
        self.source_audit.take();
        self.notifications.take();
        self.lifecycle
            .store(Lifecycle::Stopped as u8, Ordering::Release);
        let exit = self.stop_exit.take().expect("joined dispatch");
        match self.server_stop_error.take() {
            Some(error) if exit.is_ok() => Err(error),
            _ => exit,
        }
    }

    /// Samples policy-outcome ownership counters.
    ///
    /// Every classifier/policy outcome is counted, never silently dropped,
    /// and so is every APDU a full queue had no room for. A sample takes a
    /// short synchronous lock the dispatch task also takes, never held
    /// across an await, so it never waits on that task (#2759).
    pub fn policy_counters(&self) -> PolicyCountersSnapshot {
        let drops = self
            .ingress
            .as_ref()
            .map(EndpointIngress::full_drop_counters)
            .unwrap_or_default();
        let counters = self.shared.counters();
        PolicyCountersSnapshot {
            ingress_policy: counters.ingress_policy,
            no_server_role: counters.no_server_role,
            no_client_role: counters.no_client_role,
            unclaimed_terminal: counters.unclaimed_terminal,
            unconfirmed_full_drops: drops.unconfirmed_full_drops,
            confirmed_full_drops: drops.confirmed_full_drops,
            answer_full_drops: drops.answer_full_drops,
            policy_full_drops: drops.policy_full_drops,
        }
    }

    /// Samples the shared outbound lease count.
    ///
    /// Returns `usize::MAX` only when the coordinator lock is poisoned
    /// (internal invariant violation surfaced honestly, never hidden).
    pub fn active_leases(&self) -> usize {
        self.coordinator.active_count().unwrap_or(usize::MAX)
    }

    /// Shares the device-wide coordinator (dispatch + tests only).
    #[doc(hidden)]
    pub fn coordinator(&self) -> Arc<OutboundTransactionCoordinator> {
        Arc::clone(&self.coordinator)
    }

    /// Borrows the composed identity.
    pub fn identity(&self) -> Option<&crate::identity::DeviceIdentity> {
        self.identity.as_ref()
    }

    /// Actual announced B/IP address after successful startup.
    /// Absent before publication, while stopping, after stop, or for other links.
    pub fn bip_local_address(&self) -> Option<std::net::SocketAddrV4> {
        (self.is_running() && self.egress.as_ref().is_some_and(|egress| egress.is_open()))
            .then_some(self.bip_local_address)
            .flatten()
    }

    /// How the transport's foreign-device registration is going (#938):
    /// whether it holds a live lease, the time left on it, and the last
    /// attempt's result and outcome, a BBMD's refusal included. It is there
    /// for a session [`BipEndpointBuilder`](crate::bip::BipEndpointBuilder)
    /// built, in any mode and after stop, and `None` for any other: take a
    /// [`BipTransport::bvlc_client_status`](bacnet_transport::bip::BipTransport::bvlc_client_status)
    /// before handing a transport to [`Self::new`].
    pub fn foreign_registration(
        &self,
    ) -> Option<bacnet_transport::bip::ForeignRegistrationSnapshot> {
        let status = self.bip_status.as_ref()?;
        Some(status.snapshot().foreign_registration)
    }

    /// Reads a BBMD's two tables, its BDT and its FDT, through this
    /// session's own B/IP socket (#2612), as
    /// [`BipTransport::read_bdt`](bacnet_transport::bip::BipTransport::read_bdt)
    /// and [`read_fdt`](bacnet_transport::bip::BipTransport::read_fdt) do,
    /// beside the roles' traffic. A read fails as one before start does
    /// while the session is stopped. It is there for a session
    /// [`BipEndpointBuilder`](crate::bip::BipEndpointBuilder) built, and
    /// `None` for any other: take a
    /// [`BipTransport::bvlc_table_reader`](bacnet_transport::bip::BipTransport::bvlc_table_reader)
    /// before handing a transport to [`Self::new`]. It only reads; the
    /// table writes stay on the transport.
    pub fn bvlc_tables(&self) -> Option<bacnet_transport::bip::BvlcTableReader> {
        self.bip_tables.clone()
    }

    /// The MS/TP transport's counters (#958): a handle to the same
    /// [`MstpDiagnostics`](bacnet_transport::mstp::MstpDiagnostics) the
    /// transport updates, readable in any state, after stop included. It is
    /// there for a session [`MstpEndpointBuilder`](crate::mstp::MstpEndpointBuilder)
    /// built, and `None` for any other: take
    /// [`MstpTransport::diagnostics`](bacnet_transport::mstp::MstpTransport::diagnostics)
    /// before handing a transport to [`Self::new`].
    pub fn mstp_diagnostics(&self) -> Option<bacnet_transport::mstp::MstpDiagnostics> {
        self.mstp_diagnostics.clone()
    }

    /// The BACnet/SC hub link's events and status (#1946): a handle on the
    /// transport's own event stream and status snapshot, readable in any
    /// state, after stop included. Subscribe to its events before
    /// [`start`](Self::start) to see the first connection, and through every
    /// reconnect and failover the transport makes after. It is there for a
    /// session [`ScEndpointBuilder`](crate::sc::ScEndpointBuilder) built, and
    /// `None` for any other: take
    /// [`ScTransport::link_monitor`](bacnet_transport::sc::ScTransport::link_monitor)
    /// before handing a transport to [`Self::new`].
    pub fn sc_link(&self) -> Option<bacnet_transport::sc::ScLinkMonitor> {
        self.sc_link.clone()
    }

    /// Returns the composed session role.
    pub fn session_role(&self) -> SessionRole {
        self.role
    }

    /// Returns true while the session dispatch is running.
    pub fn is_running(&self) -> bool {
        self.lifecycle.load(Ordering::Acquire) == Lifecycle::Running as u8
    }

    /// The state a call that needs the session running finds it in instead,
    /// from its own lifecycle (#1833): not started yet, which it can be, a
    /// stop under way, or stopped for good, as a session starts once.
    pub(crate) fn not_running_state(&self) -> error::Lifecycle {
        match self.lifecycle.load(Ordering::Acquire) {
            state if state == Lifecycle::Ready as u8 => error::Lifecycle::NotRunning,
            state if state == Lifecycle::Stopping as u8 => error::Lifecycle::Stopping,
            _ => error::Lifecycle::Closed,
        }
    }

    /// The refusal of a call that needs the session running, in
    /// [`not_running_state`](Self::not_running_state).
    pub(crate) fn not_running(&self) -> Error {
        Error::state(self.not_running_state(), "endpoint session is not running")
    }

    /// Broadcasts an I-Am consistent with the composed identity.
    ///
    /// Missing-identity path: errors without sending (no invented device).
    /// Present-identity path: encodes [`encode_iam_apdu`](crate::identity::encode_iam_apdu)
    /// and sends one local-broadcast Unconfirmed-Request via the session
    /// egress. Field-for-field identical to the server discovery I-Am built
    /// from [`server_config`](crate::identity::server_config).
    /// Transport-dependent: B/IP reaches the local subnet broadcast; SC
    /// relays via the hub as a broadcast NPDU. Fails with [`Error::State`]
    /// when the session is not running, with [`Error::InvalidConfig`] for a
    /// session with no identity or database, and with
    /// [`Error::InitiationDisabled`] while the session's
    /// server ([`with_server`](Self::with_server)) holds a DeviceCommunicationControl
    /// DISABLE_INITIATION: this I-Am answers no Who-Is (Clause 16.1).
    pub async fn broadcast_i_am(&self) -> Result<(), Error> {
        let identity = self.identity.as_ref().ok_or(Error::invalid_config(
            "endpoint I-Am requires a composed identity",
        ))?;
        if self
            .ingress
            .as_ref()
            .and_then(EndpointIngress::node_dcc)
            .is_some_and(|dcc| dcc.is_restricted())
        {
            return Err(Error::InitiationDisabled { sent: false });
        }
        let egress = self.egress.as_ref().ok_or_else(|| self.not_running())?;
        if !self.is_running() {
            return Err(self.not_running());
        }
        let database = self.database.as_ref().ok_or(Error::invalid_config(
            "endpoint I-Am has no Device database",
        ))?;
        // The guard covers only this check, never the send below (#2199),
        // and is never waited for in the caller's task (#2772): with the
        // database busy, a task of the session's waits and checks.
        let held = bacnet_server::caller_waits::HeldDatabase::new(Arc::clone(database));
        let device = identity.device_oid();
        let is_present = move |db: &ObjectDatabase| {
            db.get(&device)
                .is_some_and(|object| object.object_identifier() == device)
        };
        let check = async move { is_present(&*held.read().await) };
        let pool = match &self.server {
            Some(server) => Some(Arc::clone(server.notification_pool())),
            None => self.notifications.clone(),
        };
        // The session's pool, or the server's beside one, which the stop
        // closes; without either, the runtime, as the task ends once its
        // caller has its answer or has gone.
        let host = move |task: bacnet_server::caller_waits::CallerTask| {
            let Ok(runtime) = tokio::runtime::Handle::try_current() else {
                return Err(task);
            };
            // A pool the stop closed drops the task, and its caller hears
            // of that from the task's end.
            match pool {
                Some(pool) => {
                    let _ = pool.spawn(task);
                }
                None => drop(runtime.spawn(task)),
            }
            Ok(())
        };
        let checked = bacnet_server::caller_waits::for_caller(host, check, |present| present).await;
        let present = match checked {
            Some(present) => present,
            // Its runtime, or the pool a stop closed, dropped the task
            // before it checked: check here.
            None => is_present(&*database.read().await),
        };
        if !present {
            return Err(Error::invalid_config("endpoint I-Am Device is absent"));
        }
        let apdu = crate::identity::encode_iam_apdu(identity)?;
        egress
            .send_apdu(
                apdu,
                bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::LocalBroadcast,
                false,
                bacnet_npdu::NetworkPriority::Normal,
                Vec::new(),
            )
            .await
    }
}

#[cfg(test)]
#[path = "source_reporter_tests.rs"]
mod source_reporter_tests;

impl<T: TransportPort + 'static> Drop for EndpointSession<T> {
    fn drop(&mut self) {
        // Synchronous abort path: never orphan dispatch/ingress/role work.
        // `stop()` remains the graceful path; Drop only seals + aborts. The
        // ingress, dropped after this, aborts the link through the layer the
        // client may still share (#2617), so no live client keeps it up.
        for task in [&self.network_number_task, &self.router_task]
            .into_iter()
            .flatten()
        {
            task.abort();
        }
        if let Some(source) = self.source_audit.take() {
            source.close();
        }
        if let Some(notifications) = self.notifications.take() {
            notifications.close();
        }
        if let Some(cancel) = self.cancel_tx.take() {
            let _ = cancel.send(());
        }
        if let Some(task) = self.dispatch_task.take() {
            task.abort();
        }
        // The server's own drop aborts its work and lets go of the node's
        // DeviceCommunicationControl hold.
        drop(self.server.take());
        // The session's own handle, kept through stop(), goes off the
        // runtime if it is the last (#1561). Every other endpoint holder
        // (the server, source Audit, the aborted tasks' handles) lets go the
        // same way when it drops, whichever goes last.
        if let Some(db) = self.database.take() {
            drop(drop_database_off_runtime(db));
        }
    }
}

struct DispatchParts {
    inbound: mpsc::Receiver<ReceivedApdu>,
    terminal: mpsc::Receiver<ReceivedApdu>,
    policy: mpsc::Receiver<PolicyOutcome>,
    notifications: Option<Arc<NotificationTransactions>>,
    coordinator: Arc<OutboundTransactionCoordinator>,
    shared: Arc<SessionShared>,
    /// The layer's network number, which the session's Number owner
    /// publishes; answers relayed with it as SNET match like direct ones.
    local_network: LocalNetworkNumber,
}

#[path = "session_dispatch.rs"]
mod dispatch;
use dispatch::{DispatchEvent, handle_ingress_policy, handle_terminal, next_event};

async fn dispatch_loop(mut parts: DispatchParts, mut cancel: oneshot::Receiver<()>) -> SessionExit {
    // One consumer retains all receivers and joins; cancellation stays first.
    let mut next = 0;
    loop {
        let event = tokio::select! {
            biased;
            _ = &mut cancel => return SessionExit::Cancelled,
            event = next_event(&mut parts, &mut next) => event,
        };
        match event {
            DispatchEvent::Worker(joined) => NotificationTransactions::observe(Some(joined)),
            DispatchEvent::Inbound(received) => handle_inbound(&parts.shared, received),
            DispatchEvent::Terminal(received) => handle_terminal(&mut parts, received),
            DispatchEvent::Policy(outcome) => handle_ingress_policy(&parts.shared, outcome),
            DispatchEvent::Closed => return SessionExit::ReceiversClosed,
        }
    }
}

/// A request in the session's own request queue, which no server takes: the
/// session has no server role, or its server's seat is gone (#2617). It is
/// counted (`no_server_role`), and its one-use reply sender is released
/// without sending, as a full queue's drop releases one.
fn handle_inbound(shared: &SessionShared, received: ReceivedApdu) {
    shared.counters().no_server_role += 1;
    drop(received.reply_tx);
}

#[cfg(test)]
#[path = "source_read_tests.rs"]
mod source_read_tests;

#[cfg(test)]
#[path = "registered_port_bip6_tests.rs"]
mod registered_port_bip6_tests;
#[cfg(test)]
#[path = "registered_port_lifetime_tests.rs"]
mod registered_port_lifetime_tests;
#[cfg(test)]
#[path = "registered_port_wire_tests.rs"]
mod registered_port_wire_tests;

#[cfg(test)]
#[path = "network_number_tests.rs"]
mod network_number_tests;

/// The in-memory link the tests run on under paused time (#2293, #2298).
#[cfg(test)]
#[path = "../tests/support/bus_port.rs"]
mod bus_port;

/// Waits on paused time that fail at their limit instead of hanging (#2293).
#[cfg(test)]
#[path = "../tests/support/eventually.rs"]
mod eventually;

/// Reruns a test whose closing rebind loses the freed port to another
/// process (#1419).
#[cfg(test)]
#[path = "../tests/support/port_retry.rs"]
mod port_retry;

#[cfg(test)]
#[path = "local_network_tests.rs"]
mod local_network_tests;

#[cfg(test)]
#[path = "client_pacing_tests.rs"]
mod client_pacing_tests;

#[path = "device_identity.rs"]
mod device_identity;
