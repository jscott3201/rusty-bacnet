//! A full server on a node's link (#2617): what differs when the server
//! starts on a [`ServerSeat`] in place of a transport of its own.
//!
//! The server's start is generic over a [`Link`]. [`Own`] is a transport the
//! server starts, answers the local network number for and stops: the code
//! a standalone server has always run, step for step. [`Seat`] is a node's
//! started network layer, shared, with the node's pool of invoke IDs and the
//! APDUs the node's ingress hands the server. The dispatch loop and every
//! request handler are the same code either way; nothing a request touches
//! differs.
//!
//! On a seat the node owns the link:
//!
//! - its one Number owner answers What-Is-Network-Number and publishes the
//!   number to the shared layer, so the server runs none and reads what it
//!   publishes;
//! - the server's invoke IDs come from the node's pool, so they never clash
//!   with a seated client's;
//! - the server's [`stop`](BACnetServer::stop) joins its own work and
//!   settles its saves as always, but neither seals the shared layer's
//!   replies (another role answers through them) nor stops the layer: the
//!   node does that, and once it has, what the server still sends fails with
//!   the stopped transport's error.
//!
//! The node is one device (#2617, PR 2):
//!
//! - its DeviceCommunicationControl state is the server's: what the server
//!   accepts, and its duration, an ENABLE or a restart, reaches the node's
//!   client too, and the node enables initiation again when the server
//!   stops or is dropped;
//! - with the node's device settings on the seat, the Device the server
//!   serves states the node's: the node's identity, the smaller of the two
//!   roles' largest accepted APDUs, one APDU timeout and retry count, and
//!   the client's executed services beside the server's;
//! - target Audit follows the number the node's Number owner publishes.
//!
//! A registered Network Port is the node's: the node reserves it before its
//! link starts and publishes it after, and a seated build takes the port it
//! names ([`SeatedServerBuilder::node_registered_port`]) for wildcard reads,
//! refusing one the builder names itself.
//!
//! The seat holds at most a quarter of the node's invoke IDs at once
//! ([`SEAT_LEASES`](super::notification_transactions::SEAT_LEASES)), so the
//! node's client keeps the rest.

use std::any::Any;
use std::future::Future;

use bacnet_endpoint_core::seat::{NodeDevice, SeatHolds, SeatNumber, ServerDcc, ServerSeat};
use bacnet_network::layer::{ApduReceiver, ReceivedApdu, ReceivedNetworkControl};

use super::audit_recipient_routes::AuditRoutes;
use super::*;
use crate::device_view::DeviceExecution;

/// Where the dispatch loop takes its APDUs from.
pub(super) trait Apdus: Send + 'static {
    /// The next APDU, or `None` once the source has closed. Cancel safe.
    fn recv(&mut self) -> impl Future<Output = Option<ReceivedApdu>> + Send + '_;
}

/// A standalone server's own layer: the very future it always awaited.
impl Apdus for ApduReceiver {
    #[inline(always)]
    fn recv(&mut self) -> impl Future<Output = Option<ReceivedApdu>> + Send + '_ {
        ApduReceiver::recv(self)
    }
}

/// A seat's queue, which the node's ingress fills, and the node's word that
/// the server may start taking from it.
pub(super) struct SeatApdus {
    /// Until the node lets go of it, or drops it, nothing is taken: what
    /// ingress hands the server waits in the queue.
    gate: Option<oneshot::Receiver<()>>,
    queue: mpsc::Receiver<ReceivedApdu>,
}

impl Apdus for SeatApdus {
    // The trait asks for a `Send` future, which an `async fn` here would
    // leave the compiler to infer.
    #[allow(clippy::manual_async_fn)]
    fn recv(&mut self) -> impl Future<Output = Option<ReceivedApdu>> + Send + '_ {
        async move {
            // Cancel safe: an unfinished wait is polled again, and a
            // finished one is let go of in the same poll.
            if let Some(gate) = self.gate.as_mut() {
                let _ = gate.await;
                self.gate = None;
            }
            self.queue.recv().await
        }
    }
}

/// What a link gives the server's start once it is up.
pub(super) struct Started<T: TransportPort, A> {
    pub(super) network: Arc<NetworkLayer<T>>,
    pub(super) apdus: A,
    pub(super) audit_routes: Arc<AuditRoutes>,
    /// The Number controls, when the server answers for the network number.
    pub(super) controls: Option<mpsc::Receiver<ReceivedNetworkControl>>,
    pub(super) notification_transactions: Arc<NotificationTransactions>,
    pub(super) seated: Option<Seated>,
    /// The Device's execution profile: the server's alone, or on a node
    /// with a client role the node's, with time synchronization as the
    /// server's policy executes it (#2617).
    pub(super) execution: DeviceExecution,
    /// On a seat, the server's hold on the node's DeviceCommunicationControl
    /// state.
    pub(super) dcc: Option<ServerDcc>,
    /// On a seat, the notice that the node's Number owner published the
    /// network number.
    pub(super) number: Option<SeatNumber>,
}

/// How the server comes by its network layer.
pub(super) trait Link<T: TransportPort + 'static>: Send + Sized + 'static {
    /// Where its APDUs come from.
    type Apdus: Apdus;

    /// The transport, before the link is up.
    fn transport(&self) -> &T;

    /// Bring the link up, with the registered Network Port and the Audit
    /// routes as the server configured them.
    fn start<'a>(
        self,
        db: &'a mut ObjectDatabase,
        config: &'a ServerConfig,
        routes: AuditRoutes,
    ) -> impl Future<Output = Result<Started<T, Self::Apdus>, Error>> + Send + 'a;

    /// Let go of a layer the start brought up, after a later step failed.
    fn abandon(network: Arc<NetworkLayer<T>>) -> impl Future<Output = Result<(), Error>> + Send;
}

/// A transport of the server's own.
pub(super) struct Own<T>(pub(super) T);

impl<T: TransportPort + 'static> Link<T> for Own<T> {
    type Apdus = ApduReceiver;

    fn transport(&self) -> &T {
        &self.0
    }

    async fn start(
        self,
        db: &mut ObjectDatabase,
        config: &ServerConfig,
        routes: AuditRoutes,
    ) -> Result<Started<T, ApduReceiver>, Error> {
        let (network, apdus, audit_routes, controls) =
            super::network_port::start(db, config, self.0, routes).await?;
        Ok(Started {
            network: Arc::new(network),
            apdus,
            audit_routes,
            controls,
            notification_transactions: NotificationTransactions::new(),
            seated: None,
            execution: DeviceExecution::server(false, &config.time_sync_policy),
            dcc: None,
            number: None,
        })
    }

    async fn abandon(network: Arc<NetworkLayer<T>>) -> Result<(), Error> {
        let network = Arc::try_unwrap(network).map_err(|_| {
            Error::Encoding("startup cleanup has an unexpected network owner".into())
        })?;
        super::heap_futures::boxed(|| {
            super::network_port::StartingNetwork::from_network(network).cleanup()
        })
        .await
    }
}

/// A node's seat, and the node's gate on its queue.
pub(super) struct Seat<T: TransportPort>(ServerSeat<T>, Option<oneshot::Receiver<()>>);

impl<T: TransportPort + 'static> Link<T> for Seat<T> {
    type Apdus = SeatApdus;

    fn transport(&self) -> &T {
        self.0.network.transport()
    }

    async fn start(
        self,
        db: &mut ObjectDatabase,
        config: &ServerConfig,
        routes: AuditRoutes,
    ) -> Result<Started<T, Self::Apdus>, Error> {
        let ServerSeat {
            network,
            coordinator,
            apdus,
            holds,
            number,
            dcc,
            node,
            ..
        } = self.0;
        // The node's link is up already: the Audit routes take its facts
        // now. A refusal leaves the link as it is; it is the node's.
        let audit_routes = routes.settle(db, config, &network)?;
        Ok(Started {
            network,
            apdus: SeatApdus {
                gate: self.1,
                queue: apdus,
            },
            audit_routes: Arc::new(audit_routes),
            controls: None,
            notification_transactions: NotificationTransactions::with_lease_cap(
                coordinator,
                super::notification_transactions::SEAT_LEASES,
            ),
            seated: Some(Seated::new(&holds)),
            execution: DeviceExecution::server(
                node.is_some_and(|node| node.client),
                &config.time_sync_policy,
            ),
            dcc: Some(dcc),
            number: Some(number),
        })
    }

    async fn abandon(_: Arc<NetworkLayer<T>>) -> Result<(), Error> {
        // The node's layer: nothing to stop here.
        Ok(())
    }
}

/// A server's hold on its seat: the seat takes another server once this
/// drops, without waiting for the dropped server's tasks to end.
pub(super) struct Seated {
    _alive: Arc<dyn Any + Send + Sync>,
}

impl Seated {
    fn new(holds: &SeatHolds) -> Self {
        let alive: Arc<dyn Any + Send + Sync> = Arc::new(());
        holds.alive_while(Arc::downgrade(&alive));
        Self { _alive: alive }
    }
}

/// A [`ServerBuilder`] with a seat in place of a transport, from
/// [`ServerBuilder::seat`].
pub struct SeatedServerBuilder<T: TransportPort> {
    config: ServerConfig,
    db: Option<ObjectDatabase>,
    seat: ServerSeat<T>,
    configured_device_bindings: Vec<DeviceBinding>,
    clock_zone: ClockConfig,
    /// Whether the builder had a transport of its own set, which a seated
    /// build refuses.
    had_transport: bool,
    /// The node's Device identity, which the server serves on a seat that
    /// carries the node's device settings.
    node_identity: Option<bacnet_objects::DeviceIdentity>,
    /// The node's gate on the seat's queue.
    gate: Option<oneshot::Receiver<()>>,
    /// The node's registered Network Port, which it reserved and published.
    node_port: Option<ObjectIdentifier>,
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Run the server on `seat`, a node's started link, in place of a
    /// transport of its own (#2617). Set the builder's other options first.
    /// A builder that already has a transport is refused when it builds:
    /// the server takes one link or the other. See
    /// [`SeatedServerBuilder::build`] for what a seat changes.
    pub fn seat(self, seat: ServerSeat<T>) -> SeatedServerBuilder<T> {
        SeatedServerBuilder {
            config: self.config,
            db: self.db,
            seat,
            configured_device_bindings: self.configured_device_bindings,
            clock_zone: self.clock_zone,
            had_transport: self.transport.is_some(),
            node_identity: None,
            gate: None,
            node_port: None,
        }
    }

    /// What a node checks of this builder before its link starts (#2617).
    #[doc(hidden)]
    pub fn node_facts(&self) -> NodeBuilderFacts {
        NodeBuilderFacts {
            transport: self.transport.is_some(),
            database: self.db.is_some(),
            registered_network_port: self.config.registered_network_port.is_some(),
            audit_reporters: self.config.audit_reporters.is_some(),
            decides_mutations: self.config.mutation_policy
                == crate::mutation::MutationPolicy::DenyAll
                || self
                    .config
                    .policy
                    .as_ref()
                    .is_some_and(|policy| policy.decides_mutations()),
            reinitialize_handler: self.config.on_reinitialize.is_some(),
        }
    }

    /// Refuse, before a node's link starts, what a seated build of this
    /// builder would refuse without the started link (#2617), so the node
    /// stays as it was: an identity, `cov_retry_timeout_ms` or
    /// `notification_retries` that contradicts the node's, a database
    /// Device, the node's own `db`, that declares other APDU timing, and
    /// whatever every start checks of its settings (its policies and
    /// budgets, the Device's segment timeout, the device bindings against
    /// the link's group rule, the Audit Reporters and their routes), on the
    /// settings the seat gives the server. Checking the Audit routes marks
    /// each Reporter's route as a start does, which the start marks again.
    #[doc(hidden)]
    pub fn check_node(
        &self,
        identity: &bacnet_objects::DeviceIdentity,
        node: NodeDevice,
        link: &NodeLink<'_>,
        db: Option<&mut ObjectDatabase>,
    ) -> Result<(), Error> {
        node_contradictions(&self.config, identity, node, link.capacity, db.as_deref())?;
        let mut config = self.config.clone();
        config.device = identity
            .resolve(node.client_max_apdu_length)?
            .resolve(link.capacity)?;
        config.cov_retry_timeout_ms = node.apdu_timeout_ms;
        config.notification_retries = node.apdu_retries;
        let bindings = self.configured_device_bindings.clone();
        let settled = super::lifecycle::settings::settle(&mut config, bindings, link.is_group)?;
        if let Some(db) = db {
            crate::local_device::validate_startup(db, &config.device)?;
            let bindings = &settled.device_bindings;
            AuditRoutes::prepare_for(db, &config, bindings, link.bip_broadcast, link.is_group)?;
        }
        Ok(())
    }
}

/// What a node's link says of itself before it starts (#2617), for
/// [`ServerBuilder::check_node`].
#[doc(hidden)]
pub struct NodeLink<'a> {
    /// The largest APDU the link will accept.
    pub capacity: u16,
    /// The link's own group check, its transport's
    /// [`TransportPort::is_group_destination`], which a server's start asks.
    pub is_group: &'a dyn Fn(&[u8]) -> bool,
    /// The link's B/IP broadcast endpoint, on B/IP.
    pub bip_broadcast: Option<std::net::SocketAddrV4>,
}

/// What a node checks of a [`ServerBuilder`] before it builds the server on
/// its seat, so a refusal leaves its link unstarted (#2617).
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeBuilderFacts {
    /// A transport of its own is set.
    pub transport: bool,
    /// A database is set.
    pub database: bool,
    /// A registered Network Port is named.
    pub registered_network_port: bool,
    /// Target Audit Reporters are configured.
    pub audit_reporters: bool,
    /// Something decides the covered mutations: an installed policy that
    /// decides them, or [`MutationPolicy::DenyAll`](crate::mutation::MutationPolicy::DenyAll).
    pub decides_mutations: bool,
    /// A ReinitializeDevice handler is set.
    pub reinitialize_handler: bool,
}

/// Follow the node's Number owner for target Audit: each publication may
/// change whether its recipient resolves (#1460), as the server's own Number
/// owner reports one. Ends with the server's Audit.
pub(super) fn follow_number<T: TransportPort + 'static>(
    number: SeatNumber,
    audit: std::sync::Weak<super::audit_recipient::TargetAudit<T>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            number.changed().await;
            let Some(audit) = audit.upgrade() else {
                break;
            };
            audit.number_changed();
        }
    })
}

/// A setting a seated build was given that contradicts the node's.
fn contradicts(what: &str) -> Error {
    Error::Encoding(format!(
        "a server on a node's seat takes the node's {what}: the builder set a different one"
    ))
}

/// The value the supplied database's Device declares for `property`, when
/// it declares an Unsigned one.
fn declared(
    db: &ObjectDatabase,
    device: &ObjectIdentifier,
    property: PropertyIdentifier,
) -> Option<u64> {
    match db.get(device)?.read_property(property, None) {
        Ok(PropertyValue::Unsigned(value)) => Some(value),
        _ => None,
    }
}

impl<T: TransportPort + 'static> SeatedServerBuilder<T> {
    /// Build and start the server on its seat: the node's started network
    /// layer and its pool of invoke IDs, shared with the node's other roles.
    ///
    /// The server is the one a transport of its own gives, with the same
    /// services, policies and counters. It answers what the node hands it:
    /// every request but a ConfirmedCOVNotification, which is a seated
    /// client's, the answers to its own requests, and a requester's Abort or
    /// Segment-ACK. What differs:
    ///
    /// - the node has the one owner of the local network number, so the
    ///   server answers no What-Is-Network-Number and takes the number the
    ///   node publishes;
    /// - [`BACnetServer::stop`] stops the server alone: the node stops the
    ///   link. Stop the server first for a graceful end. When the node stops
    ///   first, the server's queue closes, it runs on for its own
    ///   notifications, and what it sends fails with the stopped
    ///   transport's error;
    /// - the node is one device. A DeviceCommunicationControl the server
    ///   accepts binds the node's client too (Clause 16.1), until its
    ///   duration runs out, an ENABLE or an accepted WARMSTART or COLDSTART,
    ///   or the server stops or is dropped;
    /// - when the seat carries the node's device settings, as the
    ///   endpoint's session gives it, the server serves the node's Device:
    ///   the node's identity, with Max_APDU_Length_Accepted the smaller of
    ///   the two roles' (Clause 12.11.18); the node's APDU_Timeout and
    ///   Number_Of_APDU_Retries, which its confirmed notifications use too
    ///   (`cov_retry_timeout_ms`, `notification_retries`); and, on a node
    ///   with a client role, Protocol_Services_Supported listing I-Am,
    ///   I-Have and both COV notifications beside the server's services.
    ///   Segmentation_Supported is the identity's, which the server follows;
    ///   the node's client keeps its own segmentation settings for now;
    /// - target Audit routes its recipient by the number the node publishes.
    ///
    /// Fails, before it changes anything, on a builder that had a transport
    /// of its own set, on one with a registered Network Port, which a seat
    /// can't take yet, on a Device identity, `cov_retry_timeout_ms` or
    /// `notification_retries` the builder set that differ from the node's
    /// (an identity is compared after both are resolved to the link, and
    /// without its service list, which a full server builds itself),
    /// on a database whose Device declares another APDU_Timeout or
    /// Number_Of_APDU_Retries than the node's (one that declares the object
    /// model's defaults, 6000 ms and 3, is given the node's), and on
    /// whatever a transport of its own would refuse. Must be called within
    /// a Tokio runtime.
    pub async fn build(mut self) -> Result<BACnetServer<T>, Error> {
        if self.had_transport {
            return Err(Error::Encoding(
                "a server builder with a transport of its own takes no seat as well".into(),
            ));
        }
        if self.config.registered_network_port.is_some() {
            return Err(Error::Encoding(
                "a server on a node's seat takes no registered Network Port of its own: the \
                 node reserves and publishes its port"
                    .into(),
            ));
        }
        let node = self.seat.node;
        let state_timing = match node {
            Some(node) => self.take_node(node)?,
            None => false,
        };
        let mut db = crate::local_device::prepare_database(
            &mut self.config,
            self.db,
            self.seat.network.transport().local_receive_apdu_capacity(),
        )?;
        // The node's port serves wildcard reads, and written values take
        // effect through this server's ReinitializeDevice handler, as on a
        // server's own link.
        if let Some(oid) = self.node_port {
            db.set_network_port_activation_internal(oid, self.config.on_reinitialize.is_some())?;
            self.config.registered_network_port = Some(oid);
        }
        // The Device states the node's timing: a stock one, and a supplied
        // one that declares the object model's defaults.
        if let (Some(node), true) = (node, state_timing)
            && let Some(mut device) = db
                .get_mut(&self.config.device.device_oid())
                .and_then(|device| device.device_authority_internal())
        {
            device.set_apdu_timing(
                u32::try_from(node.apdu_timeout_ms).unwrap_or(u32::MAX),
                u32::from(node.apdu_retries),
            );
        }
        BACnetServer::start_on_link(
            self.config,
            db,
            Seat(self.seat, self.gate),
            Some(self.clock_zone),
            self.configured_device_bindings,
        )
        .await
    }
}

impl<T: TransportPort + 'static> SeatedServerBuilder<T> {
    /// The node's Device identity, which the server serves when the seat
    /// carries the node's device settings (the endpoint's session gives
    /// both).
    #[doc(hidden)]
    pub fn node_identity(mut self, identity: bacnet_objects::DeviceIdentity) -> Self {
        self.node_identity = Some(identity);
        self
    }

    /// Take nothing from the seat's queue until `gate` is sent or dropped:
    /// the node finishes its own start first, and what arrives meanwhile
    /// waits in the queue, which the node's ingress bounds (#2617). The
    /// hold delays only the server's reading of its queue: its stop and
    /// drop end its dispatch whether or not the node has let go.
    #[doc(hidden)]
    pub fn hold_requests_until(mut self, gate: oneshot::Receiver<()>) -> Self {
        self.gate = Some(gate);
        self
    }

    /// Serve `oid`, the registered Network Port the node reserved and
    /// published, for wildcard reads (#2617).
    #[doc(hidden)]
    pub fn node_registered_port(mut self, oid: ObjectIdentifier) -> Self {
        self.node_port = Some(oid);
        self
    }

    /// Tell `observer` of each ReinitializeDevice the server accepts: the
    /// node's source Audit records it (#2617).
    #[doc(hidden)]
    pub fn on_accepted_reinitialize(
        mut self,
        observer: super::reinitialize::EndpointReinitializeAudit,
    ) -> Self {
        self.config.accepted_reinitialize_audit = Some(observer);
        self
    }

    /// Take the node's device settings in place of the builder's, refusing
    /// any the builder set that differ from them (#2617). Whether the
    /// Device is to be given the node's timing: a stock one is, and a
    /// supplied one that declares the object model's defaults, which no
    /// identity-built database can set otherwise.
    fn take_node(&mut self, node: NodeDevice) -> Result<bool, Error> {
        let identity = self.node_identity.take().ok_or_else(|| {
            Error::Encoding("a server on a node's seat needs the node's Device identity".into())
        })?;
        let capacity = self.seat.network.transport().local_receive_apdu_capacity();
        let state_timing =
            node_contradictions(&self.config, &identity, node, capacity, self.db.as_ref())?;
        // The node accepts the smaller of its roles' largest APDUs
        // (Clause 12.11.18); the link's capacity bounds it further at start.
        self.config.device = identity.resolve(node.client_max_apdu_length)?;
        self.config.cov_retry_timeout_ms = node.apdu_timeout_ms;
        self.config.notification_retries = node.apdu_retries;
        Ok(state_timing)
    }
}

/// Refuse the device settings in `config` and `db` that contradict the
/// node's, as [`SeatedServerBuilder::take_node`] takes them: whether the
/// Device is to be given the node's timing.
fn node_contradictions(
    config: &ServerConfig,
    identity: &bacnet_objects::DeviceIdentity,
    node: NodeDevice,
    capacity: u16,
    db: Option<&ObjectDatabase>,
) -> Result<bool, Error> {
    let defaults = ServerConfig::default();
    // Compared as the link serves them: the node resolved its identity
    // against the link at start, so on a link that takes less than the
    // identity asks (480 octets on MS/TP) the two differ only there. The
    // identity's service list is left out: the server serves the node's
    // executed services whatever it says.
    let served = |identity: &bacnet_objects::DeviceIdentity| {
        identity
            .resolve(capacity)
            .map(|resolved| resolved.with_services(&[]))
    };
    if config.device == defaults.device {
        tracing::debug!("a seated server serves the node's Device identity");
    } else if served(&config.device)? != served(identity)? {
        return Err(contradicts("Device identity"));
    } else if config.device.services() != identity.services() {
        tracing::debug!(
            "a seated server serves the node's Device identity in place of the builder's, \
                 which differs only in its service list"
        );
    }
    let timing = [
        (
            "cov_retry_timeout_ms",
            config.cov_retry_timeout_ms,
            defaults.cov_retry_timeout_ms,
            node.apdu_timeout_ms,
        ),
        (
            "notification_retries",
            u64::from(config.notification_retries),
            u64::from(defaults.notification_retries),
            u64::from(node.apdu_retries),
        ),
    ];
    for (setting, set, default, node) in timing {
        if set == default {
            tracing::debug!(
                setting,
                default,
                node,
                "a seated server takes the node's value"
            );
        } else if set != node {
            return Err(contradicts(match setting {
                "cov_retry_timeout_ms" => "APDU timeout (cov_retry_timeout_ms)",
                _ => "APDU retries (notification_retries)",
            }));
        }
    }
    let mut state_timing = true;
    if let Some(db) = db {
        let device = identity.device_oid();
        let declared = (
            declared(db, &device, PropertyIdentifier::APDU_TIMEOUT),
            declared(db, &device, PropertyIdentifier::NUMBER_OF_APDU_RETRIES),
        );
        let model = bacnet_objects::device::DeviceConfig::default();
        let defaults = (
            Some(u64::from(model.apdu_timeout)),
            Some(u64::from(model.apdu_retries)),
        );
        let node_timing = (
            Some(node.apdu_timeout_ms),
            Some(u64::from(node.apdu_retries)),
        );
        state_timing = declared == defaults;
        if state_timing {
            tracing::debug!(
                apdu_timeout_ms = node.apdu_timeout_ms,
                apdu_retries = node.apdu_retries,
                "the database's Device declares the object model's default APDU timing: \
                     it is given the node's"
            );
        } else if declared != node_timing && declared != (None, None) {
            return Err(Error::Encoding(format!(
                "a server on a node's seat serves the node's APDU_Timeout ({} ms) and \
                     Number_Of_APDU_Retries ({}): the database's Device declares others",
                node.apdu_timeout_ms, node.apdu_retries
            )));
        }
    }
    Ok(state_timing)
}

impl<T: TransportPort> BACnetServer<T> {
    /// Whether the server owns its link, so its stop seals the layer's
    /// replies and stops it: not on a node's seat.
    pub(super) fn owns_link(&self) -> bool {
        self.seated.is_none()
    }
}

#[cfg(test)]
#[path = "server_seat_tests.rs"]
mod tests;

/// The node's device settings and DeviceCommunicationControl on a seat.
#[cfg(test)]
#[path = "server_seat_node_tests.rs"]
mod node_tests;
