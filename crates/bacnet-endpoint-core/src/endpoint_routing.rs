//! Which queue one received APDU goes to: the endpoint's own request and
//! answer queues, and a full client's seat when one is attached (#2617).
use super::*;
use crate::coordinator::LeaseHolder;
use crate::seat::{Attached, SeatSlot};

/// ConfirmedCOVNotification, as a confirmed service choice octet.
const CONFIRMED_COV_NOTIFICATION: u8 = 1;
/// ConfirmedEventNotification, as a confirmed service choice octet.
const CONFIRMED_EVENT_NOTIFICATION: u8 = 2;

/// Senders for the queues a classified inbound APDU can be routed to.
pub(super) struct IngressQueues {
    pub(super) inbound_tx: mpsc::Sender<ReceivedApdu>,
    pub(super) terminal_tx: mpsc::Sender<ReceivedApdu>,
    pub(super) policy_tx: mpsc::Sender<PolicyOutcome>,
    /// The client seat, when one is attached.
    pub(super) seats: Arc<SeatSlot>,
}

/// What the seat takes of one APDU.
enum SeatShare {
    /// Nothing: the endpoint's own queues take it, as with no seat.
    None,
    /// A copy with no reply authority; the endpoint's queue keeps the APDU.
    Copy,
    /// The APDU itself, with its reply authority.
    Whole,
}

/// The seat's share of a valid APDU, from its first octets (see
/// [`crate::seat`]). Only an answer's lease is looked up, and nothing is
/// claimed.
fn seat_share(
    attached: &Attached,
    route: IngressRoute,
    received: &ReceivedApdu,
    local_network: Option<u16>,
) -> SeatShare {
    let apdu = &received.apdu;
    let first = apdu.first().copied().unwrap_or(0);
    match (route, first >> 4) {
        (IngressRoute::InboundRequest, 1) => SeatShare::Copy,
        (IngressRoute::InboundRequest, _) if !attached.roles.server => SeatShare::Whole,
        (IngressRoute::InboundRequest, _) => {
            // The service choice follows the sequence number and window
            // size when the request is segmented.
            let service = if first & 0x08 == 0 { 3 } else { 5 };
            match apdu.get(service).copied() {
                Some(CONFIRMED_COV_NOTIFICATION) => SeatShare::Whole,
                Some(CONFIRMED_EVENT_NOTIFICATION) => SeatShare::Copy,
                _ => SeatShare::None,
            }
        }
        // An Abort (7) or Segment-ACK (4) with the server bit clear came
        // from a requester: it is about a request this node serves.
        (IngressRoute::TerminalOrSegment, 4 | 7) if first & 0x01 == 0 => SeatShare::None,
        (IngressRoute::TerminalOrSegment, _) => {
            let holder = apdu.get(1).and_then(|invoke_id| {
                attached.coordinator.holder_of_source(
                    &received.source_mac,
                    received.source_network.as_ref(),
                    local_network,
                    *invoke_id,
                )
            });
            if holder == Some(LeaseHolder::Client) {
                SeatShare::Whole
            } else {
                SeatShare::None
            }
        }
    }
}

/// The seat's copy of an APDU another role answers: the same octets, and no
/// way to reply.
fn seat_copy(received: &ReceivedApdu) -> ReceivedApdu {
    let mut copy = received.clone();
    copy.direct_response = None;
    copy.router_response = None;
    copy
}

pub(super) fn route_received(
    received: ReceivedApdu,
    queues: &IngressQueues,
    transport: &impl TransportPort,
    local_network: Option<u16>,
) -> Option<ClassifierExit> {
    let IngressQueues {
        inbound_tx,
        terminal_tx,
        policy_tx,
        seats,
    } = queues;
    let route = match classify(&received, transport) {
        Ok(route) => route,
        Err(reason) => return send_policy(policy_tx, PolicyOutcome { reason, received }),
    };

    let mut received = Some(received);
    seats.with(|attached| {
        let apdu = received.as_ref()?;
        match seat_share(attached, route, apdu, local_network) {
            SeatShare::None => {}
            SeatShare::Copy => {
                let _ = seats.offer(attached, seat_copy(apdu));
            }
            // A seat whose client is gone gives the APDU back, and it goes
            // the way it would with no seat.
            SeatShare::Whole => received = seats.offer(attached, received.take()?),
        }
        Some(())
    });
    let received = received?;

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

/// The layer's network controls, which only its APDU receive queues, shared
/// out: the endpoint's Number owner when the link takes the Number controls,
/// and a copy for the client seat when one is attached. Without either, a
/// control is discarded, as the layer discards one nobody asked for.
pub(super) struct ControlTee {
    pub(super) layer: mpsc::Receiver<bacnet_network::layer::ReceivedNetworkControl>,
    pub(super) numbers: Option<mpsc::Sender<bacnet_network::layer::ReceivedNetworkControl>>,
    pub(super) seats: Arc<SeatSlot>,
}

impl ControlTee {
    fn drain(&mut self) {
        while let Ok(control) = self.layer.try_recv() {
            self.seats.offer_control(&control);
            if let Some(numbers) = &self.numbers
                && let Err(mpsc::error::TrySendError::Closed(_)) = numbers.try_send(control)
            {
                self.numbers = None;
            }
        }
    }
}

/// The next APDU, as [`ApduReceiver::recv`](bacnet_network::layer::ApduReceiver::recv)
/// gives it, sharing out the controls each poll of the layer queued. Cancel
/// safe: a control taken from the layer is handed on in the same poll.
pub(super) async fn next_apdu(
    apdu_rx: &mut bacnet_network::layer::ApduReceiver,
    controls: &mut ControlTee,
) -> Option<ReceivedApdu> {
    std::future::poll_fn(|cx| {
        let polled = apdu_rx.poll_recv(cx);
        controls.drain();
        polled
    })
    .await
}
