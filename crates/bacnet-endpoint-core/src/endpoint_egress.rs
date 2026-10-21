//! Bounded endpoint send admission and explicit completion ownership.
use super::*;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_network::network_number::LocalNetworkNumber;
use bacnet_network::response_route::ResponseRoute;
use bacnet_transport::port::GroupDestinations;

pub(super) enum NetworkServicePayload {
    Apdu(Vec<u8>),
    LocalControl(Vec<u8>),
}

pub(super) struct NetworkServiceCommand {
    pub(super) payload: NetworkServicePayload,
    pub(super) response_route: Option<ResponseRoute>,
    pub(super) destination: EndpointApduDestination,
    pub(super) expecting_reply: bool,
    pub(super) priority: NetworkPriority,
    pub(super) data_attributes: Vec<DataAttribute>,
    pub(super) completion: oneshot::Sender<EndpointSendOutcome>,
    pub(super) deadline: Option<tokio::time::Instant>,
    pub(super) cancel_on_drop: bool,
}

/// One APDU send being admitted to the bounded command queue.
struct ApduAdmission {
    apdu: Vec<u8>,
    destination: EndpointApduDestination,
    expecting_reply: bool,
    priority: NetworkPriority,
    data_attributes: Vec<DataAttribute>,
    /// Latest time the send may start, if bounded.
    deadline: Option<tokio::time::Instant>,
    /// Whether dropping the completion retracts the send.
    cancel_on_drop: bool,
    /// Immutable ingress authority a reply must follow, for response sends.
    response_route: Option<ResponseRoute>,
}

/// Completion of an admitted endpoint send, without claiming remote receipt.
#[doc(hidden)]
pub struct EndpointSendOutcome {
    /// Local send completion; success is not remote receipt or execution.
    pub result: Result<(), Error>,
    /// Transport execution began; an error may have an ambiguous wire outcome.
    pub attempted: bool,
}

/// One queued send completion. Deadline-bound and explicitly owned sends cancel
/// when this is dropped; ordinary deadline-free admission retains detached semantics.
#[doc(hidden)]
pub struct EndpointSend(oneshot::Receiver<EndpointSendOutcome>);

impl EndpointSend {
    #[doc(hidden)]
    pub async fn complete(self) -> EndpointSendOutcome {
        self.0.await.unwrap_or_else(|_| EndpointSendOutcome {
            result: Err(shutdown_error()),
            attempted: false,
        })
    }
}

/// Known local outcome before any endpoint transport execution.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointEgressAdmissionError {
    /// The bounded command queue has no free slot.
    QueueFull,
    /// The endpoint has sealed or its command receiver has closed.
    Closed,
}

impl From<EndpointEgressAdmissionError> for Error {
    fn from(error: EndpointEgressAdmissionError) -> Self {
        match error {
            EndpointEgressAdmissionError::QueueFull => {
                Error::Encoding("endpoint egress queue is full".into())
            }
            EndpointEgressAdmissionError::Closed => shutdown_error(),
        }
    }
}

/// Bounded APDU network-service sender for roles attached to an endpoint session.
#[doc(hidden)]
#[derive(Clone)]
pub struct EndpointEgress {
    pub(super) commands: mpsc::Sender<NetworkServiceCommand>,
    pub(super) open: Arc<AtomicBool>,
    pub(super) local_network: LocalNetworkNumber,
    /// The link's group-destination rule, taken from the transport at start.
    pub(super) group_destinations: GroupDestinations,
}

impl EndpointApduDestination {
    /// This destination as sent once the endpoint knows `local_network`, the
    /// number of the network its link is attached to (#1403). A routed
    /// destination on that network is a station on this link, so it goes to
    /// the station's MAC with no DNET and not through the router (Clause
    /// 6.5.1): a non-routing peer discards an APDU whose DNET names a network
    /// (Clause 6.5.2.1). A broadcast to that network is a local broadcast.
    /// Every other destination, and every one while the number is unknown,
    /// is unchanged. It checks nothing itself: the requester checks a routed
    /// unicast's DADR before calling it, and source Audit applies its stricter
    /// B/IP unicast check after.
    #[doc(hidden)]
    pub fn localized(self, local_network: Option<u16>) -> Self {
        let here = |network: u16| Some(network) == local_network;
        match self {
            Self::Routed {
                destination_network,
                destination_mac,
                ..
            }
            | Self::RoutedViaLocalBroadcast {
                destination_network,
                destination_mac,
            } if here(destination_network) => Self::Direct { destination_mac },
            Self::RemoteBroadcast {
                destination_network,
            } if here(destination_network) => Self::LocalBroadcast,
            destination => destination,
        }
    }
}

impl EndpointEgress {
    /// The number of the network this endpoint's link is attached to: the
    /// layer's own [`LocalNetworkNumber`], which the endpoint's Number owner
    /// publishes. Senders read it to send traffic for that network as local
    /// traffic ([`EndpointApduDestination::localized`]); the egress itself
    /// sends every destination as its caller names it, so answers keep the
    /// route their request arrived by.
    #[doc(hidden)]
    pub fn local_network_number(&self) -> &LocalNetworkNumber {
        &self.local_network
    }

    /// Whether a `Direct` send to `mac` would reach a group of nodes, by the
    /// rule the transport gave at start
    /// ([`TransportPort::group_destinations`](bacnet_transport::port::TransportPort::group_destinations)).
    /// The egress refuses anything but an Unconfirmed-Request there when it
    /// sends; a sender that takes state for a send, such as a transaction,
    /// asks here first (#1479).
    #[doc(hidden)]
    pub fn is_group_destination(&self, mac: &[u8]) -> bool {
        self.group_destinations.contains(mac)
    }

    /// Whether this ingress still admits transport work.
    #[doc(hidden)]
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire) && !self.commands.is_closed()
    }

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
        self.admit_apdu(
            apdu,
            destination,
            expecting_reply,
            priority,
            data_attributes,
            None,
        )?
        .complete()
        .await
        .result
    }

    /// Queue one send synchronously, distinguishing admission from execution.
    /// A deadline also makes receiver cancellation retract queued work. Sends
    /// already in progress may have reached the peer when cancellation wins.
    #[doc(hidden)]
    pub fn admit_apdu(
        &self,
        apdu: Vec<u8>,
        destination: EndpointApduDestination,
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: Vec<DataAttribute>,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<EndpointSend, EndpointEgressAdmissionError> {
        self.admit(ApduAdmission {
            apdu,
            destination,
            expecting_reply,
            priority,
            data_attributes,
            deadline,
            cancel_on_drop: deadline.is_some(),
            response_route: None,
        })
    }

    /// Queue caller-owned work without a deadline. Dropping its completion retracts
    /// queued work and cancels an in-progress transport future, whose wire outcome
    /// may already be ambiguous. A session-owned caller can retain this same guard.
    #[doc(hidden)]
    pub fn admit_owned_apdu(
        &self,
        apdu: Vec<u8>,
        destination: EndpointApduDestination,
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: Vec<DataAttribute>,
    ) -> Result<EndpointSend, EndpointEgressAdmissionError> {
        self.admit(ApduAdmission {
            apdu,
            destination,
            expecting_reply,
            priority,
            data_attributes,
            deadline: None,
            cancel_on_drop: true,
            response_route: None,
        })
    }

    /// Queue a caller-owned reply under its immutable ingress authority.
    /// Dropping completion retracts queued work; a started socket write may
    /// already have reached the peer. Invalid/stale direct routes never fall back.
    #[doc(hidden)]
    pub fn admit_response_apdu(
        &self,
        apdu: Vec<u8>,
        next_hop: MacAddr,
        destination: Option<NpduAddress>,
        route: ResponseRoute,
    ) -> Result<EndpointSend, EndpointEgressAdmissionError> {
        let destination = match destination {
            Some(address) => EndpointApduDestination::Routed {
                destination_network: address.network,
                destination_mac: address.mac_address,
                router_mac: next_hop,
            },
            None => EndpointApduDestination::Direct {
                destination_mac: next_hop,
            },
        };
        self.admit(ApduAdmission {
            apdu,
            destination,
            expecting_reply: false,
            priority: NetworkPriority::Normal,
            data_attributes: Vec::new(),
            deadline: None,
            cancel_on_drop: true,
            response_route: Some(route),
        })
    }

    fn admit(&self, send: ApduAdmission) -> Result<EndpointSend, EndpointEgressAdmissionError> {
        let ApduAdmission {
            apdu,
            destination,
            expecting_reply,
            priority,
            data_attributes,
            deadline,
            cancel_on_drop,
            response_route,
        } = send;
        if !self.open.load(Ordering::Acquire) {
            return Err(EndpointEgressAdmissionError::Closed);
        }
        let (completion, result) = oneshot::channel();
        let command = NetworkServiceCommand {
            payload: NetworkServicePayload::Apdu(apdu),
            response_route,
            destination,
            expecting_reply,
            priority,
            data_attributes,
            completion,
            deadline,
            cancel_on_drop,
        };
        match self.commands.try_send(command) {
            Ok(()) => Ok(EndpointSend(result)),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(EndpointEgressAdmissionError::Closed),
            Err(mpsc::error::TrySendError::Full(_)) => Err(EndpointEgressAdmissionError::QueueFull),
        }
    }

    /// Queue a local Network-Number-Is NPDU, with caller-owned cancellation.
    /// Dropping the waiter retracts queued work; a started send may have hit wire.
    #[doc(hidden)]
    pub async fn send_network_number_is(&self, npdu: Vec<u8>) -> Result<(), Error> {
        if npdu.len() != 6
            || npdu[..3] != [1, 0x80, 0x13]
            || npdu[5] > 1
            || matches!(u16::from_be_bytes([npdu[3], npdu[4]]), 0 | 65535)
        {
            return Err(Error::Encoding(
                "invalid local Network-Number-Is NPDU".into(),
            ));
        }
        if !self.is_open() {
            return Err(shutdown_error());
        }
        let (completion, result) = oneshot::channel();
        self.commands
            .try_send(NetworkServiceCommand {
                payload: NetworkServicePayload::LocalControl(npdu),
                response_route: None,
                destination: EndpointApduDestination::LocalBroadcast,
                expecting_reply: false,
                priority: NetworkPriority::Normal,
                data_attributes: Vec::new(),
                completion,
                deadline: None,
                cancel_on_drop: true,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Closed(_) => {
                    Error::from(EndpointEgressAdmissionError::Closed)
                }
                mpsc::error::TrySendError::Full(_) => {
                    Error::from(EndpointEgressAdmissionError::QueueFull)
                }
            })?;
        EndpointSend(result).complete().await.result
    }

    /// Wait for queue capacity without retaining an ordinary notification.
    /// This is a wake hint, not a reserved slot; callers retry synchronous admission.
    #[doc(hidden)]
    pub async fn wait_for_capacity(&self) -> Result<(), EndpointEgressAdmissionError> {
        if !self.open.load(Ordering::Acquire) {
            return Err(EndpointEgressAdmissionError::Closed);
        }
        let permit = self
            .commands
            .reserve()
            .await
            .map_err(|_| EndpointEgressAdmissionError::Closed)?;
        drop(permit);
        if self.open.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(EndpointEgressAdmissionError::Closed)
        }
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
