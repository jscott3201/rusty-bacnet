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
//! What a seat can't take yet: a registered Network Port, whose reservation
//! needs the transport before it starts. A seated build refuses one before
//! it changes anything.

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

/// A seat's queue, which the node's ingress fills.
impl Apdus for mpsc::Receiver<ReceivedApdu> {
    #[inline(always)]
    fn recv(&mut self) -> impl Future<Output = Option<ReceivedApdu>> + Send + '_ {
        mpsc::Receiver::recv(self)
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
    /// with a client role the node's (#2617).
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
            execution: DeviceExecution::FullServer,
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

/// A node's seat.
pub(super) struct Seat<T: TransportPort>(ServerSeat<T>);

impl<T: TransportPort + 'static> Link<T> for Seat<T> {
    type Apdus = mpsc::Receiver<ReceivedApdu>;

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
            apdus,
            audit_routes: Arc::new(audit_routes),
            controls: None,
            notification_transactions: NotificationTransactions::with_coordinator(coordinator),
            seated: Some(Seated::new(&holds)),
            execution: match node {
                Some(node) if node.client => DeviceExecution::Node,
                _ => DeviceExecution::FullServer,
            },
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
        }
    }
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
    /// `notification_retries` the builder set that differ from the node's,
    /// on a database whose Device declares another APDU_Timeout or
    /// Number_Of_APDU_Retries than the node's (one that declares the object
    /// model's defaults, 6000 ms and 3, is given the node's), and on
    /// whatever a transport
    /// of its own would refuse. Must be called within a Tokio runtime.
    pub async fn build(mut self) -> Result<BACnetServer<T>, Error> {
        if self.had_transport {
            return Err(Error::Encoding(
                "a server builder with a transport of its own takes no seat as well".into(),
            ));
        }
        if self.config.registered_network_port.is_some() {
            return Err(Error::Encoding(
                "a server on a node's seat takes no registered Network Port: the node owns \
                 the link"
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
            Seat(self.seat),
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

    /// Take the node's device settings in place of the builder's, refusing
    /// any the builder set that differ from them (#2617). Whether the
    /// Device is to be given the node's timing: a stock one is, and a
    /// supplied one that declares the object model's defaults, which no
    /// identity-built database can set otherwise.
    fn take_node(&mut self, node: NodeDevice) -> Result<bool, Error> {
        let identity = self.node_identity.take().ok_or_else(|| {
            Error::Encoding("a server on a node's seat needs the node's Device identity".into())
        })?;
        let defaults = ServerConfig::default();
        // Compared as the link serves them: the node resolved its identity
        // against the link at start, so on a link that takes less than the
        // identity asks (480 octets on MS/TP) the two differ only there. The
        // identity's service list is left out: the server serves the node's
        // executed services whatever it says.
        let capacity = self.seat.network.transport().local_receive_apdu_capacity();
        let served = |identity: &bacnet_objects::DeviceIdentity| {
            identity
                .resolve(capacity)
                .map(|resolved| resolved.with_services(&[]))
        };
        if self.config.device == defaults.device {
            tracing::debug!("a seated server serves the node's Device identity");
        } else if served(&self.config.device)? != served(&identity)? {
            return Err(contradicts("Device identity"));
        }
        let timing = [
            (
                "cov_retry_timeout_ms",
                self.config.cov_retry_timeout_ms,
                defaults.cov_retry_timeout_ms,
                node.apdu_timeout_ms,
            ),
            (
                "notification_retries",
                u64::from(self.config.notification_retries),
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
        if let Some(db) = &self.db {
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
        // The node accepts the smaller of its roles' largest APDUs
        // (Clause 12.11.18); the link's capacity bounds it further at start.
        self.config.device = identity.resolve(node.client_max_apdu_length)?;
        self.config.cov_retry_timeout_ms = node.apdu_timeout_ms;
        self.config.notification_retries = node.apdu_retries;
        Ok(state_timing)
    }
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
