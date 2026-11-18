//! Single-owner endpoint session: one ingress + shared coordinator.
//!
//! Lifecycle lives here ONLY. Role handles expose no lifecycle methods.
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
//! Outbound leases use RAII guards. Ordinary `read_property*` calls release their
//! exact lease when cancelled. Source-audited calls transfer to bounded session
//! ownership before egress admission, so dropping their caller does not cancel
//! admitted work. `stop()` stops the server first, whose own stop settles its
//! objects' saves, or hands that to a task while the database is held (see
//! [`stop`](EndpointSession::stop)), then seals admission, closes roles, joins
//! dispatch and owned workers, and stops ingress. Undelivered audit records may be lost on shutdown.
//! Cloned role handles keep working until shutdown, then fail closed; after
//! the session drops, [`Weak`](std::sync::Weak) upgrade fails and every role
//! call reports shutdown.
//!
//! The session keeps its object database through `stop()` and lets go of it
//! when it drops. Every holder of the database (the session, its server,
//! the source Audit runtime and the session's tasks) lets go through
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
//! lifecycle mutation is owner-exclusive (`&mut`) or via atomics/async
//! mutexes; role handles synchronize through the shared session token.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use bacnet_client::EndpointRequester;
use bacnet_client::client::ClientConfig;
use bacnet_encoding::apdu::Apdu;
use bacnet_endpoint_core::coordinator::{
    AdmissionOutcome, CoordinatorError, OutboundTransactionCoordinator,
};
use bacnet_endpoint_core::endpoint_ingress::{EndpointIngress, PolicyOutcome, PolicyReason};
use bacnet_network::layer::ReceivedApdu;
use bacnet_network::network_number::LocalNetworkNumber;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
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

use crate::roles::{
    ClientRoleHandle, SessionToken, admit_once, decode_terminal, is_requester_lease,
};
use bacnet_server::server::{
    __endpoint_NotificationTransactions as NotificationTransactions, drop_database_off_runtime,
};

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
/// `role()` setter) and fixed for the session lifetime. Determines which
/// role handles exist after [`start`](EndpointSession::start):
///
/// - [`ClientOnly`](SessionRole::ClientOnly): `client()` is `Some`, the
///   server role's accessors are `None`. Inbound requests are counted
///   (`no_server_role`) and their one-use reply sender released without
///   sending.
/// - [`ServerOnly`](SessionRole::ServerOnly): the server role is the full
///   `server()` the session builds at start, the read-only one
///   ([`read_only_server`]) unless
///   [`with_server`](EndpointSession::with_server) gave it a builder
///   (#2617); `client()` is `None`. Terminal responses with no consumer
///   release their exact lease and count `no_client_role`.
/// - [`Both`](SessionRole::Both): both roles are present above the one
///   shared coordinator.
///
/// A session with a server role needs a Device identity
/// ([`device`](EndpointSession::device)): its start refuses one without,
/// before the link starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionRole {
    /// Client requester only.
    ClientOnly,
    /// Server role only: a full server ([`EndpointSession::with_server`],
    /// or [`read_only_server`] without one).
    ServerOnly,
    /// Both sibling roles above one ingress/coordinator.
    Both,
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
    /// at 16. A [`full_client`](EndpointSession::full_client)'s answers
    /// don't use this queue but its seat's, which has tiers of its own.
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
    /// [`Error::Encoding`] otherwise.
    pub queue_capacity: usize,
    /// The node's APDU timeout in milliseconds (default 6000): how long the
    /// client roles wait for an answer before they send a request again,
    /// and, with a full server ([`with_server`](EndpointSession::with_server)), how long
    /// its confirmed notifications wait, and the Device's APDU_Timeout
    /// (Clause 12.11.28).
    pub apdu_timeout_ms: u64,
    /// The node's APDU retries (default 3): how many times the client roles
    /// send a request again, and, with a full server
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
    /// an hour; [`EndpointSession::new`] returns [`Error::Encoding`] past it.
    pub min_request_interval_ms: u64,
}

impl SessionConfig {
    /// The checks [`EndpointSession::new`] makes, without a transport, so a
    /// builder that dials can make them before it does (#1946).
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.queue_capacity == 0 {
            return Err(Error::Encoding(
                "endpoint session queue capacity must be greater than zero".into(),
            ));
        }
        let interval = self.min_request_interval_ms;
        if interval > bacnet_client::client::MAX_MIN_REQUEST_INTERVAL_MS {
            return Err(Error::Encoding(format!(
                "endpoint session min-request-interval {interval} ms is more than {} ms",
                bacnet_client::client::MAX_MIN_REQUEST_INTERVAL_MS
            )));
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
    /// Terminal responses with no client/notification role.
    pub no_client_role: u64,
    /// Terminal responses no lease claimed (unknown/peer/service/policy).
    pub unclaimed_terminal: u64,
    /// Unconfirmed requests past their share of the full request queue
    /// (see [`SessionConfig::queue_capacity`]).
    pub unconfirmed_full_drops: u64,
    /// Confirmed requests for the session's own roles that its full request
    /// queue had no room for; none was answered, and the requester's retry
    /// is its recovery. One for a full client (its COV notification, or any
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
    token: Arc<SessionToken>,
    counters: Mutex<PolicyCounters>,
}

/// One owned single-device endpoint session.
///
/// Owns ONE [`EndpointIngress`], the shared [`OutboundTransactionCoordinator`],
/// role registration, policy-outcome ownership, timers, egress and
/// termination. Start-once/stop-once; `Drop` aborts without orphaning.
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
    requester: Option<EndpointRequester>,
    /// The source Audit's pool of notifications on a session with no
    /// server; beside a server, its pool is the server's.
    notifications: Option<Arc<NotificationTransactions>>,
    client_handle: Option<ClientRoleHandle>,
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
    cancel_tx: Option<oneshot::Sender<()>>,
    lifecycle: AtomicU8,
    role: SessionRole,
    #[allow(dead_code)]
    config: SessionConfig,
    client_config: ClientConfig,
    database: Option<Arc<RwLock<ObjectDatabase>>>,
    source_audit_reporter: Option<ObjectIdentifier>,
    source_audit: Option<Arc<crate::source_audit::SourceAudit>>,
    pub(crate) source_audit_bindings: Vec<(ObjectIdentifier, std::net::SocketAddrV4)>,
    pub(crate) source_audit_link_bindings: Vec<(ObjectIdentifier, bacnet_types::MacAddr)>,
    pub(crate) source_audit_routed_bindings: Vec<(ObjectIdentifier, u16, bacnet_types::MacAddr)>,
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
    /// Returns [`Error::Encoding`] when `config.queue_capacity` is zero or
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
        let coordinator = Arc::new(OutboundTransactionCoordinator::new());
        let token = SessionToken::new(Arc::clone(&coordinator));
        let client_config = ClientConfig {
            apdu_timeout_ms: config.apdu_timeout_ms,
            apdu_retries: config.apdu_retries,
            max_apdu_length: config.max_apdu_length,
            min_request_interval_ms: config.min_request_interval_ms,
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
                token,
                counters: Mutex::new(PolicyCounters {
                    ingress_policy: 0,
                    no_server_role: 0,
                    no_client_role: 0,
                    unclaimed_terminal: 0,
                }),
            }),
            coordinator,
            ingress: Some(ingress),
            requester: None,
            notifications: None,
            client_handle: None,
            server_builder: None,
            server: None,
            server_stop_error: None,
            server_gate: None,
            dispatch_task: None,
            network_number_task: None,
            cancel_tx: None,
            lifecycle: AtomicU8::new(Lifecycle::Ready as u8),
            role,
            config,
            client_config,
            database: None,
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
    /// Device must match the resolved identity and declare segmentation
    /// NO_SEGMENTATION, as the session's identity must for now. The session
    /// retains ownership even in `ClientOnly` mode; with a server role it
    /// hands the database to its server at start, and shares it from then
    /// on.
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
    /// Activates bounded source READ reporting on direct IPv4 B/IP. The built-in
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
    /// 480 default for the client role max-APDU; timers/retries stay from
    /// `SessionConfig`. Startup resolves the ceiling against the owned link.
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
    /// Start-once: a second call returns
    /// [`Error::Encoding`] without
    /// binding again. Takes `&mut self` so only the owner can start.
    /// Source-profile validation runs before lifecycle consumption or ingress
    /// startup; its errors leave the session ready for correction and retry.
    /// Profile initialization errors after ingress starts cancel and join ingress
    /// before returning, leaving the session terminal. Canceling that cleanup
    /// leaves a stopping session whose teardown can resume with [`stop`](Self::stop).
    pub async fn start(&mut self) -> Result<(), Error> {
        if self.lifecycle.load(Ordering::Acquire) != Lifecycle::Ready as u8 {
            return Err(Error::Encoding(
                "endpoint session cannot be started more than once".into(),
            ));
        }
        let source_routes = self.prepare_start().await?;
        if self.lifecycle.compare_exchange(
            Lifecycle::Ready as u8,
            Lifecycle::Running as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) != Ok(Lifecycle::Ready as u8)
        {
            return Err(Error::Encoding(
                "endpoint session cannot be started more than once".into(),
            ));
        }
        if let Err(error) = self.start_link(source_routes).await {
            // Keep cleanup ownership in self before awaiting. Cancellation leaves
            // Stopping plus intact joins; stop/Drop can still finish teardown.
            let _ = self.stop().await;
            return Err(error);
        }
        Ok(())
    }

    /// Stops dispatch, roles and ingress once; joins termination.
    ///
    /// Returns the dispatch [`SessionExit`]. A canceled stop retains its joins
    /// and sealed membership protection; a later call finishes teardown. Calling before
    /// `start()` or twice returns
    /// [`Error::Encoding`]. Takes
    /// `&mut self` so only the owner can stop; cloned role handles observe
    /// shutdown and fail closed.
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
    /// The stopped server lets go of its database handle from a task, a
    /// runtime turn after this call returns, as a standalone server's drop
    /// does: code that takes the database back once the session is gone
    /// (`Arc::try_unwrap` of a clone it kept) lets that turn pass first.
    ///
    /// A [`full_client`](Self::full_client) still alive doesn't hold the
    /// stop up (#2617): its tasks end, the link stops, and its requests
    /// after that fail with the transport's stopped error; dropping it later
    /// changes nothing. The database remains attached after termination.
    pub async fn stop(&mut self) -> Result<SessionExit, Error> {
        let state = self.lifecycle.load(Ordering::Acquire);
        if state != Lifecycle::Running as u8 && state != Lifecycle::Stopping as u8 {
            return Err(Error::Encoding("endpoint session is not running".into()));
        }
        self.lifecycle
            .store(Lifecycle::Stopping as u8, Ordering::Release);
        self.shared.token.shutdown();
        if let Some(task) = self.network_number_task.as_mut() {
            task.abort();
            let _ = task.await;
        }
        self.network_number_task = None;
        if let Some(source) = &self.source_audit {
            source.close();
        }
        self.stop_server().await;
        if let Some(requester) = &self.requester {
            requester.close();
        }
        if let Some(notifications) = &self.notifications {
            notifications.close();
        }
        if let Some(cancel) = self.cancel_tx.take() {
            let _ = cancel.send(());
        }
        if self.stop_exit.is_none() {
            self.stop_exit = Some(match self.dispatch_task.as_mut() {
                Some(task) => task.await.map_err(|error| {
                    Error::Encoding(format!("endpoint session dispatch failed: {error}"))
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
        self.requester.take();
        self.client_handle = None;
        self.lifecycle
            .store(Lifecycle::Stopped as u8, Ordering::Release);
        let exit = self.stop_exit.take().expect("joined dispatch");
        match self.server_stop_error.take() {
            Some(error) if exit.is_ok() => Err(error),
            _ => exit,
        }
    }

    /// Borrows the client role (`None` when not composed).
    ///
    /// Returns `None` for [`ServerOnly`](SessionRole::ServerOnly) sessions,
    /// and after [`stop`](Self::stop) (handles are released at termination).
    pub fn client(&self) -> Option<&ClientRoleHandle> {
        self.client_handle.as_ref()
    }

    /// Clones the client role handle with its session binding.
    ///
    /// The clone holds only a [`Weak`](std::sync::Weak) session token: it
    /// works while the session runs and fails closed after `stop()`/drop.
    /// Used to hand the client role to tasks that outlive the borrow.
    pub fn cloned_client_handle(&self) -> Option<ClientRoleHandle> {
        self.client_handle.clone()
    }

    /// Samples policy-outcome ownership counters.
    ///
    /// Every classifier/policy outcome is counted, never silently dropped,
    /// and so is every APDU a full queue had no room for.
    pub async fn policy_counters(&self) -> PolicyCountersSnapshot {
        let drops = self
            .ingress
            .as_ref()
            .map(EndpointIngress::full_drop_counters)
            .unwrap_or_default();
        let counters = self.shared.counters.lock().await;
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

    /// Borrows the composed identity (narrow admin; no lifecycle).
    ///
    /// RB-15 narrow admin borrow: roles hold no transport/session lifecycle;
    /// only the session owner exposes identity + counters + coordinator.
    /// A role handle attempting `stop`/transport access fails at compile
    /// time (no such method) and at runtime its post-stop calls fail closed.
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

    /// Returns the composed session role (narrow admin).
    pub fn session_role(&self) -> SessionRole {
        self.role
    }

    /// Returns true while the session dispatch is running (narrow admin).
    pub fn is_running(&self) -> bool {
        self.lifecycle.load(Ordering::Acquire) == Lifecycle::Running as u8
    }

    /// Broadcasts an I-Am consistent with the composed identity.
    ///
    /// Missing-identity path: errors without sending (no invented device).
    /// Present-identity path: encodes [`encode_iam_apdu`](crate::identity::encode_iam_apdu)
    /// and sends one local-broadcast Unconfirmed-Request via the session
    /// egress. Field-for-field identical to the server discovery I-Am built
    /// from [`server_config`](crate::identity::server_config).
    /// Transport-dependent: B/IP reaches the local subnet broadcast; SC
    /// relays via the hub as a broadcast NPDU. Fails when the session is not
    /// running, and with [`Error::InitiationDisabled`] while the session's
    /// server ([`with_server`](Self::with_server)) holds a DeviceCommunicationControl
    /// DISABLE_INITIATION: this I-Am answers no Who-Is (Clause 16.1).
    pub async fn broadcast_i_am(&self) -> Result<(), Error> {
        let identity = self
            .identity
            .as_ref()
            .ok_or_else(|| Error::Encoding("endpoint I-Am requires a composed identity".into()))?;
        if self
            .ingress
            .as_ref()
            .and_then(EndpointIngress::node_dcc)
            .is_some_and(|dcc| dcc.is_restricted())
        {
            return Err(Error::InitiationDisabled { sent: false });
        }
        let egress = self
            .egress
            .as_ref()
            .ok_or_else(|| Error::Encoding("endpoint session is not running".into()))?;
        if !self.is_running() {
            return Err(Error::Encoding("endpoint session is not running".into()));
        }
        let database = self
            .database
            .as_ref()
            .ok_or_else(|| Error::Encoding("endpoint I-Am has no Device database".into()))?;
        // The guard covers only this check, never the send below (#2199).
        let present = database
            .read()
            .await
            .get(&identity.device_oid())
            .is_some_and(|object| object.object_identifier() == identity.device_oid());
        if !present {
            return Err(Error::Encoding("endpoint I-Am Device is absent".into()));
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
        // `stop()` remains the graceful path; Drop only seals + aborts.
        self.shared.token.shutdown();
        if let Some(task) = &self.network_number_task {
            task.abort();
        }
        if let Some(source) = self.source_audit.take() {
            source.close();
        }
        if let Some(requester) = self.requester.take() {
            requester.close();
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
    requester: Option<EndpointRequester>,
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
            DispatchEvent::Inbound(received) => handle_inbound(&parts.shared, received).await,
            DispatchEvent::Terminal(received) => handle_terminal(&mut parts, received).await,
            DispatchEvent::Policy(outcome) => handle_ingress_policy(&parts.shared, outcome).await,
            DispatchEvent::Closed => return SessionExit::ReceiversClosed,
        }
    }
}

/// A request in the session's own request queue, which no server takes: the
/// session has no server role, or its server's seat is gone (#2617). It is
/// counted (`no_server_role`), and its one-use reply sender is released
/// without sending, as a full queue's drop releases one.
async fn handle_inbound(shared: &SessionShared, received: ReceivedApdu) {
    shared.counters.lock().await.no_server_role += 1;
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
