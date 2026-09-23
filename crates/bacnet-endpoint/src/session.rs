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
//! admitted work. `stop()` seals admission, closes roles, joins dispatch and owned
//! workers, then stops ingress. Undelivered audit records may be lost on shutdown.
//! Cloned role handles keep working until shutdown, then fail closed; after
//! the session drops, [`Weak`](std::sync::Weak) upgrade fails and every role
//! call reports shutdown.
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

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use bacnet_client::client::ClientConfig;
use bacnet_client::EndpointRequester;
use bacnet_encoding::apdu::Apdu;
use bacnet_endpoint_core::coordinator::{
    AdmissionOutcome, CoordinatorError, OutboundTransactionCoordinator,
};
use bacnet_endpoint_core::endpoint_ingress::{
    ClassifierExit, EndpointIngress, PolicyOutcome, PolicyReason,
};
use bacnet_network::layer::ReceivedApdu;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio::task::JoinHandle;

#[path = "source_reporter.rs"]
mod source_reporter;

use crate::roles::{
    admit_once, decode_terminal, inbound_canonical_peer, is_requester_lease, ClientRoleHandle,
    ServerRoleHandle, SessionToken,
};
use bacnet_server::server::{
    __endpoint_EndpointResponder as EndpointResponder,
    __endpoint_NotificationTransactions as NotificationTransactions,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Ready = 0,
    Running = 1,
    Stopped = 2,
}

/// Which roles the session composes.
///
/// Chosen at [`EndpointSession::new`] time (or via the endpoint builders'
/// `role()` setter) and fixed for the session lifetime. Determines which
/// role handles exist after [`start`](EndpointSession::start):
///
/// - [`ClientOnly`](SessionRole::ClientOnly): `client()` is `Some`, `server()`
///   is `None`. Inbound requests are counted (`no_server_role`) and their
///   one-use reply sender released without sending.
/// - [`ServerOnly`](SessionRole::ServerOnly): `server()` is `Some`,
///   `client()` is `None`. Terminal responses with no consumer release their
///   exact lease and count `no_client_role`.
/// - [`Both`](SessionRole::Both): both handles are present above the one
///   shared coordinator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionRole {
    /// Client requester only.
    ClientOnly,
    /// Server responder + notification pool only.
    ServerOnly,
    /// Both sibling roles above one ingress/coordinator.
    Both,
}

/// Session tuning (bounded queues + client timers).
///
/// Typed at the public boundary: every field is validated where it matters
/// (`queue_capacity == 0` fails [`EndpointSession::new`]; APDU/timer values
/// flow into the client role config unchanged).
///
/// ```
/// use bacnet_endpoint::session::SessionConfig;
///
/// let config = SessionConfig::default();
/// assert!(config.queue_capacity > 0);
/// assert_eq!(config.max_apdu_length, 480);
/// ```
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// Bounded capacity for each ingress queue + egress channel.
    ///
    /// Must be greater than zero; [`EndpointSession::new`] returns
    /// [`Error::Encoding`](bacnet_types::error::Error::Encoding) otherwise.
    pub queue_capacity: usize,
    /// Client APDU timeout (ms).
    pub apdu_timeout_ms: u64,
    /// Client APDU retries.
    pub apdu_retries: u8,
    /// Client max APDU length.
    ///
    /// Standalone default is 480. Composing a
    /// [`DeviceIdentity`](crate::identity::DeviceIdentity) via
    /// [`with_identity`](EndpointSession::with_identity) overrides this with
    /// the identity value; the MS/TP builder additionally rejects identities
    /// above its 480 transport bound at build time.
    pub max_apdu_length: u16,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 16,
            apdu_timeout_ms: 1_000,
            apdu_retries: 0,
            max_apdu_length: 480,
        }
    }
}

/// Snapshot of policy-outcome ownership (session-owned, bounded).
///
/// The session counts every classifier/policy outcome instead of dropping it
/// silently. Sampled via [`policy_counters`](EndpointSession::policy_counters).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PolicyCountersSnapshot {
    /// Ingress classifier outcomes owned by the session.
    pub ingress_policy: u64,
    /// Inbound requests with no server role (client-only).
    pub no_server_role: u64,
    /// Terminal responses with no client/notification role.
    pub no_client_role: u64,
    /// Terminal responses no lease claimed (unknown/peer/service/policy).
    pub unclaimed_terminal: u64,
    /// Responder declined (group/unsupported/closed).
    pub responder_declined: u64,
}

struct PolicyCounters {
    ingress_policy: u64,
    no_server_role: u64,
    no_client_role: u64,
    unclaimed_terminal: u64,
    responder_declined: u64,
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
/// An optional [`DeviceIdentity`](crate::identity::DeviceIdentity) is the
/// single source for I-Am + Device readback + role limits when composed via
/// [`with_identity`](Self::with_identity). Standalone sessions without an
/// identity keep the `SessionConfig` 480 default untouched.
///
/// ```no_run
/// use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
/// use bacnet_transport::loopback::LoopbackTransport;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), bacnet_types::error::Error> {
/// let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
/// let mut session =
///     EndpointSession::new(transport, SessionRole::Both, SessionConfig::default())?;
/// session.start().await?;
/// assert!(session.is_running());
/// session.stop().await?;
/// # Ok(())
/// # }
/// ```
pub struct EndpointSession<T: TransportPort + 'static> {
    shared: Arc<SessionShared>,
    coordinator: Arc<OutboundTransactionCoordinator>,
    ingress: Option<EndpointIngress<T>>,
    requester: Option<EndpointRequester>,
    responder: Option<Arc<EndpointResponder>>,
    notifications: Option<Arc<NotificationTransactions>>,
    client_handle: Option<ClientRoleHandle>,
    server_handle: Option<ServerRoleHandle>,
    dispatch_task: Option<JoinHandle<SessionExit>>,
    cancel_tx: Option<oneshot::Sender<()>>,
    lifecycle: AtomicU8,
    role: SessionRole,
    #[allow(dead_code)]
    config: SessionConfig,
    client_config: ClientConfig,
    database: Option<Arc<RwLock<ObjectDatabase>>>,
    source_audit_reporter: Option<ObjectIdentifier>,
    source_read: Option<Arc<crate::source_read::SourceRead>>,
    source_broadcast: std::net::Ipv4Addr,
    static_source_audit_recipient: Option<crate::bip::StaticSourceAuditRecipient>,
    identity: Option<crate::identity::DeviceIdentity>,
    egress: Option<bacnet_endpoint_core::endpoint_ingress::EndpointEgress>,
}

/// Terminal state of the session dispatch task.
///
/// Returned by [`stop`](EndpointSession::stop): either an explicit stop won
/// the race, the ingress classifier exited, or all dispatch receivers closed.
#[derive(Debug)]
pub enum SessionExit {
    /// Explicit `stop()` won the race.
    Cancelled,
    /// Ingress classifier exited (input closed / policy route full/closed).
    Ingress(ClassifierExit),
    /// Dispatch receivers all closed.
    ReceiversClosed,
}

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// Creates a session owning `transport` (not yet started).
    ///
    /// Returns [`Error::Encoding`](bacnet_types::error::Error::Encoding) when
    /// `config.queue_capacity == 0`. Normally built via
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
        if config.queue_capacity == 0 {
            return Err(Error::Encoding(
                "endpoint session queue capacity must be greater than zero".into(),
            ));
        }
        let coordinator = Arc::new(OutboundTransactionCoordinator::new());
        let token = SessionToken::new(Arc::clone(&coordinator));
        let client_config = ClientConfig {
            apdu_timeout_ms: config.apdu_timeout_ms,
            apdu_retries: config.apdu_retries,
            max_apdu_length: config.max_apdu_length,
            ..ClientConfig::default()
        };
        Ok(Self {
            shared: Arc::new(SessionShared {
                token,
                counters: Mutex::new(PolicyCounters {
                    ingress_policy: 0,
                    no_server_role: 0,
                    no_client_role: 0,
                    unclaimed_terminal: 0,
                    responder_declined: 0,
                }),
            }),
            coordinator,
            ingress: Some(EndpointIngress::new(transport, config.queue_capacity)),
            requester: None,
            responder: None,
            notifications: None,
            client_handle: None,
            server_handle: None,
            dispatch_task: None,
            cancel_tx: None,
            lifecycle: AtomicU8::new(Lifecycle::Ready as u8),
            role,
            config,
            client_config,
            database: None,
            source_audit_reporter: None,
            source_read: None,
            source_broadcast: std::net::Ipv4Addr::BROADCAST,
            static_source_audit_recipient: None,
            identity: None,
            egress: None,
        })
    }

    /// Attaches the local object database (before start).
    ///
    /// Must be called before [`start`](Self::start); when the server role is
    /// composed without a database, an empty one is used. When a
    /// [`DeviceIdentity`](crate::identity::DeviceIdentity) is also composed,
    /// build the database from that same identity (see
    /// [`DeviceIdentity::build_database`](crate::identity::DeviceIdentity::build_database))
    /// so Device readback agrees with I-Am.
    /// The session retains ownership even in `ClientOnly` mode and shares this
    /// same database with the responder in `Both` mode.
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
    /// Without a static recipient this establishes ownership only and emits no
    /// records. A B/IP builder's static recipient enables bounded source READ
    /// reporting; its profile requires absent Monitored_Objects. Standalone
    /// `BACnetClient` source ownership
    /// and source emission remain unsupported. Target Reporter behavior is
    /// unchanged. Repeated calls before start replace the selection.
    ///
    /// [`start`](Self::start) requires a client role, an attached database with
    /// exactly one local Device (matching the composed identity, if present),
    /// and the selected Audit Reporter capability with no other source Reporter.
    /// Validation failure changes no flags and leaves configuration retryable.
    /// Success makes the selected Reporter's source property true and prevents
    /// its deletion; other Reporters remain targets.
    /// The adapter and its construction are private to this endpoint owner;
    /// the wrapped object's Reporter configuration capability remains unchanged.
    ///
    /// ```compile_fail,E0603
    /// use bacnet_endpoint::session::source_reporter::SourceReporter;
    /// ```
    ///
    /// ```no_run
    /// use bacnet_endpoint::{identity::DeviceIdentity, session::{EndpointSession, SessionConfig, SessionRole}};
    /// use bacnet_objects::{audit::AuditReporterObject, traits::BACnetObject};
    /// use bacnet_transport::loopback::LoopbackTransport;
    /// # async fn example() -> Result<(), bacnet_types::error::Error> {
    /// let identity = DeviceIdentity::new(123, 42)?;
    /// let mut db = identity.build_database()?;
    /// let reporter = AuditReporterObject::new(1, "Source configuration")?;
    /// let reporter_oid = reporter.object_identifier();
    /// db.add(Box::new(reporter))?;
    /// let (transport, _peer) = LoopbackTransport::pair(vec![1], vec![2]);
    /// let mut session = EndpointSession::new(transport, SessionRole::ClientOnly, SessionConfig::default())?
    ///     .with_database(db).with_identity(identity).with_source_audit_reporter(reporter_oid);
    /// session.start().await?; // Establishes ownership only; emits no audit traffic.
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
    /// `SessionConfig`. The database should already be built from the same
    /// identity (see `DeviceIdentity::build_database`) so I-Am vs
    /// ReadProperty vs role limits agree. No existing-test churn: sessions
    /// without an identity keep the 480 default.
    ///
    /// # Panics
    /// Panics if startup has already consumed the session configuration.
    pub fn with_identity(mut self, identity: crate::identity::DeviceIdentity) -> Self {
        self.assert_configurable();
        identity.apply_to_client_config(&mut self.client_config);
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

    fn prepare_source_audit_reporter(&mut self) -> Result<(), Error> {
        let Some(selected) = self.source_audit_reporter else {
            if self.static_source_audit_recipient.is_some() {
                return Err(Error::Encoding(
                    "static source audit recipient requires a selected source Audit Reporter"
                        .into(),
                ));
            }
            return Ok(());
        };
        if self.role == SessionRole::ServerOnly {
            return Err(Error::Encoding(
                "source Audit Reporter requires a client role".into(),
            ));
        }
        let db = self.database.as_mut().ok_or_else(|| {
            Error::Encoding("source Audit Reporter requires an attached local database".into())
        })?;
        // Ready sessions have not shared the Arc with a responder. Synchronous,
        // exclusive validation has no await/cancellation or concurrent mutation.
        let db = Arc::get_mut(db)
            .expect("database is unshared before startup")
            .get_mut();
        let devices = db.find_by_type(ObjectType::DEVICE);
        if devices.len() != 1 {
            return Err(Error::Encoding(
                "source Audit Reporter requires exactly one local Device".into(),
            ));
        }
        if self
            .identity
            .as_ref()
            .is_some_and(|identity| devices[0].instance_number() != identity.instance())
        {
            return Err(Error::Encoding(
                "source Audit Reporter local Device does not match session identity".into(),
            ));
        }
        if selected.object_type() != ObjectType::AUDIT_REPORTER {
            return Err(Error::Encoding(
                "source selection must be an Audit Reporter".into(),
            ));
        }
        let object = db.get(&selected).ok_or_else(|| {
            Error::Encoding("selected source Audit Reporter is absent from the database".into())
        })?;
        if !object
            .audit_reporter_internal()
            .is_some_and(|reporter| reporter.object_identifier() == selected)
        {
            return Err(Error::Encoding(
                "selected object lacks the Audit Reporter capability".into(),
            ));
        }
        if self.static_source_audit_recipient.is_some()
            && object.audit_reporter_internal().is_some_and(|reporter| {
                reporter
                    .property_list()
                    .contains(&PropertyIdentifier::MONITORED_OBJECTS)
            })
        {
            return Err(Error::Encoding(
                "source READ does not support Monitored_Objects".into(),
            ));
        }
        for (oid, object) in db.iter_objects() {
            if oid != selected && oid.object_type() == ObjectType::AUDIT_REPORTER {
                match object.read_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER, None) {
                    Ok(PropertyValue::Boolean(false)) => {}
                    Ok(PropertyValue::Boolean(true)) => {
                        return Err(Error::Encoding(
                            "database already contains a conflicting source Audit Reporter".into(),
                        ))
                    }
                    _ => {
                        return Err(Error::Encoding(
                            "cannot determine another Audit Reporter's source ownership".into(),
                        ))
                    }
                }
            }
        }
        // Only this fully validated owner can install the private adapter. The
        // existing slot is wrapped in place: no remove/add, rebind or index churn.
        source_reporter::install(
            db.get_mut(&selected)
                .expect("selected Reporter was validated"),
            self.static_source_audit_recipient.is_some(),
        )
    }

    /// Starts ingress, roles and the single dispatch consumer once.
    ///
    /// Start-once: a second call returns
    /// [`Error::Encoding`](bacnet_types::error::Error::Encoding) without
    /// binding again. Takes `&mut self` so only the owner can start.
    /// Source-profile validation runs before lifecycle consumption or ingress
    /// startup; its errors leave the session ready for correction and retry.
    pub async fn start(&mut self) -> Result<(), Error> {
        if self.lifecycle.load(Ordering::Acquire) != Lifecycle::Ready as u8 {
            return Err(Error::Encoding(
                "endpoint session cannot be started more than once".into(),
            ));
        }
        self.prepare_source_audit_reporter()?;
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
        let ingress = self
            .ingress
            .as_mut()
            .ok_or_else(|| Error::Encoding("endpoint session ingress owner is missing".into()))?;
        let receivers = ingress.start().await?;
        let egress = receivers.egress.clone();

        // Role registration shares ONE coordinator + ONE egress. No second
        // demultiplexer, no role-side Invoke-ID allocation: the requester and
        // notification pool reserve from `self.coordinator`; the responder
        // reuses the wire invoke ID directly.
        let notifications = (matches!(self.role, SessionRole::ServerOnly | SessionRole::Both)
            || self.static_source_audit_recipient.is_some())
        .then(|| NotificationTransactions::with_coordinator(Arc::clone(&self.coordinator)));
        let source_read = self.static_source_audit_recipient.map(|recipient| {
            crate::source_read::SourceRead::new(
                Arc::clone(self.database.as_ref().expect("validated source database")),
                self.source_audit_reporter
                    .expect("validated source Reporter"),
                recipient,
                self.source_broadcast,
                egress.clone(),
                notifications.as_ref().expect("source worker owner"),
                self.client_config.max_apdu_length,
            )
        });
        let (requester, client_handle) =
            if matches!(self.role, SessionRole::ClientOnly | SessionRole::Both) {
                let requester = EndpointRequester::new(
                    egress.clone(),
                    Arc::clone(&self.coordinator),
                    self.client_config.clone(),
                )?;
                let mut handle = ClientRoleHandle::new(&self.shared.token, requester.clone());
                if let Some(source) = &source_read {
                    handle = handle.with_source_read(source);
                }
                (Some(requester), Some(handle))
            } else {
                (None, None)
            };
        let (responder, server_handle) =
            if matches!(self.role, SessionRole::ServerOnly | SessionRole::Both) {
                let db = self
                    .database
                    .get_or_insert_with(|| Arc::new(RwLock::new(ObjectDatabase::new())));
                let responder = Arc::new(EndpointResponder::new(Arc::clone(db), egress.clone()));
                let handle = ServerRoleHandle::new(
                    &self.shared.token,
                    Arc::clone(&responder),
                    Arc::clone(notifications.as_ref().expect("server worker owner")),
                );
                (Some(responder), Some(handle))
            } else {
                (None, None)
            };

        let (cancel_tx, cancel_rx) = oneshot::channel();
        let dispatch = DispatchParts {
            inbound: receivers.inbound_requests,
            terminal: receivers.terminal_or_segment,
            policy: receivers.policy_outcomes,
            requester: requester.clone(),
            responder: responder.clone(),
            notifications: notifications.clone(),
            coordinator: Arc::clone(&self.coordinator),
            shared: Arc::clone(&self.shared),
        };
        let task = tokio::spawn(dispatch_loop(dispatch, cancel_rx));

        // Dispatch owns the three ingress receivers (single consumer); the
        // session retains one egress clone for the identity I-Am path while
        // the receiver halves move into dispatch. No second demultiplexer.
        self.egress = Some(egress);
        self.source_read = source_read;
        self.requester = requester;
        self.responder = responder;
        self.notifications = notifications;
        self.client_handle = client_handle;
        self.server_handle = server_handle;
        self.dispatch_task = Some(task);
        self.cancel_tx = Some(cancel_tx);
        Ok(())
    }

    /// Stops dispatch, roles and ingress once; joins termination.
    ///
    /// Stop-once: returns the dispatch [`SessionExit`]. Calling before
    /// `start()` or twice returns
    /// [`Error::Encoding`](bacnet_types::error::Error::Encoding). Takes
    /// `&mut self` so only the owner can stop; cloned role handles observe
    /// shutdown and fail closed.
    pub async fn stop(&mut self) -> Result<SessionExit, Error> {
        if self.lifecycle.compare_exchange(
            Lifecycle::Running as u8,
            Lifecycle::Stopped as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) != Ok(Lifecycle::Running as u8)
        {
            return Err(Error::Encoding("endpoint session is not running".into()));
        }
        self.shared.token.shutdown();
        if let Some(source) = self.source_read.take() {
            source.close();
        }
        if let Some(requester) = self.requester.take() {
            requester.close();
        }
        if let Some(responder) = self.responder.take() {
            responder.close();
        }
        let notifications = self.notifications.take();
        if let Some(notifications) = &notifications {
            notifications.close();
        }
        if let Some(cancel) = self.cancel_tx.take() {
            let _ = cancel.send(());
        }
        let exit = match self.dispatch_task.take() {
            Some(task) => task
                .await
                .map_err(|e| Error::Encoding(format!("endpoint session dispatch failed: {e}"))),
            None => Ok(SessionExit::ReceiversClosed),
        };
        if let Some(notifications) = notifications {
            while let Some(result) = notifications.join_next().await {
                NotificationTransactions::observe(Some(result));
            }
        }
        if let Some(ingress) = self.ingress.as_mut() {
            // Ingress stop reports its classifier exit; session exit above
            // already owns termination, so a closed ingress here is expected.
            let _ = ingress.stop().await;
        }
        // Dispatch owned the ingress receivers (single consumer); dropping
        // the task + ingress ends them with no detached queue outliving
        // termination.
        self.client_handle = None;
        self.server_handle = None;
        exit
    }

    /// Borrows the client role (`None` when not composed).
    ///
    /// Returns `None` for [`ServerOnly`](SessionRole::ServerOnly) sessions,
    /// and after [`stop`](Self::stop) (handles are released at termination).
    pub fn client(&self) -> Option<&ClientRoleHandle> {
        self.client_handle.as_ref()
    }

    /// Borrows the server role (`None` when not composed).
    ///
    /// Returns `None` for [`ClientOnly`](SessionRole::ClientOnly) sessions,
    /// and after [`stop`](Self::stop).
    pub fn server(&self) -> Option<&ServerRoleHandle> {
        self.server_handle.as_ref()
    }

    /// Clones the client role handle with its session binding.
    ///
    /// The clone holds only a [`Weak`](std::sync::Weak) session token: it
    /// works while the session runs and fails closed after `stop()`/drop.
    /// Used to hand the client role to tasks that outlive the borrow.
    pub fn cloned_client_handle(&self) -> Option<ClientRoleHandle> {
        self.client_handle.clone()
    }

    /// Clones the server role handle with its session binding.
    ///
    /// Same [`Weak`](std::sync::Weak)-token semantics as
    /// [`cloned_client_handle`](Self::cloned_client_handle); see
    /// [`is_session_alive`](crate::roles::ServerRoleHandle::is_session_alive).
    pub fn cloned_server_handle(&self) -> Option<ServerRoleHandle> {
        self.server_handle.clone()
    }

    /// Samples policy-outcome ownership counters.
    ///
    /// Every classifier/policy outcome is counted, never silently dropped.
    pub async fn policy_counters(&self) -> PolicyCountersSnapshot {
        let counters = self.shared.counters.lock().await;
        PolicyCountersSnapshot {
            ingress_policy: counters.ingress_policy,
            no_server_role: counters.no_server_role,
            no_client_role: counters.no_client_role,
            unclaimed_terminal: counters.unclaimed_terminal,
            responder_declined: counters.responder_declined,
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
    /// Present-identity path: encodes [`DeviceIdentity::encode_iam_apdu`](crate::identity::DeviceIdentity::encode_iam_apdu)
    /// and sends one local-broadcast Unconfirmed-Request via the session
    /// egress. Field-for-field identical to the server discovery I-Am built
    /// from [`DeviceIdentity::server_config`](crate::identity::DeviceIdentity::server_config).
    /// Transport-dependent: B/IP reaches the local subnet broadcast; SC
    /// relays via the hub as a broadcast NPDU. Fails when the session is not
    /// running.
    pub async fn broadcast_i_am(&self) -> Result<(), Error> {
        let identity = self
            .identity
            .as_ref()
            .ok_or_else(|| Error::Encoding("endpoint I-Am requires a composed identity".into()))?;
        let egress = self
            .egress
            .as_ref()
            .ok_or_else(|| Error::Encoding("endpoint session is not running".into()))?;
        if !self.is_running() {
            return Err(Error::Encoding("endpoint session is not running".into()));
        }
        let apdu = identity.encode_iam_apdu()?;
        egress
            .send_apdu(
                apdu,
                bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::LocalBroadcast,
                false,
                bacnet_types::enums::NetworkPriority::NORMAL,
                Vec::new(),
            )
            .await
    }
}

impl EndpointSession<bacnet_transport::bip::BipTransport> {
    // Only the concrete B/IP builder supplies this validated value. No public
    // setter, runtime update, or transport-neutral recipient API is exposed.
    pub(crate) fn with_static_source_audit_recipient(
        mut self,
        recipient: crate::bip::StaticSourceAuditRecipient,
        broadcast: std::net::Ipv4Addr,
    ) -> Self {
        self.assert_configurable();
        self.static_source_audit_recipient = Some(recipient);
        self.source_broadcast = broadcast;
        self
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
        if let Some(source) = self.source_read.take() {
            source.close();
        }
        if let Some(requester) = self.requester.take() {
            requester.close();
        }
        if let Some(responder) = self.responder.take() {
            responder.close();
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
    }
}

struct DispatchParts {
    inbound: mpsc::Receiver<ReceivedApdu>,
    terminal: mpsc::Receiver<ReceivedApdu>,
    policy: mpsc::Receiver<PolicyOutcome>,
    requester: Option<EndpointRequester>,
    responder: Option<Arc<EndpointResponder>>,
    notifications: Option<Arc<NotificationTransactions>>,
    coordinator: Arc<OutboundTransactionCoordinator>,
    shared: Arc<SessionShared>,
}

async fn dispatch_loop(mut parts: DispatchParts, mut cancel: oneshot::Receiver<()>) -> SessionExit {
    // Single ingress consumer: this task alone polls all three receivers.
    // Every await is cancellation-safe: leases are held by RAII guards
    // (request guard / notification operation) so dropping this future
    // cannot strand a coordinator slot.
    loop {
        tokio::select! {
            biased;
            _ = &mut cancel => return SessionExit::Cancelled,
            joined = async {
                match &parts.notifications {
                    Some(owner) => owner.join_next().await,
                    None => std::future::pending().await,
                }
            } => NotificationTransactions::observe(joined),
            received = parts.inbound.recv() => {
                let Some(received) = received else {
                    // Inbound closed: keep terminal/policy draining; ingress
                    // closure is reported via policy/terminal shutdown below.
                    // If both are also closed, exit.
                    if parts.terminal.is_closed() && parts.policy.is_closed() {
                        return SessionExit::ReceiversClosed;
                    }
                    continue;
                };
                handle_inbound(&mut parts, received).await;
            }
            received = parts.terminal.recv() => {
                let Some(received) = received else {
                    if parts.inbound.is_closed() && parts.policy.is_closed() {
                        return SessionExit::ReceiversClosed;
                    }
                    continue;
                };
                handle_terminal(&mut parts, received).await;
            }
            outcome = parts.policy.recv() => {
                let Some(outcome) = outcome else {
                    if parts.inbound.is_closed() && parts.terminal.is_closed() {
                        return SessionExit::ReceiversClosed;
                    }
                    continue;
                };
                handle_ingress_policy(&parts.shared, outcome).await;
            }
        }
    }
}

async fn handle_inbound(parts: &mut DispatchParts, received: ReceivedApdu) {
    // Preserve the full envelope structurally (raw + effective group,
    // attributes, ingress identity, provenance) — no new decisions here
    // beyond role presence + responder scope.
    let _peer = inbound_canonical_peer(&received);
    let _link_group = received.link_layer_group;
    let _is_group = received.is_group;
    let _attributes = received.data_attributes.clone();
    let _provenance = received.provenance;
    let _ingress = received.ingress_network;
    let Some(responder) = parts.responder.as_ref() else {
        let mut counters = parts.shared.counters.lock().await;
        counters.no_server_role += 1;
        // One-use reply ownership: release a lone reply sender without
        // sending bytes, mirroring queue-admission drop semantics.
        if let Some(reply_tx) = received.reply_tx {
            drop(reply_tx);
        }
        return;
    };
    // RB-17 deferred-reply wiring (minimal correlation, session-side): when
    // the one-shot suspension arm is set and this request carries the MS/TP
    // one-use prompt reply sender, drop the sender BEFORE the responder sees
    // it. Dropping releases the MAC to send ReplyPostponed; the responder
    // then observes `reply_tx: None` and answers through the single
    // EndpointEgress → queue_npdu path, which the MAC transmits as
    // DataNotExpectingReply at the next token opportunity with the identical
    // wire invoke ID + destination. Prompt (sender present, unarmed) and
    // token-owned (sender absent) stay distinct by construction: the
    // responder never sees both for one request. No second serial owner, no
    // MAC state duplication, no transport change.
    let mut received = received;
    if received.reply_tx.is_some() && parts.shared.token.take_suspend() {
        let _ = received.reply_tx.take();
    }
    match responder.handle(received).await {
        Ok(true) | Ok(false) => {}
        Err(_) => {
            let mut counters = parts.shared.counters.lock().await;
            counters.responder_declined += 1;
        }
    }
}

async fn handle_terminal(parts: &mut DispatchParts, received: ReceivedApdu) {
    let Some(apdu) = decode_terminal(&received) else {
        let mut counters = parts.shared.counters.lock().await;
        counters.unclaimed_terminal += 1;
        if let Some(reply_tx) = received.reply_tx {
            drop(reply_tx);
        }
        return;
    };
    // Inbound server transactions never allocate here: requests are not
    // terminal traffic. A Confirmed/Unconfirmed request in this queue is a
    // classifier violation → policy-owned, no coordinator interaction.
    if matches!(
        apdu,
        Apdu::ConfirmedRequest(_) | Apdu::UnconfirmedRequest(_)
    ) {
        let mut counters = parts.shared.counters.lock().await;
        counters.unclaimed_terminal += 1;
        if let Some(reply_tx) = received.reply_tx {
            drop(reply_tx);
        }
        return;
    }
    // Exactly one shared-coordinator admit per terminal APDU (exact-once
    // terminal claim). Equal numeric IDs are unambiguous: the admitted
    // lease owner selects requester vs notification.
    let outcome = admit_once(&parts.coordinator, &received, &apdu);
    let admission = match outcome {
        Ok(AdmissionOutcome::Admitted(admission)) => admission,
        Ok(_) => {
            let mut counters = parts.shared.counters.lock().await;
            counters.unclaimed_terminal += 1;
            if let Some(reply_tx) = received.reply_tx {
                drop(reply_tx);
            }
            return;
        }
        Err(CoordinatorError::StatePoisoned) => {
            let mut counters = parts.shared.counters.lock().await;
            counters.unclaimed_terminal += 1;
            if let Some(reply_tx) = received.reply_tx {
                drop(reply_tx);
            }
            return;
        }
    };
    if is_requester_lease(&admission) {
        let Some(requester) = parts.requester.as_ref() else {
            // Lease was admitted but has no consumer: release the exact
            // lease so exhaustion → release → reuse still holds.
            let _ = parts.coordinator.release(admission.token());
            let mut counters = parts.shared.counters.lock().await;
            counters.no_client_role += 1;
            if let Some(reply_tx) = received.reply_tx {
                drop(reply_tx);
            }
            return;
        };
        // `complete_pre_admitted` delivers via the exact token without
        // re-admitting; a `false` means the TSM no longer owns it (lost
        // consumer / stale), the coordinator lease was already completed
        // inside the TSM path on success.
        let _ = requester
            .complete_pre_admitted(admission, apdu, received)
            .await;
    } else {
        let Some(notifications) = parts.notifications.as_ref() else {
            let _ = parts.coordinator.release(admission.token());
            let mut counters = parts.shared.counters.lock().await;
            counters.no_client_role += 1;
            if let Some(reply_tx) = received.reply_tx {
                drop(reply_tx);
            }
            return;
        };
        let _ = notifications.complete_pre_admitted(admission, &apdu);
        // Notification replies carry no reply sender; drop one if present.
        if let Some(reply_tx) = received.reply_tx {
            drop(reply_tx);
        }
    }
}

async fn handle_ingress_policy(shared: &Arc<SessionShared>, outcome: PolicyOutcome) {
    // Policy-outcome ownership: the session counts every classifier outcome
    // (malformed/unsupported/route-full/route-closed) instead of dropping it.
    let mut counters = shared.counters.lock().await;
    counters.ingress_policy += 1;
    let _ = outcome.reason;
    // Policy outcomes own the full envelope including a possible one-use
    // reply sender; releasing here preserves single-consumer + one-use
    // ownership without sending bytes.
    if let Some(reply_tx) = outcome.received.reply_tx {
        drop(reply_tx);
    }
    let _ = PolicyReason::MalformedApdu;
}

#[cfg(test)]
#[path = "source_read_tests.rs"]
mod source_read_tests;
