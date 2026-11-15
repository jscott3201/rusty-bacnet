//! Where the dispatch loop's APDUs and network controls come from: the
//! client's own network layer, or a seat on a node's (#2617).
//!
//! A standalone client makes and starts a network layer, and its dispatch
//! task reads that layer's receivers ([`Own`]). A client on a
//! [`ClientSeat`] shares a node's started layer and its invoke-ID
//! coordinator, and reads the queues the node fills with what is the
//! client's to take ([`Seated`]). The dispatch loop is generic over the
//! two, so the standalone client's loop is the code it was, and nothing a
//! seat needs sits in a struct a request touches.

use std::task::{Context, Poll};

use bacnet_endpoint_core::seat::{ClientSeat, SeatHold, SeatHolds, SeatNumber};
use bacnet_network::layer::{ApduReceiver, ReceivedApdu, ReceivedNetworkControl};
use bacnet_network::network_number::NumberControl;

use super::dispatch_turn::{Turn, TurnOrder};
use super::lifecycle::Controls;
use super::network_number::{self, Owner};
use super::*;

/// One turn of the dispatch loop.
pub(super) type InboundTurn<C> = Turn<ReceivedApdu, ReceivedNetworkControl, C>;

/// The source of a client's inbound work, and what differs with it.
pub(super) trait Inbound<T: TransportPort + 'static>: Send + 'static {
    /// Whether the client runs on a seat, so a node owns the layer.
    const SEATED: bool;

    /// The started network layer the client sends on.
    fn network(&self) -> &Arc<NetworkLayer<T>>;

    /// Take the next piece of work, as [`TurnOrder::poll_turn`] orders it.
    fn poll_turn<C>(
        &mut self,
        order: &mut TurnOrder,
        cx: &mut Context<'_>,
        cleanups: impl FnOnce(&mut Context<'_>) -> Poll<Option<C>>,
    ) -> Poll<InboundTurn<C>>;

    /// Whether the client answers the confirmed requests it is sent: always,
    /// except on a seat beside a server role.
    fn answers_for_device(&self) -> bool;

    /// Start the task that takes the Number controls.
    fn spawn_number_owner(&self, owner: Owner) -> (mpsc::Sender<NumberControl>, JoinHandle<()>);

    /// Take the client's transaction state and discovery state, which a
    /// seat closes when its queues end. Called once, before the dispatch
    /// task starts.
    fn bind(&mut self, tsm: &Arc<SyncMutex<Tsm>>, discovery: &Arc<network_number::Discovery>);

    /// The seat's line back to its node, when the client runs on one.
    fn seat(&self) -> Option<SeatHolds>;
}

/// A standalone client's own layer and its receivers.
pub(super) struct Own<T: TransportPort> {
    network: Arc<NetworkLayer<T>>,
    apdu_rx: ApduReceiver,
    controls: Controls,
}

impl<T: TransportPort + 'static> Own<T> {
    pub(super) fn new(
        network: Arc<NetworkLayer<T>>,
        apdu_rx: ApduReceiver,
        controls: mpsc::Receiver<ReceivedNetworkControl>,
    ) -> Self {
        let controls = Controls::new(controls, network.network_control_ingress_sequence());
        Self {
            network,
            apdu_rx,
            controls,
        }
    }
}

impl<T: TransportPort + 'static> Inbound<T> for Own<T> {
    const SEATED: bool = false;

    #[inline(always)]
    fn network(&self) -> &Arc<NetworkLayer<T>> {
        &self.network
    }

    #[inline(always)]
    fn poll_turn<C>(
        &mut self,
        order: &mut TurnOrder,
        cx: &mut Context<'_>,
        cleanups: impl FnOnce(&mut Context<'_>) -> Poll<Option<C>>,
    ) -> Poll<InboundTurn<C>> {
        let Self {
            network,
            apdu_rx,
            controls,
        } = self;
        order.poll_turn(
            cx,
            || controls.take(network.network_control_ingress_sequence()),
            |cx| apdu_rx.poll_recv(cx),
            cleanups,
        )
    }

    #[inline(always)]
    fn answers_for_device(&self) -> bool {
        true
    }

    #[inline(always)]
    fn spawn_number_owner(&self, owner: Owner) -> (mpsc::Sender<NumberControl>, JoinHandle<()>) {
        network_number::spawn(&self.network, owner)
    }

    #[inline(always)]
    fn bind(&mut self, _: &Arc<SyncMutex<Tsm>>, _: &Arc<network_number::Discovery>) {}

    #[inline(always)]
    fn seat(&self) -> Option<SeatHolds> {
        None
    }
}

/// A node's layer with the seat's hold on it. The layer is the first field,
/// so it is let go before the hold, and the node sees the hold drop only
/// once this reference is gone.
struct HeldNetwork<T: TransportPort> {
    network: Arc<NetworkLayer<T>>,
    _hold: SeatHold,
}

/// A seat's queues.
pub(super) struct Seated<T: TransportPort> {
    held: HeldNetwork<T>,
    apdus: mpsc::Receiver<ReceivedApdu>,
    controls: mpsc::Receiver<ReceivedNetworkControl>,
    /// A server role answers for the device.
    server: bool,
    number: SeatNumber,
    holds: SeatHolds,
    /// What closes with the seat: see the `Drop`.
    closing: Option<(Arc<SyncMutex<Tsm>>, Arc<network_number::Discovery>)>,
}

/// The seat's queues are gone, with the dispatch task that read them: the
/// node stopped or dropped the seat, or aborted the task, or the client was
/// stopped. Nothing can answer a request any more, so the open ones fail
/// now, not at their timeouts, and no new one is sent: a confirmed request
/// finds no coordinator, and the sends that keep no transaction find the
/// seal. The client's `transport()` is left as it is.
impl<T: TransportPort> Drop for Seated<T> {
    fn drop(&mut self) {
        if let Some((tsm, discovery)) = self.closing.take() {
            discovery.asking.seal();
            // A blocking lock, where the crate's other drops only try: a
            // close skipped here would leave the client's leases held and
            // its coordinator in place. It can't deadlock: an aborted task
            // is dropped by the runtime, never inside a caller that holds
            // the guard, and no TSM guard is held across an await.
            tsm.lock().close_coordinated();
        }
    }
}

impl<T: TransportPort + 'static> Inbound<T> for Seated<T> {
    const SEATED: bool = true;

    fn network(&self) -> &Arc<NetworkLayer<T>> {
        &self.held.network
    }

    fn poll_turn<C>(
        &mut self,
        order: &mut TurnOrder,
        cx: &mut Context<'_>,
        cleanups: impl FnOnce(&mut Context<'_>) -> Poll<Option<C>>,
    ) -> Poll<InboundTurn<C>> {
        let Self {
            apdus, controls, ..
        } = self;
        let turn = order.poll_turn(
            cx,
            || controls.try_recv().ok(),
            |cx| apdus.poll_recv(cx),
            cleanups,
        );
        if turn.is_ready() {
            return turn;
        }
        // The node's ingress task fills the control queue, not this task's
        // APDU receive, so the queue needs this task's waker too.
        match controls.poll_recv(cx) {
            Poll::Ready(Some(control)) => Poll::Ready(Turn::Control(control)),
            Poll::Ready(None) | Poll::Pending => Poll::Pending,
        }
    }

    fn answers_for_device(&self) -> bool {
        !self.server
    }

    fn spawn_number_owner(&self, owner: Owner) -> (mpsc::Sender<NumberControl>, JoinHandle<()>) {
        let held = HeldNetwork {
            network: Arc::clone(&self.held.network),
            _hold: self.held._hold.clone(),
        };
        spawn_seated_number(held, owner, self.number.clone())
    }

    fn bind(&mut self, tsm: &Arc<SyncMutex<Tsm>>, discovery: &Arc<network_number::Discovery>) {
        self.closing = Some((Arc::clone(tsm), Arc::clone(discovery)));
    }

    fn seat(&self) -> Option<SeatHolds> {
        Some(self.holds.clone())
    }
}

impl<T: TransportPort> BACnetClient<T> {
    /// Refuse a send that keeps no transaction once the client's seat has
    /// closed. A client with a transport of its own has no seat to close.
    pub(super) fn seat_open(&self) -> Result<(), Error> {
        if self.device_table.asking.is_sealed() {
            return Err(Error::Encoding(
                "the client's seat has closed: its node stopped".into(),
            ));
        }
        Ok(())
    }
}

/// The Number work of a seated client. The node has one Number owner, which
/// answers What-Is-Network-Number and publishes the number to the shared
/// layer; this task neither answers nor publishes. It asks, as a standalone
/// client does (on first need, and again when the link returns), which the
/// node's owner never does, and it folds the discovery table when the
/// node's owner has published another number.
fn spawn_seated_number<T: TransportPort + 'static>(
    held: HeldNetwork<T>,
    owner: Owner,
    published: SeatNumber,
) -> (mpsc::Sender<NumberControl>, JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel(network_number::CAPACITY);
    let task = tokio::spawn(async move {
        let Owner {
            discovery,
            device_tx,
            collision_tx,
            mut link_returns,
        } = owner;
        let network = &held.network;
        let asking = &discovery.asking;
        // The number the table was last folded to. One published before
        // this client started needs no fold: its table was empty then.
        let mut folded = network.local_network_number().get();
        loop {
            // No branch holds a lock, and each arm finishes its work, the
            // send included, before the next turn polls any of them.
            tokio::select! {
                control = rx.recv() => match control {
                    // The node's owner takes the same control; a number it
                    // changes is folded below, or when it says so.
                    Some(_) => {}
                    None => break,
                },
                () = published.changed() => {}
                () = asking.wanted() => {
                    asking.owner_need(network).await;
                    continue;
                }
                () = network_number::link_returned(&mut link_returns) => {
                    asking.again(network).await;
                    continue;
                }
            }
            let now = network.local_network_number().get();
            if let Some(number) = now.filter(|_| now != folded) {
                device_events::fold_to_number(
                    &mut discovery.table.lock(),
                    folded.is_some(),
                    number,
                    (&device_tx, &collision_tx),
                );
                folded = now;
            }
        }
    });
    (tx, task)
}

/// A [`ClientBuilder`] with a seat in place of a transport, from
/// [`ClientBuilder::seat`].
pub struct SeatedClientBuilder<T: TransportPort> {
    config: ClientConfig,
    options: ClientOptions,
    seat: ClientSeat<T>,
}

impl<T: TransportPort + 'static> ClientBuilder<T> {
    /// Run the client on `seat`, a node's started link, in place of a
    /// transport of its own (#2617). Set the builder's other options first.
    /// See [`BACnetClient::start_on_seat`].
    pub fn seat(self, seat: ClientSeat<T>) -> SeatedClientBuilder<T> {
        SeatedClientBuilder {
            config: self.config,
            options: self.options,
            seat,
        }
    }
}

impl<T: TransportPort + 'static> SeatedClientBuilder<T> {
    /// Build and start the client on its seat.
    pub async fn build(self) -> Result<BACnetClient<T>, Error> {
        BACnetClient::start_on_seat(self.config, self.seat, self.options)
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Start the client on a seat: a node's started network layer and its
    /// invoke-ID coordinator, shared with the node's other roles (#2617).
    ///
    /// The client is the one a transport of its own gives, with the same
    /// calls: it sends on the shared layer, so [`Self::transport`] is the
    /// node's real link, and its invoke IDs come from the node's one pool.
    /// What differs:
    ///
    /// - it receives what the node hands it: the answers to its own
    ///   requests, every Unconfirmed-Request, each ConfirmedCOVNotification,
    ///   and the network controls;
    /// - beside a server role it answers only a ConfirmedCOVNotification.
    ///   It rejects no other Confirmed-Request, and a
    ///   ConfirmedEventNotification is published to
    ///   [`Self::event_notifications`] without an acknowledgment, which is
    ///   the server role's to send. An endpoint's narrow server role doesn't
    ///   execute that service and answers it with a Reject (unrecognized
    ///   service), until a full server takes a seat (#2617 step 4). With no
    ///   server role the client answers as a standalone client does;
    /// - the node has the one owner of the local network number. This
    ///   client still asks What-Is-Network-Number on first need, but it
    ///   never answers one, and it takes the number the node's owner
    ///   publishes;
    /// - [`Self::stop`] stops the client alone. The node stops the link,
    ///   which it can do once this client is dropped. When the node stops
    ///   first, this client's tasks end, its open requests fail, and it
    ///   sends nothing more: every later request and unconfirmed send
    ///   fails at once. [`Self::transport`] is the node's link for as long
    ///   as the client lives, and a call made on it directly, such as a
    ///   B/IP BVLC query, still goes out.
    ///
    /// Must be called within a Tokio runtime. Fails on settings
    /// [`Self::start_with_options`] refuses, and once the node is stopping.
    pub fn start_on_seat(
        config: ClientConfig,
        seat: ClientSeat<T>,
        options: ClientOptions,
    ) -> Result<Self, Error> {
        config.validate_request_settings()?;
        let _bucket = cap_max_apdu_to_transport(
            config.max_apdu_length,
            seat.network.transport().egress_apdu_limit(),
        )?;
        config.validate_window()?;
        options.validate()?;
        let ClientSeat {
            network,
            coordinator,
            apdus,
            controls,
            roles,
            number,
            holds,
            ..
        } = seat;
        let hold = holds
            .hold()
            .ok_or_else(|| Error::Encoding("the seat's node is stopping".into()))?;
        let inbound = Seated {
            held: HeldNetwork {
                network,
                _hold: hold,
            },
            apdus,
            controls,
            server: roles.server,
            number,
            holds,
            closing: None,
        };
        Ok(Self::assemble(config, options, inbound, coordinator))
    }
}
