//! Which queue one received APDU goes to: the endpoint's own request and
//! answer queues, a full client's seat and a full server's seat when they
//! are attached (#2617). The server seat, attached, takes the place of the
//! request queue.
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
//! Each queue keeps a [`reserve`] (a quarter of it, at least one slot from
//! two up) that the traffic it can best spare may not take:
//!
//! - An Unconfirmed-Request opens no transaction and nobody answers it, so it
//!   may fill the request queue only down to the reserve: a burst of them
//!   (the I-Ams a global Who-Is brings) leaves room for the confirmed
//!   requests the server role owes an answer.
//! - A Confirmed-Request that finds no room is dropped without an answer.
//!   This tier only queues; answering is the server role's, which refuses
//!   work at its own admission. A silent drop lets the requester's retry
//!   (Clause 5.3.1) succeed once a passing burst is over, where an Abort
//!   would end the transaction, and most clients don't retry one.
//! - An answer no lease holds may fill the answer queue only down to the
//!   reserve, so it can't take the last slots from an answer to one of the
//!   endpoint's own requests (found by a look at the coordinator that claims
//!   nothing, only once the queue is that full). An answer that still finds
//!   no room is dropped: its lease ends by its owner's timeout and retry, as
//!   if the link had lost it.
//! - Each seat has three tiers: Unconfirmed-Requests stop at half of it,
//!   Confirmed-Requests at its reserve, and answers to the role's own
//!   requests (and, on the server seat, a requester's Abort or Segment-ACK)
//!   may fill it ([`SeatSlot::takes`](crate::seat::SeatSlot::takes)). The
//!   client seat's copy of a requester's Abort stops at half, as
//!   Unconfirmed-Requests do.
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
/// ConfirmedCOVNotificationMultiple, as a confirmed service choice octet.
const CONFIRMED_COV_NOTIFICATION_MULTIPLE: u8 = 31;

/// The least time between two log lines about full-queue drops: a flood is
/// reported, but its report can't become one.
pub const DROP_LOG_INTERVAL: Duration = Duration::from_secs(10);

/// The slots of a queue of `capacity` that the traffic it can best spare
/// may not take: a quarter, and at least one from a capacity of two up. A
/// queue of one has none.
pub(crate) fn reserve(capacity: usize) -> usize {
    if capacity < 2 {
        0
    } else {
        (capacity / 4).max(1)
    }
}

/// Whether only `queue`'s [`reserve`] is left.
pub(crate) fn down_to_reserve<T>(queue: &mpsc::Sender<T>) -> bool {
    queue.capacity() <= reserve(queue.max_capacity())
}

/// Whether one of `coordinator`'s leases holds the answer `received`. Looks,
/// claims nothing. With no coordinator every answer counts as held.
///
/// A Segment-ACK or Abort a requester sends (server bit clear) is about a
/// request this node serves, not about a lease: none counts as held,
/// whatever lease its invoke ID happens to match (#2640).
fn held_by_lease(
    coordinator: Option<&OutboundTransactionCoordinator>,
    received: &ReceivedApdu,
    local_network: Option<u16>,
) -> bool {
    let Some(coordinator) = coordinator else {
        return true;
    };
    if from_requester(&received.apdu) {
        return false;
    }
    received.apdu.get(1).is_some_and(|invoke_id| {
        coordinator
            .holder_of_source(
                &received.source_mac,
                received.source_network.as_ref(),
                local_network,
                *invoke_id,
            )
            .is_some()
    })
}

/// What ingress dropped because a queue was full, by class, since the
/// endpoint was made. Each count saturates. The seats' own drops are in
/// [`ClientSeatCounters`](crate::seat::ClientSeatCounters) and
/// [`ServerSeatCounters`](crate::seat::ServerSeatCounters).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct IngressDropCounters {
    /// Unconfirmed-Requests past their share of the request queue.
    pub unconfirmed_full_drops: u64,
    /// Confirmed-Requests for the endpoint's own roles that its full request
    /// queue had no room for; none was answered. One that goes to a seat
    /// instead (a COV notification to the client, any on an endpoint with no
    /// server role, or any while a full server is seated) and finds no room
    /// there is counted in that seat's counters.
    pub confirmed_full_drops: u64,
    /// Answers and segments (Simple-ACK, Complex-ACK, Error, Reject, Abort,
    /// Segment-ACK) the full answer queue had no room for: those no lease
    /// holds once only its reserve was left, and any once it was full.
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

    /// The drops since `earlier`, class by class.
    fn since(&self, earlier: &Self) -> Self {
        Self {
            unconfirmed_full_drops: self
                .unconfirmed_full_drops
                .saturating_sub(earlier.unconfirmed_full_drops),
            confirmed_full_drops: self
                .confirmed_full_drops
                .saturating_sub(earlier.confirmed_full_drops),
            answer_full_drops: self
                .answer_full_drops
                .saturating_sub(earlier.answer_full_drops),
            policy_full_drops: self
                .policy_full_drops
                .saturating_sub(earlier.policy_full_drops),
        }
    }
}

/// What one log line about drops reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DropReport {
    /// The drops since the line before.
    pub(super) dropped: u64,
    /// Whether any of them was more than an Unconfirmed-Request, so the line
    /// is a warning; unconfirmed drops alone are routine (a large discovery)
    /// and go out at debug level.
    pub(super) warned: bool,
}

/// The class a full-queue drop is counted under.
#[derive(Clone, Copy)]
pub(super) enum Dropped {
    Unconfirmed,
    Confirmed,
    Answer,
    Policy,
}

/// When each kind of drop line last went out, and the counts the last line
/// of either kind covered.
#[derive(Default)]
struct DropLog {
    /// The last debug line: unconfirmed drops alone, routine.
    debug: Option<tokio::time::Instant>,
    /// The last warning: any other class. A debug line never delays one.
    warn: Option<tokio::time::Instant>,
    reported: IngressDropCounters,
}

/// Ingress's full-queue drop counts, shared with the endpoint's owner.
#[derive(Default)]
pub(super) struct IngressDrops {
    unconfirmed: AtomicU64,
    confirmed: AtomicU64,
    answer: AtomicU64,
    policy: AtomicU64,
    logged: Mutex<DropLog>,
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

    /// Count one drop, and log the drops no line has reported yet when the
    /// last line of their kind is at least [`DROP_LOG_INTERVAL`] old: a
    /// warning when any is more than an Unconfirmed-Request, a debug line
    /// otherwise. Each kind keeps its own interval. Returns what was logged.
    pub(super) fn add(&self, class: Dropped) -> Option<DropReport> {
        let counter = match class {
            Dropped::Unconfirmed => &self.unconfirmed,
            Dropped::Confirmed => &self.confirmed,
            Dropped::Answer => &self.answer,
            Dropped::Policy => &self.policy,
        };
        let _ = counter.try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
            count.checked_add(1)
        });
        self.report(false)
    }

    /// Log the drops no line has reported yet, if any: when classification
    /// ends, so the tail of a burst is reported without a timer.
    pub(super) fn flush(&self) -> Option<DropReport> {
        self.report(true)
    }

    fn report(&self, force: bool) -> Option<DropReport> {
        let now = tokio::time::Instant::now();
        let mut log = self.logged.lock().unwrap_or_else(PoisonError::into_inner);
        let counters = self.counters();
        let since = counters.since(&log.reported);
        let dropped = since.total();
        if dropped == 0 {
            return None;
        }
        let warned = dropped != since.unconfirmed_full_drops;
        let last = if warned { log.warn } else { log.debug };
        if !force && last.is_some_and(|at| now.duration_since(at) < DROP_LOG_INTERVAL) {
            return None;
        }
        // A warning covers the unconfirmed drops too, so it also counts as
        // the debug line of its interval.
        log.debug = Some(now);
        if warned {
            log.warn = Some(now);
        }
        log.reported = counters;
        drop(log);
        macro_rules! line {
            ($level:ident) => {
                tracing::$level!(
                    dropped,
                    unconfirmed = since.unconfirmed_full_drops,
                    confirmed = since.confirmed_full_drops,
                    answers = since.answer_full_drops,
                    refused = since.policy_full_drops,
                    "endpoint ingress dropped {dropped} APDUs a full queue had no room for \
                     since its last report; it reports again at most every {} s",
                    DROP_LOG_INTERVAL.as_secs()
                )
            };
        }
        if warned {
            line!(warn);
        } else {
            line!(debug);
        }
        Some(DropReport { dropped, warned })
    }
}

/// Senders for the queues a classified inbound APDU can be routed to.
pub(super) struct IngressQueues {
    pub(super) inbound_tx: mpsc::Sender<ReceivedApdu>,
    pub(super) terminal_tx: mpsc::Sender<ReceivedApdu>,
    pub(super) policy_tx: mpsc::Sender<PolicyOutcome>,
    /// The client and server seats, when attached.
    pub(super) seats: Arc<SeatSlot>,
    /// What a full queue dropped.
    pub(super) drops: Arc<IngressDrops>,
    /// The endpoint's invoke-ID pool, whose leases' answers come before
    /// answers no lease holds.
    pub(super) coordinator: Option<Arc<OutboundTransactionCoordinator>>,
    /// See [`EndpointIngress::observe_sources`].
    pub(super) observer: Option<SourceObserver>,
}

/// Show `observer` what came from another network: every request, and an
/// answer one of `coordinator`'s leases holds, whichever role takes it. The
/// lease must match the answer's peer and invoke ID and, where the PDU
/// carries a service (a Simple-ACK, an Error, an unsegmented Complex-ACK),
/// its service; a Reject, an Abort, a Segment-ACK or a segment is matched
/// by its peer and invoke ID alone. Admission's other checks, the lease's
/// owner and the answers it takes, are not made here.
fn observe_source(
    observer: &SourceObserver,
    coordinator: Option<&OutboundTransactionCoordinator>,
    route: IngressRoute,
    received: &ReceivedApdu,
    local_network: Option<u16>,
) {
    if received.source_network.is_none() {
        return;
    }
    let held = || {
        let apdu = &received.apdu;
        if route != IngressRoute::TerminalOrSegment || from_requester(apdu) {
            return false;
        }
        let (Some(pool), Some(invoke_id)) = (coordinator, apdu.get(1)) else {
            return false;
        };
        let source = received.source_network.as_ref();
        pool.lease_of_source(&received.source_mac, source, local_network, *invoke_id)
            .is_some_and(|(_, service)| answer_service(apdu).is_none_or(|s| s == service.to_raw()))
    };
    if route == IngressRoute::InboundRequest || held() {
        observer(received);
    }
}

/// The service an answer's octets name, where admission checks it: a
/// Simple-ACK's, an Error's, an unsegmented Complex-ACK's.
fn answer_service(apdu: &[u8]) -> Option<u8> {
    let first = *apdu.first()?;
    match first >> 4 {
        2 | 5 => apdu.get(2).copied(),
        3 if first & 0x08 == 0 => apdu.get(2).copied(),
        _ => None,
    }
}

/// What a seat takes of one APDU.
enum SeatShare {
    /// Nothing: the endpoint's own queues take it, as with no seat.
    None,
    /// A copy with no reply authority; the endpoint's queue keeps the APDU.
    Copy,
    /// The APDU itself, with its reply authority.
    Whole,
    /// The APDU itself, answering nothing the node asked: held to the share
    /// a copy may take, so a flood of these leaves room for the client's
    /// own answers (#2635).
    Stray,
    /// A copy with no reply authority, held to the share a [`Self::Stray`]
    /// may take: a requester's Abort, which may end a request the client
    /// is reassembling (#2646). The endpoint's queue keeps the APDU.
    StrayCopy,
}

/// Whether `apdu` is an Abort (7) or Segment-ACK (4) with the server bit
/// clear: one a requester sent, about a request this node serves.
fn from_requester(apdu: &[u8]) -> bool {
    apdu.first()
        .is_some_and(|first| matches!(first >> 4, 4 | 7) && first & 0x01 == 0)
}

/// Whether `apdu` is an Abort (7) with the server bit clear: a requester
/// ending a request, which may be one the client reassembles.
fn requester_abort(apdu: &[u8]) -> bool {
    apdu.first()
        .is_some_and(|first| first >> 4 == 7 && first & 0x01 == 0)
}

/// Whether `apdu` is a segmented Complex-ACK (3 with the segmented-message
/// bit) or a Segment-ACK (4) the answering side sent (server bit set). With
/// no lease to hold one, it is the client's: in its idle state Clause 5.4.4.1
/// has it abort either. Every other answer no lease holds, a server's Abort
/// among them, the idle client would only drop, so it stays with the
/// endpoint's answer queue, which counts it.
fn client_stray(apdu: &[u8]) -> bool {
    apdu.first().is_some_and(|first| match first >> 4 {
        3 => first & 0x08 != 0,
        4 => first & 0x01 != 0,
        _ => false,
    })
}

/// The role whose lease holds the answer `received`, if any: looked up once
/// for both seats, claiming nothing. Requests and a requester's Abort or
/// Segment-ACK have none.
fn lease_holder(
    coordinator: &OutboundTransactionCoordinator,
    route: IngressRoute,
    received: &ReceivedApdu,
    local_network: Option<u16>,
) -> Option<LeaseHolder> {
    if route != IngressRoute::TerminalOrSegment || from_requester(&received.apdu) {
        return None;
    }
    received.apdu.get(1).and_then(|invoke_id| {
        coordinator.holder_of_source(
            &received.source_mac,
            received.source_network.as_ref(),
            local_network,
            *invoke_id,
        )
    })
}

/// The client seat's share of a valid APDU, from its first octets and the
/// answer's lease holder (see [`crate::seat`]).
fn client_share(
    attached: &Attached,
    route: IngressRoute,
    received: &ReceivedApdu,
    holder: Option<LeaseHolder>,
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
                // A COV notification of either form is about the client's
                // own subscription: the client answers it (#2654).
                Some(CONFIRMED_COV_NOTIFICATION | CONFIRMED_COV_NOTIFICATION_MULTIPLE) => {
                    SeatShare::Whole
                }
                Some(CONFIRMED_EVENT_NOTIFICATION) => SeatShare::Copy,
                _ => SeatShare::None,
            }
        }
        (IngressRoute::TerminalOrSegment, _) if holder == Some(LeaseHolder::Client) => {
            SeatShare::Whole
        }
        // No lease holds it: a segmented answer is the client's to abort,
        // and a requester's Abort may end a request it reassembles.
        (IngressRoute::TerminalOrSegment, _) if holder.is_none() => {
            if client_stray(apdu) {
                SeatShare::Stray
            } else if requester_abort(apdu) {
                SeatShare::StrayCopy
            } else {
                SeatShare::None
            }
        }
        (IngressRoute::TerminalOrSegment, _) => SeatShare::None,
    }
}

/// Whether the server seat takes what the client seat left of a valid APDU:
/// every request (it takes the endpoint's request queue's place), a
/// requester's Abort or Segment-ACK, and an answer to the server's own
/// request.
fn server_takes(route: IngressRoute, received: &ReceivedApdu, holder: Option<LeaseHolder>) -> bool {
    match route {
        IngressRoute::InboundRequest => true,
        IngressRoute::TerminalOrSegment => {
            holder == Some(LeaseHolder::Server) || from_requester(&received.apdu)
        }
    }
}

/// Offer what is left of `received` to the seats attached (see
/// [`crate::seat`]). What comes back is the endpoint's own queues'; `None`
/// when a seat took it, or dropped it as the seat's alone.
fn share_out(
    seats: &SeatSlot,
    route: IngressRoute,
    received: ReceivedApdu,
    local_network: Option<u16>,
) -> Option<ReceivedApdu> {
    let mut received = Some(received);
    seats.with(|client, server| {
        let holder = client.or(server).and_then(|seat| {
            lease_holder(&seat.coordinator, route, received.as_ref()?, local_network)
        });
        if let Some(client) = client {
            let apdu = received.as_ref()?;
            match client_share(client, route, apdu, holder) {
                SeatShare::None => {}
                SeatShare::Copy => {
                    if seats.takes(client, apdu) {
                        let _ = seats.offer(client, seat_copy(apdu));
                    }
                }
                // The seat's alone: dropped when it has no room, never
                // handed to another role in the client's place. A seat whose
                // client is gone gives the APDU back, and it goes the way it
                // would with no seat.
                SeatShare::Whole if seats.takes(client, apdu) => {
                    received = seats.offer(client, received.take()?)
                }
                SeatShare::Whole => received = None,
                SeatShare::Stray if seats.takes_stray(client) => {
                    received = seats.offer(client, received.take()?)
                }
                SeatShare::Stray => received = None,
                SeatShare::StrayCopy => {
                    if seats.takes_stray(client) {
                        let _ = seats.offer(client, seat_copy(apdu));
                    }
                }
            }
        }
        if let Some(server) = server {
            let apdu = received.as_ref()?;
            if server_takes(route, apdu, holder) {
                // As for the client: dropped when it has no room, and given
                // back only when the server is gone.
                received = if seats.takes(server, apdu) {
                    seats.offer(server, received.take()?)
                } else {
                    None
                };
            }
        }
        Some(())
    });
    received
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
        coordinator,
        observer,
    } = queues;
    let route = match classify(&received, transport) {
        Ok(route) => route,
        Err(reason) => return send_policy(policy_tx, drops, PolicyOutcome { reason, received }),
    };
    if let Some(observer) = observer {
        let pool = coordinator.as_deref();
        observe_source(observer, pool, route, &received, local_network);
    }

    let received = share_out(seats, route, received, local_network)?;

    let (queue, class) = match route {
        IngressRoute::InboundRequest => {
            if !received.apdu.first().is_some_and(|first| first >> 4 == 1) {
                (inbound_tx, Dropped::Confirmed)
            } else if down_to_reserve(inbound_tx) {
                drops.add(Dropped::Unconfirmed);
                return None;
            } else {
                (inbound_tx, Dropped::Unconfirmed)
            }
        }
        IngressRoute::TerminalOrSegment => {
            // Only once the queue is that full is the coordinator asked.
            if down_to_reserve(terminal_tx)
                && !held_by_lease(coordinator.as_deref(), &received, local_network)
            {
                drops.add(Dropped::Answer);
                return None;
            }
            (terminal_tx, Dropped::Answer)
        }
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
/// and a copy for the client seat when one is attached; the server seat
/// takes none. Without either, a
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
