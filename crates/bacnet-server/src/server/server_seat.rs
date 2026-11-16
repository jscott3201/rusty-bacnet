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
//! What a seat can't take yet: a registered Network Port, whose reservation
//! needs the transport before it starts, and Audit Reporters, whose routes
//! follow the number the server's own Number owner publishes. A seated
//! build refuses either before it changes anything.

use std::any::Any;
use std::future::Future;

use bacnet_endpoint_core::seat::{SeatHolds, ServerSeat};
use bacnet_network::layer::{ApduReceiver, ReceivedApdu, ReceivedNetworkControl};

use super::audit_recipient_routes::AuditRoutes;
use super::*;

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
        _: &mut ObjectDatabase,
        _: &ServerConfig,
        routes: AuditRoutes,
    ) -> Result<Started<T, Self::Apdus>, Error> {
        let ServerSeat {
            network,
            coordinator,
            apdus,
            holds,
            ..
        } = self.0;
        Ok(Started {
            network,
            apdus,
            // With no Audit Reporters, which a seated build refuses, the
            // routes are what `prepare` made: nothing to finish on the link.
            audit_routes: Arc::new(routes),
            controls: None,
            notification_transactions: NotificationTransactions::with_coordinator(coordinator),
            seated: Some(Seated::new(&holds)),
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
        }
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
    ///   transport's error.
    ///
    /// Fails, before it changes anything, on a builder that had a transport
    /// of its own set, on one with a registered Network Port or Audit
    /// Reporters, which a seat can't take yet, and on whatever a transport
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
        if self.config.audit_reporters.is_some() {
            return Err(Error::Encoding(
                "a server on a node's seat takes no Audit Reporters yet".into(),
            ));
        }
        let db = crate::local_device::prepare_database(
            &mut self.config,
            self.db,
            self.seat.network.transport().local_receive_apdu_capacity(),
        )?;
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
