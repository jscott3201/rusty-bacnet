//! Which queue one received APDU goes to: the endpoint's own request and
//! answer queues, and a full client's seat when one is attached (#2617).
//!
//! # A full queue (#2635)
//!
//! The classifier never waits for a queue, and a full one never ends the
//! session: what finds no room is dropped and counted by class
//! ([`IngressDropCounters`]), and logged at most once per
//! [`DROP_LOG_INTERVAL`]. Dropping an envelope releases its one-use reply
//! sender, which on MS/TP has the MAC send Reply Postponed at once, as the
//! policy owner does for an outcome it takes.
//!
//! - An Unconfirmed-Request opens no transaction and nobody answers it, so it
//!   may use only the part of the request queue above a reserve of a
//!   quarter ([`unconfirmed_share_full`]): a burst of them (the I-Ams a
//!   global Who-Is brings) leaves room for the confirmed requests the server
//!   role owes an answer.
//! - A Confirmed-Request that finds no room is dropped without an answer.
//!   Its requester sends it again after its APDU timeout (Clause 5.3.1); an
//!   Abort would end the transaction for good, need a send from here, and
//!   reflect a flood back at the address it claims.
//! - An answer, the endpoint's own or one no lease holds, that finds the
//!   answer queue full is dropped: the lease it belongs to ends by its
//!   owner's timeout and retry, as if the link had lost it. Requests never
//!   fill that queue.
//! - An APDU the classifier refuses goes to the policy queue, and is dropped
//!   when that is full.
use std::sync::atomic::AtomicU64;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use super::*;
use crate::coordinator::LeaseHolder;
use crate::seat::{Attached, SeatSlot};

/// ConfirmedCOVNotification, as a confirmed service choice octet.
const CONFIRMED_COV_NOTIFICATION: u8 = 1;
/// ConfirmedEventNotification, as a confirmed service choice octet.
const CONFIRMED_EVENT_NOTIFICATION: u8 = 2;

/// The least time between two log lines about full-queue drops: a flood is
/// reported, but its report can't become one.
pub const DROP_LOG_INTERVAL: Duration = Duration::from_secs(10);

/// Whether an Unconfirmed-Request has used up its share of `queue`: it may
/// fill the queue only down to a reserve of a quarter of its capacity
/// (none below four), which stays for confirmed requests and answers.
pub(crate) fn unconfirmed_share_full<T>(queue: &mpsc::Sender<T>) -> bool {
    queue.capacity() <= queue.max_capacity() / 4
}

/// What ingress dropped because a queue was full, by class, since the
/// endpoint was made. Each count saturates. The client seat's own drops are
/// in [`ClientSeatCounters`](crate::seat::ClientSeatCounters).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct IngressDropCounters {
    /// Unconfirmed-Requests past their share of the request queue.
    pub unconfirmed_full_drops: u64,
    /// Confirmed-Requests the full request queue had no room for; none was
    /// answered.
    pub confirmed_full_drops: u64,
    /// Answers and segments (Simple-ACK, Complex-ACK, Error, Reject, Abort,
    /// Segment-ACK) the full answer queue had no room for.
    pub answer_full_drops: u64,
    /// Refused APDUs (malformed, an unknown PDU type, a confirmed request
    /// from a group address) the full policy queue had no room for.
    pub policy_full_drops: u64,
}

impl IngressDropCounters {
    fn total(&self) -> u64 {
        self.unconfirmed_full_drops
            .saturating_add(self.confirmed_full_drops)
            .saturating_add(self.answer_full_drops)
            .saturating_add(self.policy_full_drops)
    }
}

/// The class a full-queue drop is counted under.
#[derive(Clone, Copy)]
pub(super) enum Dropped {
    Unconfirmed,
    Confirmed,
    Answer,
    Policy,
}

/// Ingress's full-queue drop counts, shared with the endpoint's owner.
#[derive(Default)]
pub(super) struct IngressDrops {
    unconfirmed: AtomicU64,
    confirmed: AtomicU64,
    answer: AtomicU64,
    policy: AtomicU64,
    /// When the last log line went out, and the total it reported.
    logged: Mutex<Option<(tokio::time::Instant, u64)>>,
}

impl IngressDrops {
    pub(super) fn counters(&self) -> IngressDropCounters {
        IngressDropCounters {
            unconfirmed_full_drops: self.unconfirmed.load(Ordering::Relaxed),
            confirmed_full_drops: self.confirmed.load(Ordering::Relaxed),
            answer_full_drops: self.answer.load(Ordering::Relaxed),
            policy_full_drops: self.policy.load(Ordering::Relaxed),
        }
    }

    /// Count one drop, and log the counts when the last line is at least
    /// [`DROP_LOG_INTERVAL`] old. Returns the drops that line reported,
    /// when it logged one.
    pub(super) fn add(&self, class: Dropped) -> Option<u64> {
        let counter = match class {
            Dropped::Unconfirmed => &self.unconfirmed,
            Dropped::Confirmed => &self.confirmed,
            Dropped::Answer => &self.answer,
            Dropped::Policy => &self.policy,
        };
        let _ = counter.try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
            count.checked_add(1)
        });
        let now = tokio::time::Instant::now();
        let mut logged = self.logged.lock().unwrap_or_else(PoisonError::into_inner);
        if logged.is_some_and(|(at, _)| now.duration_since(at) < DROP_LOG_INTERVAL) {
            return None;
        }
        let counters = self.counters();
        let total = counters.total();
        let since = total.saturating_sub(logged.map_or(0, |(_, reported)| reported));
        *logged = Some((now, total));
        drop(logged);
        tracing::warn!(
            dropped = since,
            unconfirmed = counters.unconfirmed_full_drops,
            confirmed = counters.confirmed_full_drops,
            answers = counters.answer_full_drops,
            refused = counters.policy_full_drops,
            "endpoint ingress dropped {since} APDUs a full queue had no room for since its last \
             report; it reports again at most every {} s",
            DROP_LOG_INTERVAL.as_secs()
        );
        Some(since)
    }
}

/// Senders for the queues a classified inbound APDU can be routed to.
pub(super) struct IngressQueues {
    pub(super) inbound_tx: mpsc::Sender<ReceivedApdu>,
    pub(super) terminal_tx: mpsc::Sender<ReceivedApdu>,
    pub(super) policy_tx: mpsc::Sender<PolicyOutcome>,
    /// The client seat, when one is attached.
    pub(super) seats: Arc<SeatSlot>,
    /// What a full queue dropped.
    pub(super) drops: Arc<IngressDrops>,
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
        drops,
    } = queues;
    let route = match classify(&received, transport) {
        Ok(route) => route,
        Err(reason) => return send_policy(policy_tx, drops, PolicyOutcome { reason, received }),
    };

    let mut received = Some(received);
    seats.with(|attached| {
        let apdu = received.as_ref()?;
        match seat_share(attached, route, apdu, local_network) {
            SeatShare::None => {}
            SeatShare::Copy => {
                if seats.takes_copy(attached, apdu) {
                    let _ = seats.offer(attached, seat_copy(apdu));
                }
            }
            // A seat whose client is gone gives the APDU back, and it goes
            // the way it would with no seat.
            SeatShare::Whole => received = seats.offer(attached, received.take()?),
        }
        Some(())
    });
    let received = received?;

    let (queue, class) = match route {
        IngressRoute::InboundRequest => {
            if !received.apdu.first().is_some_and(|first| first >> 4 == 1) {
                (inbound_tx, Dropped::Confirmed)
            } else if unconfirmed_share_full(inbound_tx) {
                drops.add(Dropped::Unconfirmed);
                return None;
            } else {
                (inbound_tx, Dropped::Unconfirmed)
            }
        }
        IngressRoute::TerminalOrSegment => (terminal_tx, Dropped::Answer),
    };
    match queue.try_send(received) {
        Ok(()) => None,
        // The envelope drops here, and its reply sender with it.
        Err(mpsc::error::TrySendError::Full(_)) => {
            drops.add(class);
            None
        }
        Err(mpsc::error::TrySendError::Closed(received)) => send_policy(
            policy_tx,
            drops,
            PolicyOutcome {
                reason: PolicyReason::RouteClosed(route),
                received,
            },
        ),
    }
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

/// Hand `outcome` to the policy owner. A full policy queue drops and counts
/// it; only a closed one, whose owner is gone, ends classification.
fn send_policy(
    policy_tx: &mpsc::Sender<PolicyOutcome>,
    drops: &IngressDrops,
    outcome: PolicyOutcome,
) -> Option<ClassifierExit> {
    match policy_tx.try_send(outcome) {
        Ok(()) => None,
        Err(mpsc::error::TrySendError::Full(_)) => {
            drops.add(Dropped::Policy);
            None
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
