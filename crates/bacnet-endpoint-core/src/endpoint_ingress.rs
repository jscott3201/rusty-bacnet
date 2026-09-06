use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_network::layer::{NetworkLayer, ReceivedApdu};
use bacnet_transport::port::{DataAttribute, TransportPort};
use bacnet_types::enums::NetworkPriority;
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

fn shutdown_error() -> Error {
    Error::Encoding("endpoint shutdown".into())
}

const EFFECTIVE_GROUP_APDU_ERROR: &str =
    "effective group destination requires a valid unconfirmed request APDU";

/// Network-layer destination for one hidden endpoint APDU send.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointApduDestination {
    /// Direct unicast on the local data link.
    Direct {
        /// Destination MAC on the local data link.
        destination_mac: MacAddr,
    },
    /// Routed unicast through a known next-hop router.
    Routed {
        /// Ultimate BACnet network number.
        destination_network: u16,
        /// Ultimate BACnet MAC address.
        destination_mac: MacAddr,
        /// Immediate router MAC on the local data link.
        router_mac: MacAddr,
    },
    /// Routed unicast using a local broadcast because the router MAC is unknown.
    RoutedViaLocalBroadcast {
        /// Ultimate BACnet network number.
        destination_network: u16,
        /// Ultimate BACnet MAC address.
        destination_mac: MacAddr,
    },
    /// Broadcast on only the local BACnet network.
    LocalBroadcast,
    /// Broadcast to one remote BACnet network.
    RemoteBroadcast {
        /// Ultimate BACnet network number.
        destination_network: u16,
    },
    /// Broadcast to all reachable BACnet networks.
    GlobalBroadcast,
}

struct NetworkServiceCommand {
    apdu: Vec<u8>,
    destination: EndpointApduDestination,
    expecting_reply: bool,
    priority: NetworkPriority,
    data_attributes: Vec<DataAttribute>,
    completion: oneshot::Sender<Result<(), Error>>,
}

/// Bounded APDU network-service sender for roles attached to an endpoint session.
#[doc(hidden)]
#[derive(Clone)]
pub struct EndpointEgress {
    commands: mpsc::Sender<NetworkServiceCommand>,
    open: Arc<AtomicBool>,
}

impl EndpointEgress {
    /// Sends one APDU without granting network lifecycle access.
    #[doc(hidden)]
    pub async fn send_apdu(
        &self,
        apdu: Vec<u8>,
        destination: EndpointApduDestination,
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: Vec<DataAttribute>,
    ) -> Result<(), Error> {
        if !self.open.load(Ordering::Acquire) {
            return Err(shutdown_error());
        }

        let (completion, result) = oneshot::channel();
        let command = NetworkServiceCommand {
            apdu,
            destination,
            expecting_reply,
            priority,
            data_attributes,
            completion,
        };
        match self.commands.try_send(command) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Closed(_)) => return Err(shutdown_error()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                if !self.open.load(Ordering::Acquire) {
                    return Err(shutdown_error());
                }
                return Err(Error::Encoding("endpoint egress queue is full".into()));
            }
        }

        result.await.unwrap_or_else(|_| Err(shutdown_error()))
    }

    /// Sends one direct unicast APDU without granting network lifecycle access.
    #[doc(hidden)]
    pub async fn send_direct(
        &self,
        apdu: Vec<u8>,
        destination_mac: MacAddr,
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        self.send_apdu(
            apdu,
            EndpointApduDestination::Direct { destination_mac },
            expecting_reply,
            priority,
            Vec::new(),
        )
        .await
    }
}

/// Destination selected for one decoded APDU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressRoute {
    /// Confirmed and unconfirmed requests.
    InboundRequest,
    /// Acknowledgments, errors, rejects, aborts, and segment acknowledgments.
    TerminalOrSegment,
}

/// Why an APDU could not be delivered to a role queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyReason {
    /// APDU decoding failed.
    MalformedApdu,
    /// The APDU type nibble is outside the Standard-defined range.
    UnsupportedPduType(u8),
    /// The selected bounded role queue was full.
    RouteFull(IngressRoute),
    /// The selected role queue had no receiver.
    RouteClosed(IngressRoute),
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
pub struct IngressReceivers {
    /// Confirmed and unconfirmed request traffic.
    pub inbound_requests: mpsc::Receiver<ReceivedApdu>,
    /// Terminal response and segmentation traffic.
    pub terminal_or_segment: mpsc::Receiver<ReceivedApdu>,
    /// Traffic that endpoint policy must handle or reclaim.
    pub policy_outcomes: mpsc::Receiver<PolicyOutcome>,
    /// Bounded network-service egress for endpoint role adapters.
    #[doc(hidden)]
    pub egress: EndpointEgress,
}

/// Terminal state reported by the classifier task.
#[derive(Debug)]
pub enum ClassifierExit {
    /// Explicit endpoint cancellation won the receive race.
    Cancelled,
    /// The network layer closed its APDU stream.
    InputClosed,
    /// A full policy queue prevented lossless reclamation.
    PolicyRouteFull(PolicyOutcome),
    /// The policy queue was closed.
    PolicyRouteClosed(PolicyOutcome),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Ready,
    Running,
    Stopped,
}

/// Owns one network layer and classifies each received APDU exactly once.
pub struct EndpointIngress<T: TransportPort> {
    network: Option<NetworkLayer<T>>,
    queue_capacity: usize,
    lifecycle: Lifecycle,
    cancel_tx: Option<oneshot::Sender<()>>,
    session_task: Option<JoinHandle<Result<ClassifierExit, Error>>>,
    egress_open: Option<Arc<AtomicBool>>,
}

impl<T: TransportPort + 'static> EndpointIngress<T> {
    /// Creates ingress with the same capacity for each bounded output queue.
    pub fn new(transport: T, queue_capacity: usize) -> Self {
        Self {
            network: Some(NetworkLayer::new(transport)),
            queue_capacity,
            lifecycle: Lifecycle::Ready,
            cancel_tx: None,
            session_task: None,
            egress_open: None,
        }
    }

    /// Starts the transport, network layer, and classifier task once.
    pub async fn start(&mut self) -> Result<IngressReceivers, Error> {
        if self.lifecycle != Lifecycle::Ready {
            return Err(Error::Encoding(
                "endpoint ingress cannot be started more than once".into(),
            ));
        }
        if self.queue_capacity == 0 {
            return Err(Error::Encoding(
                "endpoint ingress queue capacity must be greater than zero".into(),
            ));
        }

        let network = self
            .network
            .as_mut()
            .ok_or_else(|| Error::Encoding("endpoint ingress network owner is missing".into()))?;
        let apdu_rx = network.start().await?;
        let (inbound_tx, inbound_requests) = mpsc::channel(self.queue_capacity);
        let (terminal_tx, terminal_or_segment) = mpsc::channel(self.queue_capacity);
        let (policy_tx, policy_outcomes) = mpsc::channel(self.queue_capacity);
        let (egress_tx, egress_rx) = mpsc::channel(self.queue_capacity);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let egress_open = Arc::new(AtomicBool::new(true));
        let egress = EndpointEgress {
            commands: egress_tx,
            open: Arc::clone(&egress_open),
        };
        let network = self
            .network
            .take()
            .ok_or_else(|| Error::Encoding("endpoint ingress network owner is missing".into()))?;

        self.session_task = Some(tokio::spawn(session_task(
            network,
            apdu_rx,
            inbound_tx,
            terminal_tx,
            policy_tx,
            egress_rx,
            cancel_rx,
            Arc::clone(&egress_open),
        )));
        self.cancel_tx = Some(cancel_tx);
        self.egress_open = Some(egress_open);
        self.lifecycle = Lifecycle::Running;

        Ok(IngressReceivers {
            inbound_requests,
            terminal_or_segment,
            policy_outcomes,
            egress,
        })
    }

    /// Cancels classification, stops the network layer, and reports classifier exit.
    pub async fn stop(&mut self) -> Result<ClassifierExit, Error> {
        if self.lifecycle != Lifecycle::Running {
            return Err(Error::Encoding("endpoint ingress is not running".into()));
        }

        self.lifecycle = Lifecycle::Stopped;
        if let Some(open) = self.egress_open.take() {
            open.store(false, Ordering::Release);
        }
        if let Some(cancel_tx) = self.cancel_tx.take() {
            let _ = cancel_tx.send(());
        }

        match self.session_task.take() {
            Some(task) => task.await.map_err(|error| {
                Error::Encoding(format!("endpoint ingress session failed: {error}"))
            })?,
            None => Err(Error::Encoding(
                "endpoint ingress session task is missing".into(),
            )),
        }
    }
}

impl<T: TransportPort> Drop for EndpointIngress<T> {
    fn drop(&mut self) {
        if let Some(open) = self.egress_open.take() {
            open.store(false, Ordering::Release);
        }
        if let Some(cancel_tx) = self.cancel_tx.take() {
            let _ = cancel_tx.send(());
        }
        if let Some(task) = self.session_task.take() {
            task.abort();
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn session_task<T: TransportPort + 'static>(
    mut network: NetworkLayer<T>,
    mut apdu_rx: mpsc::Receiver<ReceivedApdu>,
    inbound_tx: mpsc::Sender<ReceivedApdu>,
    terminal_tx: mpsc::Sender<ReceivedApdu>,
    policy_tx: mpsc::Sender<PolicyOutcome>,
    mut egress_rx: mpsc::Receiver<NetworkServiceCommand>,
    mut cancel_rx: oneshot::Receiver<()>,
    egress_open: Arc<AtomicBool>,
) -> Result<ClassifierExit, Error> {
    let mut receive_egress = true;
    let mut prefer_ingress = true;
    let exit = loop {
        let event = if prefer_ingress {
            tokio::select! {
                biased;
                _ = &mut cancel_rx => SessionEvent::Cancelled,
                received = apdu_rx.recv() => SessionEvent::Received(received),
                command = egress_rx.recv(), if receive_egress => SessionEvent::Egress(command),
            }
        } else {
            tokio::select! {
                biased;
                _ = &mut cancel_rx => SessionEvent::Cancelled,
                command = egress_rx.recv(), if receive_egress => SessionEvent::Egress(command),
                received = apdu_rx.recv() => SessionEvent::Received(received),
            }
        };

        match event {
            SessionEvent::Cancelled => break ClassifierExit::Cancelled,
            SessionEvent::Received(Some(received)) => {
                prefer_ingress = !prefer_ingress;
                if let Some(exit) = route_received(received, &inbound_tx, &terminal_tx, &policy_tx)
                {
                    break exit;
                }
            }
            SessionEvent::Received(None) => break ClassifierExit::InputClosed,
            SessionEvent::Egress(Some(command)) => {
                prefer_ingress = !prefer_ingress;
                match drive_network_service(
                    &network,
                    command,
                    &mut apdu_rx,
                    &inbound_tx,
                    &terminal_tx,
                    &policy_tx,
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

    egress_open.store(false, Ordering::Release);
    egress_rx.close();
    while let Ok(command) = egress_rx.try_recv() {
        let _ = command.completion.send(Err(shutdown_error()));
    }
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

enum PendingEvent {
    Cancelled,
    Received(Option<ReceivedApdu>),
    Sent(Result<(), Error>),
}

#[allow(clippy::too_many_arguments)]
async fn drive_network_service<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    command: NetworkServiceCommand,
    apdu_rx: &mut mpsc::Receiver<ReceivedApdu>,
    inbound_tx: &mpsc::Sender<ReceivedApdu>,
    terminal_tx: &mpsc::Sender<ReceivedApdu>,
    policy_tx: &mpsc::Sender<PolicyOutcome>,
    cancel_rx: &mut oneshot::Receiver<()>,
    prefer_ingress: &mut bool,
) -> EgressDrive {
    let NetworkServiceCommand {
        apdu,
        destination,
        expecting_reply,
        priority,
        data_attributes,
        completion,
    } = command;
    let outcome = {
        let send = send_network_service_apdu(
            network,
            &apdu,
            &destination,
            expecting_reply,
            priority,
            &data_attributes,
        );
        tokio::pin!(send);
        loop {
            let event = if *prefer_ingress {
                tokio::select! {
                    biased;
                    _ = &mut *cancel_rx => PendingEvent::Cancelled,
                    received = apdu_rx.recv() => PendingEvent::Received(received),
                    result = &mut send => PendingEvent::Sent(result),
                }
            } else {
                tokio::select! {
                    biased;
                    _ = &mut *cancel_rx => PendingEvent::Cancelled,
                    result = &mut send => PendingEvent::Sent(result),
                    received = apdu_rx.recv() => PendingEvent::Received(received),
                }
            };

            match event {
                PendingEvent::Cancelled => break EgressDrive::Cancelled,
                PendingEvent::Received(Some(received)) => {
                    *prefer_ingress = !*prefer_ingress;
                    if let Some(exit) = route_received(received, inbound_tx, terminal_tx, policy_tx)
                    {
                        break EgressDrive::Exit(exit);
                    }
                }
                PendingEvent::Received(None) => {
                    break EgressDrive::Exit(ClassifierExit::InputClosed);
                }
                PendingEvent::Sent(result) => {
                    *prefer_ingress = !*prefer_ingress;
                    let _ = completion.send(result);
                    return EgressDrive::Complete;
                }
            }
        }
    };

    let _ = completion.send(Err(shutdown_error()));
    outcome
}

async fn send_network_service_apdu<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    apdu: &[u8],
    destination: &EndpointApduDestination,
    expecting_reply: bool,
    priority: NetworkPriority,
    data_attributes: &[DataAttribute],
) -> Result<(), Error> {
    validate_effective_group_apdu(apdu, destination)?;
    match destination {
        EndpointApduDestination::Direct { destination_mac } => {
            network
                .send_apdu_with_data_attributes(
                    apdu,
                    destination_mac,
                    expecting_reply,
                    priority,
                    data_attributes,
                )
                .await
        }
        EndpointApduDestination::Routed {
            destination_network,
            destination_mac,
            router_mac,
        } => {
            network
                .send_apdu_routed_with_data_attributes(
                    apdu,
                    *destination_network,
                    destination_mac,
                    router_mac,
                    expecting_reply,
                    priority,
                    data_attributes,
                )
                .await
        }
        EndpointApduDestination::RoutedViaLocalBroadcast {
            destination_network,
            destination_mac,
        } => {
            network
                .send_apdu_routed_via_local_broadcast_with_data_attributes(
                    apdu,
                    *destination_network,
                    destination_mac,
                    expecting_reply,
                    priority,
                    data_attributes,
                )
                .await
        }
        EndpointApduDestination::LocalBroadcast => {
            network
                .broadcast_apdu_with_data_attributes(
                    apdu,
                    expecting_reply,
                    priority,
                    data_attributes,
                )
                .await
        }
        EndpointApduDestination::RemoteBroadcast {
            destination_network,
        } => {
            network
                .broadcast_to_network_with_data_attributes(
                    apdu,
                    *destination_network,
                    expecting_reply,
                    priority,
                    data_attributes,
                )
                .await
        }
        EndpointApduDestination::GlobalBroadcast => {
            network
                .broadcast_global_apdu_with_data_attributes(
                    apdu,
                    expecting_reply,
                    priority,
                    data_attributes,
                )
                .await
        }
    }
}

fn validate_effective_group_apdu(
    apdu: &[u8],
    destination: &EndpointApduDestination,
) -> Result<(), Error> {
    if !matches!(
        destination,
        EndpointApduDestination::LocalBroadcast
            | EndpointApduDestination::RemoteBroadcast { .. }
            | EndpointApduDestination::GlobalBroadcast
    ) {
        return Ok(());
    }

    match decode_apdu(apdu.to_vec().into()) {
        Ok(Apdu::UnconfirmedRequest(_)) => Ok(()),
        _ => Err(Error::Encoding(EFFECTIVE_GROUP_APDU_ERROR.into())),
    }
}

fn route_received(
    received: ReceivedApdu,
    inbound_tx: &mpsc::Sender<ReceivedApdu>,
    terminal_tx: &mpsc::Sender<ReceivedApdu>,
    policy_tx: &mpsc::Sender<PolicyOutcome>,
) -> Option<ClassifierExit> {
    let route = match classify(&received) {
        Ok(route) => route,
        Err(reason) => return send_policy(policy_tx, PolicyOutcome { reason, received }),
    };

    let send_result = match route {
        IngressRoute::InboundRequest => inbound_tx.try_send(received),
        IngressRoute::TerminalOrSegment => terminal_tx.try_send(received),
    };
    if let Err(error) = send_result {
        let (reason, received) = match error {
            mpsc::error::TrySendError::Full(received) => (PolicyReason::RouteFull(route), received),
            mpsc::error::TrySendError::Closed(received) => {
                (PolicyReason::RouteClosed(route), received)
            }
        };
        return send_policy(policy_tx, PolicyOutcome { reason, received });
    }
    None
}

fn classify(received: &ReceivedApdu) -> Result<IngressRoute, PolicyReason> {
    let Some(first) = received.apdu.first() else {
        return Err(PolicyReason::MalformedApdu);
    };
    let pdu_type = (first >> 4) & 0x0f;
    if pdu_type > 7 {
        return Err(PolicyReason::UnsupportedPduType(pdu_type));
    }

    match decode_apdu(received.apdu.clone()) {
        Ok(Apdu::ConfirmedRequest(_) | Apdu::UnconfirmedRequest(_)) => {
            Ok(IngressRoute::InboundRequest)
        }
        Ok(
            Apdu::SimpleAck(_)
            | Apdu::ComplexAck(_)
            | Apdu::Error(_)
            | Apdu::Reject(_)
            | Apdu::Abort(_)
            | Apdu::SegmentAck(_),
        ) => Ok(IngressRoute::TerminalOrSegment),
        Err(_) => Err(PolicyReason::MalformedApdu),
    }
}

fn send_policy(
    policy_tx: &mpsc::Sender<PolicyOutcome>,
    outcome: PolicyOutcome,
) -> Option<ClassifierExit> {
    match policy_tx.try_send(outcome) {
        Ok(()) => None,
        Err(mpsc::error::TrySendError::Full(outcome)) => {
            Some(ClassifierExit::PolicyRouteFull(outcome))
        }
        Err(mpsc::error::TrySendError::Closed(outcome)) => {
            Some(ClassifierExit::PolicyRouteClosed(outcome))
        }
    }
}

#[cfg(test)]
#[path = "endpoint_ingress_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "endpoint_network_service_tests.rs"]
mod network_service_tests;
