use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_network::layer::{IssuedApdu, NetworkLayer, ReceivedApdu, RoutedTarget};
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
    /// Direct unicast on the local data link. A MAC that reaches a group of
    /// nodes here, such as the link's broadcast MAC or a multicast address,
    /// is a local broadcast, so it carries only an Unconfirmed-Request APDU
    /// (Clause 6.3, #1479).
    Direct {
        /// Destination MAC on the local data link.
        destination_mac: MacAddr,
    },
    /// Routed unicast through a known next-hop router. An empty
    /// `destination_mac` asks that router to broadcast on the network, so it
    /// carries only an Unconfirmed-Request APDU (Clause 6.3, #1479).
    Routed {
        /// Ultimate BACnet network number.
        destination_network: u16,
        /// Ultimate BACnet MAC address.
        destination_mac: MacAddr,
        /// Immediate router MAC on the local data link.
        router_mac: MacAddr,
    },
    /// Routed unicast using a local broadcast because the router MAC is unknown.
    /// An empty `destination_mac` is refused: [`Self::RemoteBroadcast`]
    /// sends that network's broadcast (#1479).
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

#[path = "endpoint_egress.rs"]
mod egress;
pub use egress::{EndpointEgress, EndpointEgressAdmissionError, EndpointSend, EndpointSendOutcome};
use egress::{NetworkServiceCommand, NetworkServicePayload};

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
pub struct IngressReceivers {
    /// Opted-in single-link local controls, currently NORMAL B/IP, B/IPv6 and SC;
    /// other links retain discard behavior.
    #[doc(hidden)]
    pub network_controls: Option<mpsc::Receiver<bacnet_network::layer::ReceivedNetworkControl>>,
    /// Post-bind registration capability, with the B/IP mode, and
    /// independently supported port capacity.
    #[doc(hidden)]
    pub bip_port: Option<(bacnet_transport::port::BipPort, u16)>,
    /// Actual announced IPv4 address and bound UDP port, when this is B/IP.
    #[doc(hidden)]
    pub bip_local_address: Option<std::net::SocketAddrV4>,
    /// Actual IPv4 B/IP broadcast endpoint after transport startup.
    #[doc(hidden)]
    pub bip_broadcast_endpoint: Option<std::net::SocketAddrV4>,
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
    PolicyRouteFull(Box<PolicyOutcome>),
    /// The policy queue was closed.
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

    /// Pre-start link capability; never retains or exposes a transport reference.
    #[doc(hidden)]
    pub fn bip_broadcast_endpoint(&self) -> Option<std::net::SocketAddrV4> {
        self.network.as_ref()?.transport().bip_broadcast_endpoint()
    }

    /// Pre-bind B/IP registration capability, with the configured mode.
    #[doc(hidden)]
    pub fn bip_port(&self) -> Option<bacnet_transport::port::BipPort> {
        self.network.as_ref()?.transport().bip_port()
    }
    /// Attach selected-object protection before starting the transport.
    #[doc(hidden)]
    pub fn retain_network_port_lease_internal(&mut self, lease: Arc<()>) -> Result<(), Error> {
        if self.lifecycle != Lifecycle::Ready {
            return Err(Error::Encoding("ingress already started".into()));
        }
        self.network
            .as_mut()
            .ok_or_else(|| Error::Encoding("missing network".into()))?
            .retain_network_port_lease_internal(lease)
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
        self.lifecycle = Lifecycle::Starting;
        let controls = if network
            .transport()
            .supports_local_nonrouter_number_controls()
        {
            Some(network.enable_network_control_receiver()?)
        } else {
            None
        };
        let apdu_rx = network.start().await?;
        let controls = network
            .transport()
            .supports_local_nonrouter_number_controls()
            .then_some(controls)
            .flatten();
        let bip_port = network
            .transport()
            .bip_port()
            .map(|port| (port, network.transport().local_receive_apdu_capacity()));
        let bip_broadcast_endpoint = network.transport().bip_broadcast_endpoint();
        let bip_local_address = bip_broadcast_endpoint.and_then(|_| {
            bacnet_transport::bvll::decode_bip_mac(network.local_mac())
                .ok()
                .map(|(ip, port)| std::net::SocketAddrV4::new(ip.into(), port))
        });
        let (inbound_tx, inbound_requests) = mpsc::channel(self.queue_capacity);
        let (terminal_tx, terminal_or_segment) = mpsc::channel(self.queue_capacity);
        let (policy_tx, policy_outcomes) = mpsc::channel(self.queue_capacity);
        let queues = IngressQueues {
            inbound_tx,
            terminal_tx,
            policy_tx,
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
        let network = self
            .network
            .take()
            .ok_or_else(|| Error::Encoding("endpoint ingress network owner is missing".into()))?;

        self.session_task = Some(tokio::spawn(session_task(
            network,
            apdu_rx,
            queues,
            egress_rx,
            cancel_rx,
            Arc::clone(&egress_open),
        )));
        self.cancel_tx = Some(cancel_tx);
        self.egress_open = Some(egress_open);
        self.lifecycle = Lifecycle::Running;

        Ok(IngressReceivers {
            network_controls: controls,
            bip_port,
            bip_local_address,
            bip_broadcast_endpoint,
            inbound_requests,
            terminal_or_segment,
            policy_outcomes,
            egress,
        })
    }

    /// Cancels classification, stops the network layer, and reports classifier exit.
    pub async fn stop(&mut self) -> Result<ClassifierExit, Error> {
        if !matches!(
            self.lifecycle,
            Lifecycle::Starting | Lifecycle::Running | Lifecycle::Stopping
        ) {
            return Err(Error::Encoding("endpoint ingress is not running".into()));
        }
        if self.lifecycle == Lifecycle::Starting {
            if let Some(mut network) = self.network.take() {
                self.session_task = Some(tokio::spawn(async move {
                    network.stop().await?;
                    Ok(ClassifierExit::Cancelled)
                }));
            }
        }
        self.lifecycle = Lifecycle::Stopping;
        if let Some(open) = self.egress_open.take() {
            open.store(false, Ordering::Release);
        }
        if let Some(cancel_tx) = self.cancel_tx.take() {
            let _ = cancel_tx.send(());
        }

        let result = match self.session_task.as_mut() {
            Some(task) => match task.await {
                Ok(result) => result,
                Err(error) => Err(Error::Encoding(format!(
                    "endpoint ingress session failed: {error}"
                ))),
            },
            None => Err(Error::Encoding(
                "endpoint ingress session task is missing".into(),
            )),
        };
        self.session_task.take();
        self.lifecycle = Lifecycle::Stopped;
        result
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

/// Senders for the queues a classified inbound APDU can be routed to.
struct IngressQueues {
    inbound_tx: mpsc::Sender<ReceivedApdu>,
    terminal_tx: mpsc::Sender<ReceivedApdu>,
    policy_tx: mpsc::Sender<PolicyOutcome>,
}

async fn session_task<T: TransportPort + 'static>(
    mut network: NetworkLayer<T>,
    mut apdu_rx: mpsc::Receiver<ReceivedApdu>,
    queues: IngressQueues,
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
                if let Some(exit) = route_received(received, &queues, network.transport()) {
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

    egress_open.store(false, Ordering::Release);
    egress_rx.close();
    while let Ok(command) = egress_rx.try_recv() {
        let _ = command.completion.send(EndpointSendOutcome {
            result: Err(shutdown_error()),
            attempted: false,
        });
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
    apdu_rx: &mut mpsc::Receiver<ReceivedApdu>,
    queues: &IngressQueues,
    cancel_rx: &mut oneshot::Receiver<()>,
    prefer_ingress: &mut bool,
) -> EgressDrive {
    let NetworkServiceCommand {
        payload,
        response_route,
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
            result: Err(Error::Encoding(
                "endpoint send expired before execution".into(),
            )),
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
                    if let Some(route) = &response_route {
                        let (next_hop, destination) = match &destination {
                            EndpointApduDestination::Direct { destination_mac } => {
                                (destination_mac, None)
                            }
                            EndpointApduDestination::Routed {
                                destination_network,
                                destination_mac,
                                router_mac,
                            } => (
                                router_mac,
                                Some(bacnet_encoding::npdu::NpduAddress {
                                    network: *destination_network,
                                    mac_address: destination_mac.clone(),
                                }),
                            ),
                            _ => {
                                return Err(Error::Encoding(
                                    "response requires a unicast destination".into(),
                                ))
                            }
                        };
                        network
                            .send_response_apdu_on_issuance(
                                IssuedApdu {
                                    apdu,
                                    next_hop,
                                    destination: destination.as_ref(),
                                    expecting_reply,
                                    priority,
                                },
                                route,
                                || {},
                            )
                            .await
                    } else {
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
                            result: Err(Error::Encoding("endpoint send deadline expired".into())), attempted: true,
                        });
                        return EgressDrive::Complete;
                    },
                    _ = completion.closed(), if cancel_on_drop => return EgressDrive::Complete,
                    received = apdu_rx.recv() => PendingEvent::Received(received),
                    result = &mut send => PendingEvent::Sent(result),
                }
            } else {
                tokio::select! {
                    biased;
                    _ = &mut *cancel_rx => PendingEvent::Cancelled,
                    _ = &mut expiry => {
                        let _ = completion.send(EndpointSendOutcome {
                            result: Err(Error::Encoding("endpoint send deadline expired".into())), attempted: true,
                        });
                        return EgressDrive::Complete;
                    },
                    _ = completion.closed(), if cancel_on_drop => return EgressDrive::Complete,
                    result = &mut send => PendingEvent::Sent(result),
                    received = apdu_rx.recv() => PendingEvent::Received(received),
                }
            };

            match event {
                PendingEvent::Cancelled => break EgressDrive::Cancelled,
                PendingEvent::Received(Some(received)) => {
                    *prefer_ingress = !*prefer_ingress;
                    if let Some(exit) = route_received(received, queues, network.transport()) {
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

async fn send_network_service_apdu<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    apdu: &[u8],
    destination: &EndpointApduDestination,
    expecting_reply: bool,
    priority: NetworkPriority,
    data_attributes: &[DataAttribute],
) -> Result<(), Error> {
    validate_effective_group_apdu(apdu, destination, |mac| {
        network.transport().is_group_destination(mac)
    })?;
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
                    RoutedTarget {
                        network: *destination_network,
                        mac: destination_mac,
                        router_mac,
                    },
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
        _ => Err(Error::Encoding(EFFECTIVE_GROUP_APDU_ERROR.into())),
    }
}

fn route_received(
    received: ReceivedApdu,
    queues: &IngressQueues,
    transport: &impl TransportPort,
) -> Option<ClassifierExit> {
    let IngressQueues {
        inbound_tx,
        terminal_tx,
        policy_tx,
    } = queues;
    let route = match classify(&received, transport) {
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

fn classify(
    received: &ReceivedApdu,
    transport: &impl TransportPort,
) -> Result<IngressRoute, PolicyReason> {
    let Some(first) = received.apdu.first() else {
        return Err(PolicyReason::MalformedApdu);
    };
    let pdu_type = (first >> 4) & 0x0f;
    if pdu_type > 7 {
        return Err(PolicyReason::UnsupportedPduType(pdu_type));
    }

    match decode_apdu(received.apdu.clone()) {
        Ok(Apdu::ConfirmedRequest(_)) if transport.is_group_destination(&received.source_mac) => {
            Err(PolicyReason::GroupSource)
        }
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
            Some(ClassifierExit::PolicyRouteFull(Box::new(outcome)))
        }
        Err(mpsc::error::TrySendError::Closed(outcome)) => {
            Some(ClassifierExit::PolicyRouteClosed(Box::new(outcome)))
        }
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
