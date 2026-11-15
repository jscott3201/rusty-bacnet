//! The client seat: what a full client is handed to run on an endpoint's
//! link beside the endpoint's own roles (#2617).
//!
//! An endpoint owns one transport, one [`NetworkLayer`] and one
//! [`OutboundTransactionCoordinator`]. A [`ClientSeat`] shares the last two
//! with a client that would otherwise make its own, and gives it the APDUs
//! and network controls that are its to take. Nothing is encoded again and
//! no invoke ID is translated: the client sends on the shared layer itself,
//! and leases its invoke IDs from the shared coordinator, so they are unique
//! across the node.
//!
//! # Who receives what
//!
//! Endpoint ingress decides from an APDU's first octets, after its own
//! validity checks:
//!
//! - an Unconfirmed-Request goes to the endpoint's request queue, and a copy
//!   with no reply authority to the seat;
//! - a Confirmed-Request goes to the endpoint's request queue, for its
//!   server role. A ConfirmedCOVNotification goes to the seat instead, with
//!   its reply authority. A ConfirmedEventNotification goes to the server
//!   role, and a copy with no reply authority to the seat. On an endpoint
//!   with no server role every Confirmed-Request goes to the seat;
//! - an answer (Simple-ACK, Complex-ACK and its segments, Error, Reject, and
//!   an Abort or Segment-ACK sent by the answering side) goes to the seat
//!   when [`OutboundTransactionCoordinator::holder_of_source`] names a full
//!   client's lease for its source and invoke ID, and to the endpoint's
//!   answer queue otherwise, where an answer no lease claims is counted and
//!   dropped;
//! - an Abort or Segment-ACK sent by a requester goes to the endpoint's
//!   answer queue;
//! - every network control goes to the seat, beside the endpoint's own
//!   Number owner.
//!
//! Ingress never waits for the seat. An APDU or control that finds the
//! seat's queue full is dropped and counted ([`ClientSeatCounters`]), so a
//! client that stops reading holds nothing else up. The copy of an
//! Unconfirmed-Request finds it full once only a quarter of it is left
//! (#2635): a burst of I-Ams can't crowd out the answers to the client's
//! own requests.
//!
//! # Lifetime
//!
//! The endpoint stops the link. Its stop closes the seat's queues and aborts
//! the client's tasks, which the client names when it starts
//! ([`SeatHolds::abort_on_stop`]), then waits for each task's [`SeatHold`]
//! before it stops the layer, so none of them is still at work: a task in
//! the middle of a turn can't hold the stop up. A client that is still alive
//! keeps its own reference to the layer, and the layer stops all the same,
//! through that shared reference (#2617): the client's later requests fail
//! with the transport's stopped error, and its drop changes nothing.

use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use bacnet_network::layer::{NetworkLayer, ReceivedApdu, ReceivedNetworkControl};
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use tokio::sync::{Notify, mpsc};

use crate::coordinator::OutboundTransactionCoordinator;

/// APDUs a client seat queues by default before ingress drops for it. A
/// Who-Is can bring hundreds of I-Ams at once, and the seat's queue is the
/// only buffer between the endpoint's ingress and the client.
pub const CLIENT_SEAT_QUEUE: usize = 256;

/// Network controls a client seat queues before ingress drops for it.
pub const CLIENT_SEAT_CONTROL_QUEUE: usize = 64;

/// The other roles attached to the node a seat is on. Fixed when the seat
/// is made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SeatRoles {
    /// A server role answers for the device. The client then answers only
    /// what is its own (a ConfirmedCOVNotification): it rejects no
    /// Confirmed-Request and acknowledges no ConfirmedEventNotification.
    pub server: bool,
}

impl SeatRoles {
    /// The roles beside a client seat: whether a server role is attached.
    pub fn new(server: bool) -> Self {
        Self { server }
    }
}

/// Held by each task that keeps a seat's network layer, and dropped with
/// it, so the endpoint's stop can wait for those tasks before it stops the
/// layer. Keep it in a field after the layer's `Arc`: fields drop in order,
/// so the layer has been let go when the hold is seen to drop.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct SeatHold(#[allow(dead_code)] mpsc::Sender<()>);

/// What the endpoint knows of the client on a seat.
#[derive(Default)]
struct SeatedClient {
    /// The client's tasks, aborted when the endpoint stops.
    tasks: Vec<tokio::task::AbortHandle>,
    /// Dead once the client itself is dropped.
    alive: Option<Weak<dyn Any + Send + Sync>>,
}

/// A seat's line back to its endpoint: the [`SeatHold`]s of the client's
/// tasks, the tasks themselves, and whether the client still lives. It is
/// not a hold itself, so a seat nobody started a client on doesn't keep the
/// endpoint's stop waiting.
#[doc(hidden)]
#[derive(Clone)]
pub struct SeatHolds {
    holds: mpsc::WeakSender<()>,
    client: Arc<Mutex<SeatedClient>>,
}

impl SeatHolds {
    fn client(&self) -> std::sync::MutexGuard<'_, SeatedClient> {
        self.client.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A hold for one task, or `None` once the endpoint is stopping.
    pub fn hold(&self) -> Option<SeatHold> {
        self.holds.upgrade().map(SeatHold)
    }

    /// Have the endpoint's stop abort `task`, which keeps the layer: the
    /// stop then never waits on that task's own progress.
    pub fn abort_on_stop(&self, task: tokio::task::AbortHandle) {
        self.client().tasks.push(task);
    }

    /// The client lives while `client` can be upgraded. Once it can't, the
    /// endpoint takes another client at once, without waiting for the
    /// dropped one's tasks to finish ending.
    pub fn alive_while(&self, client: Weak<dyn Any + Send + Sync>) {
        self.client().alive = Some(client);
    }
}

/// Tells a seated client that the node's Number owner may have changed the
/// layer's local network number. The seated client runs no Number owner of
/// its own; it folds its discovery table when this fires.
#[doc(hidden)]
#[derive(Clone, Debug, Default)]
pub struct SeatNumber(Arc<Notify>);

impl SeatNumber {
    /// The node's Number owner published the number.
    pub fn published(&self) {
        self.0.notify_one();
    }

    /// Wait for the next publication. One made while nothing waited is kept.
    pub async fn changed(&self) {
        self.0.notified().await;
    }
}

/// What a full client is handed in place of a transport: see the
/// [module docs](self). Its parts are for the client that runs on it.
#[non_exhaustive]
pub struct ClientSeat<T: TransportPort> {
    /// The node's started network layer. The seat never stops it.
    #[doc(hidden)]
    pub network: Arc<NetworkLayer<T>>,
    /// The node's one pool of invoke IDs.
    #[doc(hidden)]
    pub coordinator: Arc<OutboundTransactionCoordinator>,
    /// The APDUs that are the client's.
    #[doc(hidden)]
    pub apdus: mpsc::Receiver<ReceivedApdu>,
    /// A copy of every network control the node receives.
    #[doc(hidden)]
    pub controls: mpsc::Receiver<ReceivedNetworkControl>,
    /// The other roles on the node.
    #[doc(hidden)]
    pub roles: SeatRoles,
    /// Fires when the node's Number owner publishes the network number.
    #[doc(hidden)]
    pub number: SeatNumber,
    /// The line back to the endpoint, for each task that keeps the layer.
    #[doc(hidden)]
    pub holds: SeatHolds,
}

impl<T: TransportPort> std::fmt::Debug for ClientSeat<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientSeat")
            .field("roles", &self.roles)
            .finish_non_exhaustive()
    }
}

/// What ingress did for the client seat since the endpoint started. Each
/// count saturates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClientSeatCounters {
    /// APDUs handed to the seat.
    pub apdus: u64,
    /// APDUs dropped because the seat's queue was full: for the copy of an
    /// Unconfirmed-Request, full down to the quarter kept for answers.
    pub full_drops: u64,
    /// Network controls dropped because the seat's control queue was full.
    pub control_full_drops: u64,
}

/// The senders of one attached seat.
pub(crate) struct Attached {
    pub(crate) apdus: mpsc::Sender<ReceivedApdu>,
    pub(crate) controls: mpsc::Sender<ReceivedNetworkControl>,
    pub(crate) coordinator: Arc<OutboundTransactionCoordinator>,
    pub(crate) roles: SeatRoles,
    client: Arc<Mutex<SeatedClient>>,
}

impl Attached {
    fn client(&self) -> std::sync::MutexGuard<'_, SeatedClient> {
        self.client.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the seat still has, or may still get, a client: its queue is
    /// open, and the client that started on it has not been dropped.
    fn occupied(&self) -> bool {
        let dropped = self
            .client()
            .alive
            .as_ref()
            .is_some_and(|alive| alive.strong_count() == 0);
        !self.apdus.is_closed() && !dropped
    }
}

struct State {
    attached: Option<Attached>,
    /// Mints each seat's [`SeatHold`]; gone once the endpoint is stopping.
    holds: Option<mpsc::Sender<()>>,
}

/// The endpoint's one place for a client seat, shared by its owner, which
/// attaches seats, and its ingress task, which routes to the one attached.
pub(crate) struct SeatSlot {
    state: Mutex<State>,
    /// Whether a seat is attached, so an endpoint with none takes no lock
    /// for an APDU or a control. A seat counts from the next APDU after it
    /// attaches.
    attached: AtomicBool,
    number: SeatNumber,
    apdus: AtomicU64,
    full_drops: AtomicU64,
    control_full_drops: AtomicU64,
}

fn add(counter: &AtomicU64) {
    let _ = counter.try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
        count.checked_add(1)
    });
}

impl SeatSlot {
    /// An empty slot, and the receiver that ends when every hold is gone.
    pub(crate) fn new() -> (Arc<Self>, mpsc::Receiver<()>) {
        let (holds, held) = mpsc::channel(1);
        let slot = Arc::new(Self {
            state: Mutex::new(State {
                attached: None,
                holds: Some(holds),
            }),
            attached: AtomicBool::new(false),
            number: SeatNumber::default(),
            apdus: AtomicU64::new(0),
            full_drops: AtomicU64::new(0),
            control_full_drops: AtomicU64::new(0),
        });
        (slot, held)
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn number(&self) -> &SeatNumber {
        &self.number
    }

    pub(crate) fn counters(&self) -> ClientSeatCounters {
        ClientSeatCounters {
            apdus: self.apdus.load(Ordering::Relaxed),
            full_drops: self.full_drops.load(Ordering::Relaxed),
            control_full_drops: self.control_full_drops.load(Ordering::Relaxed),
        }
    }

    /// Make the seat on `network`. One client at a time: a second seat is
    /// refused until the first one's client has dropped its queues.
    pub(crate) fn attach<T: TransportPort>(
        &self,
        network: Arc<NetworkLayer<T>>,
        coordinator: Arc<OutboundTransactionCoordinator>,
        roles: SeatRoles,
        capacity: usize,
    ) -> Result<ClientSeat<T>, Error> {
        if capacity == 0 {
            return Err(Error::Encoding(
                "client seat queue capacity must be greater than zero".into(),
            ));
        }
        let mut state = self.state();
        let Some(holds) = state.holds.as_ref().map(mpsc::Sender::downgrade) else {
            return Err(Error::Encoding("endpoint shutdown".into()));
        };
        if state.attached.as_ref().is_some_and(Attached::occupied) {
            return Err(Error::Encoding(
                "the endpoint already has a full client: drop it before making another".into(),
            ));
        }
        let (apdu_tx, apdus) = mpsc::channel(capacity);
        let (control_tx, controls) = mpsc::channel(CLIENT_SEAT_CONTROL_QUEUE);
        let client = Arc::new(Mutex::new(SeatedClient::default()));
        state.attached = Some(Attached {
            apdus: apdu_tx,
            controls: control_tx,
            coordinator: Arc::clone(&coordinator),
            roles,
            client: Arc::clone(&client),
        });
        self.attached.store(true, Ordering::Relaxed);
        Ok(ClientSeat {
            network,
            coordinator,
            apdus,
            controls,
            roles,
            number: self.number.clone(),
            holds: SeatHolds { holds, client },
        })
    }

    /// Close the seat's queues, abort its client's tasks and make no more
    /// seats: the endpoint is stopping.
    pub(crate) fn close(&self) {
        let mut state = self.state();
        self.attached.store(false, Ordering::Relaxed);
        if let Some(attached) = state.attached.take() {
            for task in attached.client().tasks.drain(..) {
                task.abort();
            }
        }
        state.holds = None;
    }

    /// Run `route` with the attached seat, if a live one is attached.
    pub(crate) fn with<R>(&self, route: impl FnOnce(&Attached) -> R) -> Option<R> {
        if !self.attached.load(Ordering::Relaxed) {
            return None;
        }
        let mut state = self.state();
        if state
            .attached
            .as_ref()
            .is_some_and(|attached| attached.apdus.is_closed())
        {
            state.attached = None;
            self.attached.store(false, Ordering::Relaxed);
        }
        state.attached.as_ref().map(route)
    }

    /// Whether the seat takes a copy of `received` now. The copy of an
    /// Unconfirmed-Request may fill the seat's queue only down to the
    /// reserve kept for answers and confirmed requests (#2635): past that it
    /// is dropped and counted here, before it is made.
    pub(crate) fn takes_copy(&self, attached: &Attached, received: &ReceivedApdu) -> bool {
        let unconfirmed = received.apdu.first().is_some_and(|first| first >> 4 == 1);
        if unconfirmed && crate::endpoint_ingress::unconfirmed_share_full(&attached.apdus) {
            add(&self.full_drops);
            return false;
        }
        true
    }

    /// Hand `received` to the seat without waiting. A full queue drops and
    /// counts it. It comes back only when the seat's client is gone.
    pub(crate) fn offer(
        &self,
        attached: &Attached,
        received: ReceivedApdu,
    ) -> Option<ReceivedApdu> {
        match attached.apdus.try_send(received) {
            Ok(()) => {
                add(&self.apdus);
                None
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                add(&self.full_drops);
                None
            }
            Err(mpsc::error::TrySendError::Closed(received)) => Some(received),
        }
    }

    /// Hand a copy of `control` to the seat, if one is attached, without
    /// waiting.
    pub(crate) fn offer_control(&self, control: &ReceivedNetworkControl) {
        self.with(|attached| {
            if let Err(mpsc::error::TrySendError::Full(_)) =
                attached.controls.try_send(control.clone())
            {
                add(&self.control_full_drops);
            }
        });
    }
}
