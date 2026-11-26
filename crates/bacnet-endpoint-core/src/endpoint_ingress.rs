use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_network::layer::{NetworkLayer, ReceivedApdu, RoutedTarget};
use bacnet_npdu::NetworkPriority;
use bacnet_transport::port::{DataAttribute, TransportPort};
use bacnet_types::MacAddr;
use bacnet_types::error::{self, Error, Refusal};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::seat::SeatSlot;

/// Number controls queued for the endpoint's Number owner, as many as the
/// layer queues for a control receiver.
const NUMBER_CONTROL_QUEUE: usize = 256;

/// What a send or a seat meets once the endpoint has shut: closed for good,
/// as a started endpoint never starts again (#1833).
pub(crate) fn shutdown_error() -> Error {
    Error::state(error::Lifecycle::Closed, "endpoint shutdown")
}

/// A start of an ingress that has started, or whose network a start took
/// (#1833).
fn started_once(label: &'static str) -> Error {
    Error::state(error::Lifecycle::AlreadyStarted, label)
}

const EFFECTIVE_GROUP_APDU_ERROR: &str =
    "effective group destination requires a valid unconfirmed request APDU";

#[path = "endpoint_destination.rs"]
mod destination;
pub use destination::EndpointApduDestination;

#[path = "endpoint_egress.rs"]
mod egress;
pub use egress::{EndpointEgress, EndpointEgressAdmissionError, EndpointSend, EndpointSendOutcome};
use egress::{NetworkServiceCommand, NetworkServicePayload, send_network_service_apdu};

#[path = "endpoint_routing.rs"]
mod routing;
use crate::coordinator::OutboundTransactionCoordinator;

#[cfg(test)]
tokio::task_local! {
    /// How long a test holds the start just after the classifier spawns.
    pub(crate) static AFTER_CLASSIFIER_SPAWN: std::time::Duration;
}
use routing::{ControlTee, IngressDrops, IngressQueues, next_apdu, route_received};
pub use routing::{DROP_LOG_INTERVAL, IngressDropCounters};
pub(crate) use routing::{copied_when_reassembled, reserve, share_reassembled};

/// What [`EndpointIngress::observe_sources`] calls with each APDU from
/// another network that it shows.
pub type SourceObserver = Arc<dyn Fn(&ReceivedApdu) + Send + Sync>;

/// Destination selected for one decoded APDU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressRoute {
    /// Confirmed and unconfirmed requests.
    InboundRequest,
    /// Acknowledgments, errors, rejects, aborts, and segment acknowledgments.
    TerminalOrSegment,
}

/// Why an APDU could not be delivered to a role queue. A full role queue
/// is no reason: what it has no room for is dropped and counted
/// ([`IngressDropCounters`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyReason {
    /// APDU decoding failed.
    MalformedApdu,
    /// The APDU type nibble is outside the Standard-defined range.
    UnsupportedPduType(u8),
    /// The selected role queue had no receiver.
    RouteClosed(IngressRoute),
    /// A confirmed request from a link-layer address that is a group
    /// destination of the transport
    /// ([`TransportPort::is_group_destination`]). Its answer would go back
    /// there, to every node in the group, so it reaches no role (#1504).
    GroupSource,
}

/// An APDU returned to the endpoint policy owner instead of a role queue.
#[derive(Debug)]
pub struct PolicyOutcome {
    /// Classification or delivery failure.
    pub reason: PolicyReason,
    /// Complete received envelope, including any reply sender.
    pub received: ReceivedApdu,
}

/// Single-consumer queues produced when endpoint ingress starts.
#[non_exhaustive]
pub struct IngressReceivers {
    /// Opted-in single-link local controls, currently NORMAL B/IP, B/IPv6 and SC;
    /// other links retain discard behavior.
    #[doc(hidden)]
    pub network_controls: Option<mpsc::Receiver<bacnet_network::layer::ReceivedNetworkControl>>,
    /// Post-bind registration capability, with the B/IP mode, and
    /// independently supported port capacity.
    #[doc(hidden)]
    pub bip_port: Option<(bacnet_transport::port::BipPort, u16)>,
    /// The same for a B/IPv6 link: its bound address, VMAC, group and mode,
    /// with the port capacity.
    #[doc(hidden)]
    pub bip6_port: Option<(bacnet_transport::port::Bip6Port, u16)>,
    /// Actual announced IPv4 address and bound UDP port, when this is B/IP.
    #[doc(hidden)]
    pub bip_local_address: Option<std::net::SocketAddrV4>,
    /// Actual IPv4 B/IP broadcast endpoint after transport startup.
    #[doc(hidden)]
    pub bip_broadcast_endpoint: Option<std::net::SocketAddrV4>,
    /// The link's own MAC after transport startup, in its own format (#2357).
    #[doc(hidden)]
    pub local_mac: MacAddr,
    /// Confirmed and unconfirmed request traffic.
    pub inbound_requests: mpsc::Receiver<ReceivedApdu>,
    /// Terminal response and segmentation traffic.
    pub terminal_or_segment: mpsc::Receiver<ReceivedApdu>,
    /// Traffic that endpoint policy must handle or reclaim.
    pub policy_outcomes: mpsc::Receiver<PolicyOutcome>,
    /// Bounded network-service egress for endpoint role adapters.
    #[doc(hidden)]
    pub egress: EndpointEgress,
    /// Tells a seated client the Number owner published the number (#2617).
    #[doc(hidden)]
    pub seat_number: crate::seat::SeatNumber,
}

/// Terminal state reported by the classifier task. A full queue is none:
/// traffic never ends classification by its volume (#2635).
#[derive(Debug)]
pub enum ClassifierExit {
    /// Explicit endpoint cancellation won the receive race.
    Cancelled,
    /// The network layer closed its APDU stream.
    InputClosed,
    /// The policy queue was closed: its owner is gone.
    PolicyRouteClosed(Box<PolicyOutcome>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Ready,
    Starting,
    Running,
    Stopping,
    Stopped,
}

/// What the session task ends with: its exit, once the layer it stopped has
/// stopped.
type SessionEnd = Result<ClassifierExit, Error>;

/// Owns one network layer and classifies each received APDU exactly once.
pub struct EndpointIngress<T: TransportPort> {
    network: Option<NetworkLayer<T>>,
    /// The started layer, which the session task owns and a client seat
    /// shares (#2617).
    started: std::sync::Weak<NetworkLayer<T>>,
    seats: Option<Arc<SeatSlot>>,
    /// What a full queue dropped, readable in any state.
    drops: Arc<IngressDrops>,
    /// See [`Self::with_coordinator`].
    coordinator: Option<Arc<OutboundTransactionCoordinator>>,
    /// See [`Self::observe_sources`].
    source_observer: Option<SourceObserver>,
    queue_capacity: usize,
    lifecycle: Lifecycle,
    cancel_tx: Option<oneshot::Sender<()>>,
    session_task: Option<JoinHandle<SessionEnd>>,
    egress_open: Option<Arc<AtomicBool>>,
}

impl<T: TransportPort + 'static> EndpointIngress<T> {
    /// Creates ingress with the same capacity for each bounded output queue.
    pub fn new(transport: T, queue_capacity: usize) -> Self {
        Self {
            network: Some(NetworkLayer::new(transport)),
            started: std::sync::Weak::new(),
            seats: None,
            drops: Arc::default(),
            coordinator: None,
            source_observer: None,
            queue_capacity,
            lifecycle: Lifecycle::Ready,
            cancel_tx: None,
            session_task: None,
            egress_open: None,
        }
    }

    /// The endpoint's pool of invoke IDs (#2635): once the answer queue is
    /// down to its last quarter, an answer none of `coordinator`'s leases
    /// holds is dropped, so it can't take a slot from one that a lease
    /// holds. The look claims nothing. Without a coordinator every answer
    /// is treated alike.
    pub fn with_coordinator(mut self, coordinator: Arc<OutboundTransactionCoordinator>) -> Self {
        self.coordinator = Some(coordinator);
        self
    }

    /// Before start, have `observer` see what comes from another network
    /// (an SNET in its NPDU) before any role takes it (#2617): every
    /// request, and each answer that a lease of [`Self::with_coordinator`]'s
    /// pool holds. A server's seat takes both, so the endpoint's own roles
    /// would see neither. The look at the pool claims nothing.
    #[doc(hidden)]
    pub fn observe_sources(&mut self, observer: SourceObserver) {
        self.source_observer = Some(observer);
    }

    /// Whether the link calls `mac` a group address, as its transport's own
    /// [`TransportPort::is_group_destination`] says, before start as after
    /// (#2617): what a server's start checks its bindings against.
    #[doc(hidden)]
    pub fn is_group_destination(&self, mac: &[u8]) -> bool {
        match (&self.network, self.started.upgrade()) {
            (Some(network), _) => network.transport().is_group_destination(mac),
            (None, Some(network)) => network.transport().is_group_destination(mac),
            (None, None) => false,
        }
    }

    /// Pre-start link capability; never retains or exposes a transport reference.
    #[doc(hidden)]
    pub fn bip_broadcast_endpoint(&self) -> Option<std::net::SocketAddrV4> {
        self.network.as_ref()?.transport().bip_broadcast_endpoint()
    }

    /// Pre-start unicast facts of any link (#2357): the transport's own MAC
    /// as configured, and its group-destination rule. The MAC is empty when
    /// the link learns it only at start: B/IPv6 reports a zero VMAC until
    /// then, which is also Device 0's, so its self check waits for
    /// [`IngressReceivers::local_mac`].
    #[doc(hidden)]
    pub fn link_unicast_facts(
        &self,
    ) -> Option<(MacAddr, bacnet_transport::port::GroupDestinations)> {
        let transport = self.network.as_ref()?.transport();
        let local = if transport.bip6_port().is_some() {
            MacAddr::new()
        } else {
            MacAddr::from_slice(transport.local_mac())
        };
        Some((local, transport.group_destinations()))
    }

    /// Pre-bind B/IP registration capability, with the configured mode.
    #[doc(hidden)]
    pub fn bip_port(&self) -> Option<bacnet_transport::port::BipPort> {
        self.network.as_ref()?.transport().bip_port()
    }

    /// Pre-bind B/IPv6 registration capability, with the configured mode.
    #[doc(hidden)]
    pub fn bip6_port(&self) -> Option<bacnet_transport::port::Bip6Port> {
        self.network.as_ref()?.transport().bip6_port()
    }
    /// Attach selected-object protection before starting the transport.
    #[doc(hidden)]
    pub fn retain_network_port_lease_internal(&mut self, lease: Arc<()>) -> Result<(), Error> {
        if self.lifecycle != Lifecycle::Ready {
            return Err(started_once("ingress already started"));
        }
        self.network
            .as_mut()
            .ok_or_else(|| started_once("missing network"))?
            .retain_network_port_lease_internal(lease)
    }

    /// Starts the transport, network layer, and classifier task once.
    pub async fn start(&mut self) -> Result<IngressReceivers, Error> {
        Ok(self.start_seated(None).await?.0)
    }

    /// [`Self::start`] with a seat for a full server attached before the
    /// classifier takes its first APDU (#2617), so no request reaches the
    /// endpoint's own queue first; `coordinator` as for
    /// [`Self::server_seat`]. Should the seat be refused, the endpoint is
    /// still running when this fails: stop it.
    #[doc(hidden)]
    pub async fn start_with_server_seat(
        &mut self,
        coordinator: Arc<crate::coordinator::OutboundTransactionCoordinator>,
    ) -> Result<(IngressReceivers, crate::seat::ServerSeat<T>), Error> {
        let (receivers, seat) = self.start_seated(Some(coordinator)).await?;
        Ok((receivers, seat.expect("a seat was asked for")?))
    }

    async fn start_seated(
        &mut self,
        server: Option<Arc<crate::coordinator::OutboundTransactionCoordinator>>,
    ) -> Result<
        (
            IngressReceivers,
            Option<Result<crate::seat::ServerSeat<T>, Error>>,
        ),
        Error,
    > {
        if self.lifecycle != Lifecycle::Ready {
            return Err(started_once(
                "endpoint ingress cannot be started more than once",
            ));
        }
        if self.queue_capacity == 0 {
            return Err(Error::invalid_config(
                "endpoint ingress queue capacity must be greater than zero",
            ));
        }

        let network = self
            .network
            .as_mut()
            .ok_or_else(|| started_once("endpoint ingress network owner is missing"))?;
        self.lifecycle = Lifecycle::Starting;
        // Every link's controls are taken, for a client seat (#2617). The
        // endpoint's own Number owner gets them only on a link that takes
        // the local Number controls; on any other they go no further
        // without a seat.
        let layer_controls = network.enable_network_control_receiver()?;
        let apdu_rx = network.start().await?;
        let (numbers, controls) = if network
            .transport()
            .supports_local_nonrouter_number_controls()
        {
            let (tx, rx) = mpsc::channel(NUMBER_CONTROL_QUEUE);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let capacity = network.transport().local_receive_apdu_capacity();
        let bip_port = network.transport().bip_port().map(|port| (port, capacity));
        let bip6_port = network.transport().bip6_port().map(|port| (port, capacity));
        let bip_broadcast_endpoint = network.transport().bip_broadcast_endpoint();
        let local_mac = MacAddr::from_slice(network.local_mac());
        let bip_local_address = bip_broadcast_endpoint.and_then(|_| {
            bacnet_transport::bvll::decode_bip_mac(network.local_mac())
                .ok()
                .map(|(ip, port)| std::net::SocketAddrV4::new(ip.into(), port))
        });
        let (inbound_tx, inbound_requests) = mpsc::channel(self.queue_capacity);
        let (terminal_tx, terminal_or_segment) = mpsc::channel(self.queue_capacity);
        let (policy_tx, policy_outcomes) = mpsc::channel(self.queue_capacity);
        let (seats, seat_holds) = SeatSlot::new();
        let queues = IngressQueues {
            inbound_tx,
            terminal_tx,
            policy_tx,
            seats: Arc::clone(&seats),
            drops: Arc::clone(&self.drops),
            coordinator: self.coordinator.clone(),
            observer: self.source_observer.clone(),
        };
        let controls_tee = ControlTee {
            layer: layer_controls,
            numbers,
            seats: Arc::clone(&seats),
        };
        let (egress_tx, egress_rx) = mpsc::channel(self.queue_capacity);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let egress_open = Arc::new(AtomicBool::new(true));
        // The layer moves into the session task below; its number slot stays
        // shared with the egress, where the endpoint's senders read it.
        let egress = EndpointEgress {
            commands: egress_tx,
            open: Arc::clone(&egress_open),
            local_network: network.local_network_number().clone(),
            group_destinations: network.transport().group_destinations(),
        };
        let network = Arc::new(
            self.network
                .take()
                .ok_or_else(|| started_once("endpoint ingress network owner is missing"))?,
        );
        self.started = Arc::downgrade(&network);
        let seat_number = seats.number().clone();
        let server_seat = server.map(|coordinator| {
            seats.attach_server(Arc::clone(&network), coordinator, self.queue_capacity)
        });
        self.seats = Some(seats);

        self.session_task = Some(tokio::spawn(session_task(
            network,
            (apdu_rx, controls_tee),
            queues,
            egress_rx,
            cancel_rx,
            (Arc::clone(&egress_open), seat_holds),
        )));
        // A test may hold the start here, the classifier already running,
        // to show that the server's seat was attached before it (#2617).
        #[cfg(test)]
        if let Ok(pause) = AFTER_CLASSIFIER_SPAWN.try_with(|pause| *pause) {
            tokio::time::sleep(pause).await;
        }
        self.cancel_tx = Some(cancel_tx);
        self.egress_open = Some(egress_open);
        self.lifecycle = Lifecycle::Running;

        let receivers = IngressReceivers {
            network_controls: controls,
            bip_port,
            bip6_port,
            bip_local_address,
            bip_broadcast_endpoint,
            local_mac,
            inbound_requests,
            terminal_or_segment,
            policy_outcomes,
            egress,
            seat_number,
        };
        Ok((receivers, server_seat))
    }

    /// A seat for a full client on the started link (#2617): the layer and
    /// `coordinator` shared, and queues of `capacity` APDUs for what is the
    /// client's to take (see [`crate::seat`]). One client at a time. Fails
    /// before start and once the endpoint is stopping.
    #[doc(hidden)]
    pub fn client_seat(
        &self,
        coordinator: Arc<crate::coordinator::OutboundTransactionCoordinator>,
        roles: crate::seat::SeatRoles,
        capacity: usize,
    ) -> Result<crate::seat::ClientSeat<T>, Error> {
        let not_running = || self.not_running();
        if self.lifecycle != Lifecycle::Running {
            return Err(not_running());
        }
        let network = self.started.upgrade().ok_or_else(not_running)?;
        self.seats
            .as_ref()
            .ok_or_else(not_running)?
            .attach(network, coordinator, roles, capacity)
    }

    /// A seat for a full server on the started link (#2617): the layer and
    /// `coordinator` shared, and a queue for what is the server's to take.
    /// While it is attached it takes the place of the endpoint's own request
    /// queue (see [`crate::seat`]), so its queue is as deep as that one, the
    /// capacity [`Self::new`] was given. One server at a time; `coordinator`
    /// must be the client seat's when one is attached. Fails before start
    /// and once the endpoint is stopping.
    #[doc(hidden)]
    pub fn server_seat(
        &self,
        coordinator: Arc<crate::coordinator::OutboundTransactionCoordinator>,
    ) -> Result<crate::seat::ServerSeat<T>, Error> {
        let not_running = || self.not_running();
        if self.lifecycle != Lifecycle::Running {
            return Err(not_running());
        }
        let network = self.started.upgrade().ok_or_else(not_running)?;
        self.seats.as_ref().ok_or_else(not_running)?.attach_server(
            network,
            coordinator,
            self.queue_capacity,
        )
    }

    /// What a call that needs the ingress running finds instead, from its
    /// own state (#1833): not started yet, a stop under way, or shut for
    /// good, as a start that failed, a stop that ended or a layer that has
    /// gone leaves it: an ingress starts once.
    fn not_running(&self) -> Error {
        let state = match self.lifecycle {
            Lifecycle::Ready => error::Lifecycle::NotRunning,
            Lifecycle::Stopping => error::Lifecycle::Stopping,
            Lifecycle::Starting | Lifecycle::Running | Lifecycle::Stopped => {
                error::Lifecycle::Closed
            }
        };
        Error::state(state, "endpoint ingress is not running")
    }

    /// What ingress dropped because a queue of its own was full, by class
    /// (#2635). Counts run from [`Self::new`] and stay readable after stop.
    pub fn full_drop_counters(&self) -> IngressDropCounters {
        self.drops.counters()
    }

    /// What ingress has handed to and dropped for the client seat.
    pub fn client_seat_counters(&self) -> crate::seat::ClientSeatCounters {
        self.seats
            .as_ref()
            .map(|seats| seats.counters())
            .unwrap_or_default()
    }

    /// What ingress has handed to and dropped for the server seat.
    pub fn server_seat_counters(&self) -> crate::seat::ServerSeatCounters {
        self.seats
            .as_ref()
            .map(|seats| seats.server_counters())
            .unwrap_or_default()
    }

    /// The node's DeviceCommunicationControl state, which a full server on
    /// its seat sets (#2617). `None` before start.
    #[doc(hidden)]
    pub fn node_dcc(&self) -> Option<crate::seat::NodeDcc> {
        self.seats.as_ref().map(|seats| seats.dcc().clone())
    }

    /// Cancels classification, stops the network layer, and reports classifier exit.
    ///
    /// The seats' queues close first, which ends a seated client's tasks;
    /// then the layer and its transport stop, through the shared reference
    /// (#2617), whether or not the seated roles themselves are still alive.
    /// Their sends after that fail with the transport's stopped error, and
    /// dropping them later changes nothing. A seated server's own stop is
    /// best run first: it joins its work and settles its saves. Dropping
    /// the ingress instead aborts the layer, with the same end for the
    /// seated roles but nothing graceful sent and nothing waited for.
    pub async fn stop(&mut self) -> Result<ClassifierExit, Error> {
        if !matches!(
            self.lifecycle,
            Lifecycle::Starting | Lifecycle::Running | Lifecycle::Stopping
        ) {
            return Err(self.not_running());
        }
        if self.lifecycle == Lifecycle::Starting
            && let Some(network) = self.network.take()
        {
            self.session_task = Some(tokio::spawn(async move {
                network.stop().await?;
                Ok(ClassifierExit::Cancelled)
            }));
        }
        self.lifecycle = Lifecycle::Stopping;
        if let Some(open) = self.egress_open.take() {
            open.store(false, Ordering::Release);
        }
        if let Some(cancel_tx) = self.cancel_tx.take() {
            let _ = cancel_tx.send(());
        }

        if let Some(seats) = &self.seats {
            seats.close();
        }

        let result = match self.session_task.as_mut() {
            Some(task) => match task.await {
                Ok(result) => result,
                // The task panicked or was aborted: whatever it held is
                // gone with it, and the ingress never starts again.
                Err(join) => Err(Error::State(
                    error::Lifecycle::Closed,
                    Refusal::text(format!("endpoint ingress session failed: {join}")),
                )),
            },
            // Only a stop takes the task: this one found it gone.
            None => Err(Error::state(
                error::Lifecycle::Closed,
                "endpoint ingress session task is missing",
            )),
        };
        self.session_task.take();
        self.lifecycle = Lifecycle::Stopped;
        result
    }
}

/// A drop aborts, never waits: the seats' queues close, the session task is
/// aborted, and the started layer is aborted through the shared reference
/// (#2617), so a seated client still alive keeps no link up. It sends
/// nothing more, the transport's sockets go as the runtime drops its aborted
/// tasks, and nothing graceful goes out; [`EndpointIngress::stop`] is the
/// graceful path.
impl<T: TransportPort> Drop for EndpointIngress<T> {
    fn drop(&mut self) {
        if let Some(seats) = &self.seats {
            seats.close();
        }
        if let Some(open) = self.egress_open.take() {
            open.store(false, Ordering::Release);
        }
        if let Some(cancel_tx) = self.cancel_tx.take() {
            let _ = cancel_tx.send(());
        }
        if let Some(task) = self.session_task.take() {
            task.abort();
        }
        if let Some(network) = self.started.upgrade() {
            network.abort();
        }
        // An aborted session task reports no tail of its own.
        let _ = self.drops.flush();
    }
}

async fn session_task<T: TransportPort + 'static>(
    network: Arc<NetworkLayer<T>>,
    (mut apdu_rx, mut controls): (bacnet_network::layer::ApduReceiver, ControlTee),
    queues: IngressQueues,
    mut egress_rx: mpsc::Receiver<NetworkServiceCommand>,
    mut cancel_rx: oneshot::Receiver<()>,
    (egress_open, mut seat_holds): (Arc<AtomicBool>, mpsc::Receiver<()>),
) -> SessionEnd {
    let mut receive_egress = true;
    let mut prefer_ingress = true;
    let exit = loop {
        let event = if prefer_ingress {
            tokio::select! {
                biased;
                _ = &mut cancel_rx => SessionEvent::Cancelled,
                received = next_apdu(&mut apdu_rx, &mut controls) => SessionEvent::Received(received),
                command = egress_rx.recv(), if receive_egress => SessionEvent::Egress(command),
            }
        } else {
            tokio::select! {
                biased;
                _ = &mut cancel_rx => SessionEvent::Cancelled,
                command = egress_rx.recv(), if receive_egress => SessionEvent::Egress(command),
                received = next_apdu(&mut apdu_rx, &mut controls) => SessionEvent::Received(received),
            }
        };

        match event {
            SessionEvent::Cancelled => break ClassifierExit::Cancelled,
            SessionEvent::Received(Some(received)) => {
                prefer_ingress = !prefer_ingress;
                let number = network.local_network_number().get();
                if let Some(exit) = route_received(received, &queues, network.transport(), number) {
                    break exit;
                }
            }
            SessionEvent::Received(None) => break ClassifierExit::InputClosed,
            SessionEvent::Egress(Some(command)) => {
                prefer_ingress = !prefer_ingress;
                match drive_network_service(
                    &network,
                    command,
                    (&mut apdu_rx, &mut controls),
                    &queues,
                    &mut cancel_rx,
                    &mut prefer_ingress,
                )
                .await
                {
                    EgressDrive::Complete => {}
                    EgressDrive::Cancelled => break ClassifierExit::Cancelled,
                    EgressDrive::Exit(exit) => break exit,
                }
            }
            SessionEvent::Egress(None) => receive_egress = false,
        }
    };
    // Drops since the last log line are reported now, not left waiting
    // for a next drop that won't come (#2635).
    let _ = queues.drops.flush();

    egress_open.store(false, Ordering::Release);
    egress_rx.close();
    while let Ok(command) = egress_rx.try_recv() {
        let _ = command.completion.send(EndpointSendOutcome {
            result: Err(shutdown_error()),
            attempted: false,
        });
    }
    // The seat's queues close and its client's tasks end, each letting go
    // of the layer before its hold, so none is still at work when the link
    // stops. A client that is itself still alive keeps its own reference:
    // the layer stops through it all the same (#2617), and that client's
    // later requests fail with the transport's stopped error.
    queues.seats.close();
    while seat_holds.recv().await.is_some() {}
    network.stop().await?;
    Ok(exit)
}

enum SessionEvent {
    Cancelled,
    Received(Option<ReceivedApdu>),
    Egress(Option<NetworkServiceCommand>),
}

enum EgressDrive {
    Complete,
    Cancelled,
    Exit(ClassifierExit),
}

/// A deadline-bound send whose deadline passed (#1833): before the send
/// began (`started` false: nothing went), or while the transport carried
/// it (it may have gone). This side's link didn't carry it in time, so an
/// I/O timeout of the link, not [`Error::Timeout`], which says a peer gave
/// no answer.
#[cold]
#[inline(never)]
fn send_deadline_expired(started: bool) -> Error {
    let text = if started {
        "endpoint send deadline expired"
    } else {
        "endpoint send expired before execution"
    };
    Error::Transport(std::io::Error::new(std::io::ErrorKind::TimedOut, text))
}

// A select! result consumed on the spot, never stored: boxing the APDU would
// add a heap allocation per APDU received while a send is in flight.
#[allow(clippy::large_enum_variant)]
enum PendingEvent {
    Cancelled,
    Received(Option<ReceivedApdu>),
    Sent(Result<(), Error>),
}

async fn drive_network_service<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    command: NetworkServiceCommand,
    (apdu_rx, controls): (&mut bacnet_network::layer::ApduReceiver, &mut ControlTee),
    queues: &IngressQueues,
    cancel_rx: &mut oneshot::Receiver<()>,
    prefer_ingress: &mut bool,
) -> EgressDrive {
    let NetworkServiceCommand {
        payload,
        destination,
        expecting_reply,
        priority,
        data_attributes,
        mut completion,
        deadline,
        cancel_on_drop,
    } = command;
    if (cancel_on_drop && completion.is_closed())
        || deadline.is_some_and(|deadline| deadline <= tokio::time::Instant::now())
    {
        let _ = completion.send(EndpointSendOutcome {
            result: Err(send_deadline_expired(false)),
            attempted: false,
        });
        return EgressDrive::Complete;
    }
    let expiry = async {
        match deadline {
            Some(deadline) => tokio::time::sleep_until(deadline).await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(expiry);
    let outcome = {
        let send = async {
            match &payload {
                NetworkServicePayload::Apdu(apdu) => {
                    send_network_service_apdu(
                        network,
                        apdu,
                        &destination,
                        expecting_reply,
                        priority,
                        &data_attributes,
                    )
                    .await
                }
                NetworkServicePayload::LocalControl(npdu) => {
                    network.transport().send_broadcast(npdu).await
                }
            }
        };
        tokio::pin!(send);
        loop {
            let event = if *prefer_ingress {
                tokio::select! {
                    biased;
                    _ = &mut *cancel_rx => PendingEvent::Cancelled,
                    _ = &mut expiry => {
                        let _ = completion.send(EndpointSendOutcome {
                            result: Err(send_deadline_expired(true)), attempted: true,
                        });
                        return EgressDrive::Complete;
                    },
                    _ = completion.closed(), if cancel_on_drop => return EgressDrive::Complete,
                    received = next_apdu(apdu_rx, controls) => PendingEvent::Received(received),
                    result = &mut send => PendingEvent::Sent(result),
                }
            } else {
                tokio::select! {
                    biased;
                    _ = &mut *cancel_rx => PendingEvent::Cancelled,
                    _ = &mut expiry => {
                        let _ = completion.send(EndpointSendOutcome {
                            result: Err(send_deadline_expired(true)), attempted: true,
                        });
                        return EgressDrive::Complete;
                    },
                    _ = completion.closed(), if cancel_on_drop => return EgressDrive::Complete,
                    result = &mut send => PendingEvent::Sent(result),
                    received = next_apdu(apdu_rx, controls) => PendingEvent::Received(received),
                }
            };

            match event {
                PendingEvent::Cancelled => break EgressDrive::Cancelled,
                PendingEvent::Received(Some(received)) => {
                    *prefer_ingress = !*prefer_ingress;
                    let number = network.local_network_number().get();
                    if let Some(exit) =
                        route_received(received, queues, network.transport(), number)
                    {
                        break EgressDrive::Exit(exit);
                    }
                }
                PendingEvent::Received(None) => {
                    break EgressDrive::Exit(ClassifierExit::InputClosed);
                }
                PendingEvent::Sent(result) => {
                    *prefer_ingress = !*prefer_ingress;
                    let _ = completion.send(EndpointSendOutcome {
                        result,
                        attempted: true,
                    });
                    return EgressDrive::Complete;
                }
            }
        }
    };

    let _ = completion.send(EndpointSendOutcome {
        result: Err(shutdown_error()),
        attempted: true,
    });
    outcome
}

/// Refuse anything but a valid Unconfirmed-Request APDU to a group: the
/// three broadcast destinations, and a direct one to a MAC that reaches a
/// group of nodes ([`TransportPort::is_group_destination`]), which with no
/// DNET is a local broadcast (Clause 6.3, #1479). The network layer refuses
/// the routed forms with no DADR itself.
fn validate_effective_group_apdu(
    apdu: &[u8],
    destination: &EndpointApduDestination,
    is_group_destination: impl FnOnce(&[u8]) -> bool,
) -> Result<(), Error> {
    let group = match destination {
        EndpointApduDestination::LocalBroadcast
        | EndpointApduDestination::RemoteBroadcast { .. }
        | EndpointApduDestination::GlobalBroadcast => true,
        EndpointApduDestination::Direct { destination_mac } => {
            is_group_destination(destination_mac)
        }
        EndpointApduDestination::Routed { .. }
        | EndpointApduDestination::RoutedViaLocalBroadcast { .. } => false,
    };
    if !group {
        return Ok(());
    }

    match decode_apdu(apdu.to_vec().into()) {
        Ok(Apdu::UnconfirmedRequest(_)) => Ok(()),
        _ => Err(Error::invalid_argument(EFFECTIVE_GROUP_APDU_ERROR)),
    }
}

#[cfg(test)]
#[path = "endpoint_ingress_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "endpoint_network_service_tests.rs"]
mod network_service_tests;

#[cfg(test)]
#[path = "endpoint_egress_deadline_tests.rs"]
mod deadline_tests;
