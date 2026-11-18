//! The seats: what a full client and a full server are handed to run on an
//! endpoint's link beside the endpoint's own roles (#2617).
//!
//! An endpoint owns one transport, one [`NetworkLayer`] and one
//! [`OutboundTransactionCoordinator`]. A [`ClientSeat`] or a [`ServerSeat`]
//! shares the last two with a role that would otherwise make its own, and
//! gives it the APDUs (and, to the client, the network controls) that are
//! its to take. Nothing is encoded again and no invoke ID is translated:
//! each role sends on the shared layer itself, and leases its invoke IDs
//! from the shared coordinator, so they are unique across the node. Both
//! seats of one endpoint share one coordinator.
//!
//! # Who receives what
//!
//! Endpoint ingress decides from an APDU's first octets, after its own
//! validity checks. The server seat, while it is attached, takes the place
//! of the endpoint's own request queue: what that queue would take goes to
//! the full server instead, and goes back to that queue, where the endpoint
//! counts and drops it, only once the full server is gone.
//!
//! - an Unconfirmed-Request goes to the server role (the server seat, or
//!   the endpoint's request queue), and a copy with no reply authority to
//!   the client seat;
//! - a Confirmed-Request goes to the server role. A ConfirmedCOVNotification
//!   or ConfirmedCOVNotificationMultiple goes to the client seat instead,
//!   with its reply authority. A
//!   ConfirmedEventNotification goes to the server role, and a copy with no
//!   reply authority to the client seat. On an endpoint with no server role
//!   every Confirmed-Request goes to the client seat;
//! - an answer (Simple-ACK, Complex-ACK and its segments, Error, Reject, and
//!   an Abort or Segment-ACK sent by the answering side) goes to the seat of
//!   the role [`OutboundTransactionCoordinator::holder_of_source`] names for
//!   its source and invoke ID: the client seat for a full client's lease,
//!   the server seat for a server's (any [`LeaseOwner::Notification`]
//!   lease, so the node must leave the server seat the only role that makes
//!   them). A segmented Complex-ACK, or a Segment-ACK sent by the answering
//!   side, that no lease holds goes to the client seat when one is attached:
//!   the client in its idle state aborts either (Clause 5.4.4.1). Any other
//!   goes to the endpoint's answer queue, where an answer no lease claims is
//!   counted and dropped;
//! - an Abort or Segment-ACK sent by a requester is about a request the
//!   node serves: it goes to the server seat, or to the endpoint's answer
//!   queue when there is none;
//! - every network control goes to the client seat, beside the endpoint's
//!   own Number owner. The server seat gets none: the node's one Number
//!   owner answers for the network number.
//!
//! [`LeaseOwner::Notification`]: crate::coordinator::LeaseOwner::Notification
//!
//! Ingress never waits for a seat. An APDU or control that finds a seat's
//! queue full is dropped and counted ([`ClientSeatCounters`],
//! [`ServerSeatCounters`]), so a role that stops reading holds nothing else
//! up, the other role included.
//!
//! Each seat's queue has three tiers (#2635), so what the role can best
//! spare finds it full first:
//!
//! - Unconfirmed-Requests (I-Ams, Who-Is, notifications), and on the client
//!   seat the segmented answers no lease holds, may fill it only down to
//!   half of it;
//! - Confirmed-Requests may fill it down to its last quarter, the reserve
//!   each endpoint queue keeps;
//! - answers to the role's own requests, and a requester's Abort or
//!   Segment-ACK, may fill all of it.
//!
//! A burst of I-Ams from the node's own discovery then shuts out neither
//! role's confirmed requests nor its answers. Each tier keeps a slot of its
//! own where the capacity allows: one slot for all at a capacity of 1;
//! unconfirmed one and answers one at 2; one each at 3; 2, 1 and 1 at 4;
//! and at the client seat's 256, 128, 64 and 64.
//!
//! The server seat's queue is as deep as the endpoint's request queue,
//! whose place it takes (the endpoint's queue capacity), and its tiers
//! scale with it. At a capacity of 2 its confirmed requests share the
//! answers' slot instead, so a confirmed request behind an unconfirmed one
//! still finds room, as it does in the request queue (#2635).
//!
//! # One device
//!
//! The node is one device, so what DeviceCommunicationControl does binds
//! both roles (Clause 16.1): the seated server sets the node's state
//! ([`NodeDcc`]) as it accepts a request, its duration runs out, or a
//! restart enables it again, and every seated client follows it
//! ([`InitiationGate`]), unless its operator started it outside it. The
//! server's seat also carries the node's own device settings
//! ([`NodeDevice`]), so the Device it serves states what the node does.
//!
//! # Lifetime
//!
//! The endpoint stops the link. Its stop closes the seats' queues and aborts
//! the tasks a role names when it starts ([`SeatHolds::abort_on_stop`]),
//! then waits for each task's [`SeatHold`] before it stops the layer, so
//! none of them is still at work: a task in the middle of a turn can't hold
//! the stop up. A role that is still alive keeps its own reference to the
//! layer, and the layer stops all the same, through that shared reference
//! (#2617): the role's later sends fail with the transport's stopped error,
//! and its drop changes nothing. A full server names no task and takes no
//! hold: it has a graceful stop of its own, best run before the endpoint's.

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
    /// what is its own (a confirmed COV notification, of either form): it
    /// rejects no
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

/// What the endpoint knows of the role on a seat.
#[derive(Default)]
struct SeatedRole {
    /// The role's tasks, aborted when the endpoint stops.
    tasks: Vec<tokio::task::AbortHandle>,
    /// Dead once the role itself is dropped.
    alive: Option<Weak<dyn Any + Send + Sync>>,
}

/// A seat's line back to its endpoint: the [`SeatHold`]s of the role's
/// tasks, the tasks themselves, and whether the role still lives. It is
/// not a hold itself, so a seat nobody started a role on doesn't keep the
/// endpoint's stop waiting.
#[doc(hidden)]
#[derive(Clone)]
pub struct SeatHolds {
    holds: mpsc::WeakSender<()>,
    role: Arc<Mutex<SeatedRole>>,
}

impl SeatHolds {
    fn role(&self) -> std::sync::MutexGuard<'_, SeatedRole> {
        self.role.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A hold for one task, or `None` once the endpoint is stopping.
    pub fn hold(&self) -> Option<SeatHold> {
        self.holds.upgrade().map(SeatHold)
    }

    /// Have the endpoint's stop abort `task`, which keeps the layer: the
    /// stop then never waits on that task's own progress.
    pub fn abort_on_stop(&self, task: tokio::task::AbortHandle) {
        self.role().tasks.push(task);
    }

    /// The role lives while `role` can be upgraded. Once it can't, the
    /// endpoint takes another role on this seat at once, without waiting
    /// for the dropped one's tasks to finish ending.
    pub fn alive_while(&self, role: Weak<dyn Any + Send + Sync>) {
        self.role().alive = Some(role);
    }
}

/// Tells a seated role that the node's Number owner may have changed the
/// layer's local network number. A seated role runs no Number owner of its
/// own: the client folds its discovery table when this fires, and a server
/// with target Audit routes its recipient again. Each seat waits on a
/// notice of its own, so one publication reaches both.
#[doc(hidden)]
#[derive(Clone, Debug, Default)]
pub struct SeatNumber {
    seats: Arc<[Notify; 2]>,
    /// Which seat's notice [`Self::changed`] waits on.
    seat: usize,
}

impl SeatNumber {
    /// This one's handle for `role`'s seat.
    pub(crate) fn for_seat(&self, role: Role) -> Self {
        Self {
            seats: Arc::clone(&self.seats),
            seat: role as usize,
        }
    }

    /// The node's Number owner published the number.
    pub fn published(&self) {
        for seat in self.seats.iter() {
            seat.notify_one();
        }
    }

    /// Wait for the next publication. One made while nothing waited is kept.
    pub async fn changed(&self) {
        self.seats[self.seat].notified().await;
    }
}

/// A seated client's part in the node's DeviceCommunicationControl
/// (Clause 16.1): told when the node's server restricts initiation, and
/// when it enables it again.
#[doc(hidden)]
pub trait InitiationGate: Send + Sync {
    /// Initiation is now restricted (`true`) or enabled again (`false`).
    fn restrict(&self, restricted: bool);
}

#[derive(Default)]
struct NodeDccState {
    restricted: bool,
    /// The server seat whose state this is (see [`ServerDcc`]); 0 for none.
    owner: u64,
    last_owner: u64,
    gates: Vec<Weak<dyn InitiationGate>>,
    /// Clients started outside the node's state, by their operator's
    /// choice: warned about, never restricted.
    exempt: Vec<Weak<dyn Any + Send + Sync>>,
}

impl NodeDccState {
    fn prune(&mut self) {
        self.gates.retain(|gate| gate.strong_count() > 0);
        self.exempt.retain(|client| client.strong_count() > 0);
    }

    fn warn_exempt(&self) {
        if self.restricted && !self.exempt.is_empty() {
            tracing::warn!(
                exempt_clients = self.exempt.len(),
                "DeviceCommunicationControl disabled initiation on this node, but a full client \
                 started outside the node's DeviceCommunicationControl keeps initiating \
                 (an operator override of Clause 16.1)"
            );
        }
    }

    fn apply(&mut self, restricted: bool) {
        self.prune();
        if self.restricted == restricted {
            return;
        }
        self.restricted = restricted;
        for gate in self.gates.iter().filter_map(Weak::upgrade) {
            gate.restrict(restricted);
        }
        self.warn_exempt();
    }
}

/// The node's DeviceCommunicationControl state (Clause 16.1), which its
/// seated server sets and every seated client follows: the standard binds
/// the device, and the node is one device. Changes are applied to each
/// client under one lock, before the server answers the request that made
/// them.
#[doc(hidden)]
#[derive(Clone, Default)]
pub struct NodeDcc(Arc<Mutex<NodeDccState>>);

impl std::fmt::Debug for NodeDcc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeDcc")
            .field("restricted", &self.is_restricted())
            .finish_non_exhaustive()
    }
}

impl NodeDcc {
    fn state(&self) -> std::sync::MutexGuard<'_, NodeDccState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the node's server has restricted initiation now.
    pub fn is_restricted(&self) -> bool {
        self.state().restricted
    }

    /// Have `gate` follow the node's state from now on, starting from the
    /// state in force. It is let go once it can't be upgraded.
    pub fn follow(&self, gate: Weak<dyn InitiationGate>) {
        let mut state = self.state();
        state.prune();
        if state.restricted
            && let Some(gate) = gate.upgrade()
        {
            gate.restrict(true);
        }
        state.gates.push(gate);
    }

    /// A client that its operator started outside the node's state, alive
    /// while `client` can be upgraded: it is never restricted, and a
    /// warning is logged when DeviceCommunicationControl takes effect while
    /// it is attached, or when it attaches while it is in effect.
    pub fn exempt(&self, client: Weak<dyn Any + Send + Sync>) {
        let mut state = self.state();
        state.prune();
        state.exempt.push(client);
        state.warn_exempt();
    }

    /// The handle a newly seated server sets the state through. The state
    /// is the newest server's: one that was seated before can no longer
    /// change it.
    fn claim(&self) -> ServerDcc {
        let mut state = self.state();
        state.last_owner += 1;
        state.owner = state.last_owner;
        let owner = state.owner;
        state.apply(false);
        ServerDcc {
            node: self.clone(),
            owner,
        }
    }
}

/// A seated server's hold on its node's DeviceCommunicationControl state.
/// Released when it drops, or by [`Self::release`]: the node then enables
/// initiation again, as a restarted device would.
#[doc(hidden)]
pub struct ServerDcc {
    node: NodeDcc,
    owner: u64,
}

impl std::fmt::Debug for ServerDcc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerDcc").finish_non_exhaustive()
    }
}

impl ServerDcc {
    /// The server restricted initiation (`true`) or enabled it (`false`).
    /// Does nothing once another server holds the node's state.
    pub fn set(&self, restricted: bool) {
        let mut state = self.node.state();
        if state.owner == self.owner {
            state.apply(restricted);
        }
    }

    /// The server is going: the node enables initiation again, unless
    /// another server holds its state by now.
    pub fn release(&self) {
        let mut state = self.node.state();
        if state.owner == self.owner {
            state.apply(false);
            state.owner = 0;
        }
    }
}

impl Drop for ServerDcc {
    fn drop(&mut self) {
        self.release();
    }
}

/// The node's own device settings, which a full server on its seat follows
/// so that the Device it serves states what the whole node does (#2617):
/// whether the node has a client role, the largest APDU that role accepts,
/// and the one APDU timeout and retry count both roles use.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeDevice {
    /// The node has a client role, whose executed services the Device
    /// lists beside the server's.
    pub client: bool,
    /// The largest APDU the node's client role accepts.
    pub client_max_apdu_length: u16,
    /// The node's APDU_Timeout, in milliseconds.
    pub apdu_timeout_ms: u64,
    /// The node's Number_Of_APDU_Retries.
    pub apdu_retries: u8,
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
    /// The node's DeviceCommunicationControl state, which the client
    /// follows unless its operator started it outside it.
    #[doc(hidden)]
    pub dcc: NodeDcc,
}

impl<T: TransportPort> std::fmt::Debug for ClientSeat<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientSeat")
            .field("roles", &self.roles)
            .finish_non_exhaustive()
    }
}

/// What a full server is handed in place of a transport: see the
/// [module docs](self). Its parts are for the server that runs on it.
#[non_exhaustive]
pub struct ServerSeat<T: TransportPort> {
    /// The node's started network layer. The seat never stops it.
    #[doc(hidden)]
    pub network: Arc<NetworkLayer<T>>,
    /// The node's one pool of invoke IDs.
    #[doc(hidden)]
    pub coordinator: Arc<OutboundTransactionCoordinator>,
    /// The APDUs that are the server's.
    #[doc(hidden)]
    pub apdus: mpsc::Receiver<ReceivedApdu>,
    /// The line back to the endpoint.
    #[doc(hidden)]
    pub holds: SeatHolds,
    /// Fires when the node's Number owner publishes the network number.
    #[doc(hidden)]
    pub number: SeatNumber,
    /// The server's hold on the node's DeviceCommunicationControl state.
    #[doc(hidden)]
    pub dcc: ServerDcc,
    /// The node's device settings, when the node has them: the endpoint's
    /// session gives them; a seat made from the ingress alone has none,
    /// and its server serves its own Device.
    #[doc(hidden)]
    pub node: Option<NodeDevice>,
}

impl<T: TransportPort> std::fmt::Debug for ServerSeat<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerSeat").finish_non_exhaustive()
    }
}

/// What ingress did for the client seat since the endpoint started. Each
/// count saturates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClientSeatCounters {
    /// APDUs handed to the seat.
    pub apdus: u64,
    /// APDUs the seat refused: those its full queue had no room for, and
    /// those its tiers held back for what the client can spare less (see
    /// the [module docs](self)). On an endpoint with no server role, a
    /// Confirmed-Request the seat refuses is counted here, not in the
    /// endpoint's own counts.
    pub full_drops: u64,
    /// Network controls dropped because the seat's control queue was full.
    pub control_full_drops: u64,
}

/// What ingress did for the server seat since the endpoint started. Each
/// count saturates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServerSeatCounters {
    /// APDUs handed to the seat. Counted as the seat's queue takes them:
    /// what a server finds still queued when it stops, or takes after it
    /// began stopping, it drops unanswered, and that is counted here, not
    /// in [`Self::full_drops`].
    pub apdus: u64,
    /// APDUs the seat refused: those its full queue had no room for, and
    /// those its tiers held back for what the server can spare less (see
    /// the [module docs](self)). A Confirmed-Request counted here got no
    /// answer: its requester's retry is its recovery. The endpoint's own
    /// counts never count what the server seat refused.
    pub full_drops: u64,
}

/// Which of the endpoint's two seats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Client = 0,
    Server = 1,
}

impl Role {
    fn name(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
        }
    }
}

/// The senders of one attached seat.
pub(crate) struct Attached {
    pub(crate) apdus: mpsc::Sender<ReceivedApdu>,
    /// The client's copy of each network control; the server seat has none.
    controls: Option<mpsc::Sender<ReceivedNetworkControl>>,
    pub(crate) coordinator: Arc<OutboundTransactionCoordinator>,
    pub(crate) roles: SeatRoles,
    role: Role,
    seated: Arc<Mutex<SeatedRole>>,
}

impl Attached {
    fn seated(&self) -> std::sync::MutexGuard<'_, SeatedRole> {
        self.seated.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the seat still has, or may still get, a role: its queue is
    /// open, and the role that started on it has not been dropped.
    fn occupied(&self) -> bool {
        let dropped = self
            .seated()
            .alive
            .as_ref()
            .is_some_and(|alive| alive.strong_count() == 0);
        !self.apdus.is_closed() && !dropped
    }
}

struct State {
    /// By [`Role`].
    seats: [Option<Attached>; 2],
    /// Mints each seat's [`SeatHold`]; gone once the endpoint is stopping.
    holds: Option<mpsc::Sender<()>>,
}

/// One seat's counts.
#[derive(Default)]
struct Counts {
    apdus: AtomicU64,
    full_drops: AtomicU64,
}

/// The endpoint's places for a client seat and a server seat, shared by its
/// owner, which attaches seats, and its ingress task, which routes to those
/// attached.
pub(crate) struct SeatSlot {
    state: Mutex<State>,
    /// Whether any seat is attached, so an endpoint with none takes no lock
    /// for an APDU or a control. A seat counts from the next APDU after it
    /// attaches.
    attached: AtomicBool,
    number: SeatNumber,
    dcc: NodeDcc,
    /// By [`Role`].
    counts: [Counts; 2],
    control_full_drops: AtomicU64,
}

/// The free slots of a seat of `capacity` that Unconfirmed-Requests leave:
/// twice the [`reserve`](crate::endpoint_ingress::reserve), so they fill at
/// most half of a seat of 8 or more, but never all of it, so they keep a
/// slot too.
pub(crate) fn copy_floor(capacity: usize) -> usize {
    (2 * crate::endpoint_ingress::reserve(capacity)).min(capacity.saturating_sub(1))
}

fn add(counter: &AtomicU64) {
    let _ = counter.try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
        count.checked_add(1)
    });
}

/// The parts of one seat [`SeatSlot::attach`] made.
struct Made {
    apdus: mpsc::Receiver<ReceivedApdu>,
    controls: Option<mpsc::Receiver<ReceivedNetworkControl>>,
    holds: SeatHolds,
}

impl SeatSlot {
    /// An empty slot, and the receiver that ends when every hold is gone.
    pub(crate) fn new() -> (Arc<Self>, mpsc::Receiver<()>) {
        let (holds, held) = mpsc::channel(1);
        let slot = Arc::new(Self {
            state: Mutex::new(State {
                seats: [None, None],
                holds: Some(holds),
            }),
            attached: AtomicBool::new(false),
            number: SeatNumber::default(),
            dcc: NodeDcc::default(),
            counts: Default::default(),
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

    pub(crate) fn dcc(&self) -> &NodeDcc {
        &self.dcc
    }

    pub(crate) fn counters(&self) -> ClientSeatCounters {
        let counts = &self.counts[Role::Client as usize];
        ClientSeatCounters {
            apdus: counts.apdus.load(Ordering::Relaxed),
            full_drops: counts.full_drops.load(Ordering::Relaxed),
            control_full_drops: self.control_full_drops.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn server_counters(&self) -> ServerSeatCounters {
        let counts = &self.counts[Role::Server as usize];
        ServerSeatCounters {
            apdus: counts.apdus.load(Ordering::Relaxed),
            full_drops: counts.full_drops.load(Ordering::Relaxed),
        }
    }

    /// Make the client's seat on `network`. One client at a time: a second
    /// seat is refused until the first one's client has dropped its queues.
    pub(crate) fn attach<T: TransportPort>(
        &self,
        network: Arc<NetworkLayer<T>>,
        coordinator: Arc<OutboundTransactionCoordinator>,
        roles: SeatRoles,
        capacity: usize,
    ) -> Result<ClientSeat<T>, Error> {
        let made = self.make(Role::Client, &coordinator, roles, capacity)?;
        Ok(ClientSeat {
            network,
            coordinator,
            apdus: made.apdus,
            controls: made.controls.expect("a client seat's controls"),
            roles,
            number: self.number.for_seat(Role::Client),
            holds: made.holds,
            dcc: self.dcc.clone(),
        })
    }

    /// Make the server's seat on `network`. One server at a time, as for
    /// the client.
    pub(crate) fn attach_server<T: TransportPort>(
        &self,
        network: Arc<NetworkLayer<T>>,
        coordinator: Arc<OutboundTransactionCoordinator>,
        capacity: usize,
    ) -> Result<ServerSeat<T>, Error> {
        let made = self.make(Role::Server, &coordinator, SeatRoles::default(), capacity)?;
        Ok(ServerSeat {
            network,
            coordinator,
            apdus: made.apdus,
            holds: made.holds,
            number: self.number.for_seat(Role::Server),
            dcc: self.dcc.claim(),
            node: None,
        })
    }

    fn make(
        &self,
        role: Role,
        coordinator: &Arc<OutboundTransactionCoordinator>,
        roles: SeatRoles,
        capacity: usize,
    ) -> Result<Made, Error> {
        if capacity == 0 {
            return Err(Error::Encoding(format!(
                "{} seat queue capacity must be greater than zero",
                role.name()
            )));
        }
        let mut state = self.state();
        let Some(holds) = state.holds.as_ref().map(mpsc::Sender::downgrade) else {
            return Err(Error::Encoding("endpoint shutdown".into()));
        };
        if state.seats[role as usize]
            .as_ref()
            .is_some_and(Attached::occupied)
        {
            return Err(Error::Encoding(format!(
                "the endpoint already has a full {}: drop it before making another",
                role.name()
            )));
        }
        // Answers go to a seat by the lease its coordinator names, so every
        // role on one node leases from the same pool.
        let other = &state.seats[1 - role as usize];
        if other
            .as_ref()
            .is_some_and(|other| other.occupied() && !Arc::ptr_eq(&other.coordinator, coordinator))
        {
            return Err(Error::Encoding(
                "every seat on one endpoint shares its pool of invoke IDs".into(),
            ));
        }
        let (apdu_tx, apdus) = mpsc::channel(capacity);
        let (control_tx, controls) = match role {
            Role::Client => {
                let (tx, rx) = mpsc::channel(CLIENT_SEAT_CONTROL_QUEUE);
                (Some(tx), Some(rx))
            }
            Role::Server => (None, None),
        };
        let seated = Arc::new(Mutex::new(SeatedRole::default()));
        state.seats[role as usize] = Some(Attached {
            apdus: apdu_tx,
            controls: control_tx,
            coordinator: Arc::clone(coordinator),
            roles,
            role,
            seated: Arc::clone(&seated),
        });
        self.attached.store(true, Ordering::Relaxed);
        Ok(Made {
            apdus,
            controls,
            holds: SeatHolds {
                holds,
                role: seated,
            },
        })
    }

    /// Close the seats' queues, abort their roles' tasks and make no more
    /// seats: the endpoint is stopping.
    pub(crate) fn close(&self) {
        let mut state = self.state();
        self.attached.store(false, Ordering::Relaxed);
        for attached in state.seats.iter_mut().filter_map(Option::take) {
            for task in attached.seated().tasks.drain(..) {
                task.abort();
            }
        }
        state.holds = None;
    }

    /// Run `route` with the attached client and server seats, if any seat
    /// is attached. A seat whose queue has closed (its role's reader is
    /// gone) is let go of first, so `route` doesn't see it; one whose role
    /// was dropped but whose queue is still open is still offered to, as
    /// its reader may yet drain it.
    pub(crate) fn with<R>(
        &self,
        route: impl FnOnce(Option<&Attached>, Option<&Attached>) -> R,
    ) -> Option<R> {
        if !self.attached.load(Ordering::Relaxed) {
            return None;
        }
        let mut state = self.state();
        for seat in &mut state.seats {
            if seat
                .as_ref()
                .is_some_and(|attached| attached.apdus.is_closed())
            {
                *seat = None;
            }
        }
        if state.seats.iter().all(Option::is_none) {
            self.attached.store(false, Ordering::Relaxed);
        }
        let [client, server] = &state.seats;
        Some(route(client.as_ref(), server.as_ref()))
    }

    /// Whether the seat takes `received`, or a copy of it, now: whether more
    /// than its tier's floor of free slots is left (see the
    /// [module docs](self)). One that isn't is dropped and counted here,
    /// before any copy is made. An answer is offered as it is.
    pub(crate) fn takes(&self, attached: &Attached, received: &ReceivedApdu) -> bool {
        let capacity = attached.apdus.max_capacity();
        let floor = match received.apdu.first().map(|first| first >> 4) {
            Some(1) => copy_floor(capacity),
            // Too small for three tiers: the server seat's confirmed
            // requests keep the request queue's rule (#2635) and may take
            // the last slot.
            Some(0) if attached.role == Role::Server && capacity < 3 => 0,
            Some(0) => crate::endpoint_ingress::reserve(capacity),
            _ => return true,
        };
        if attached.apdus.capacity() <= floor {
            add(&self.counts[attached.role as usize].full_drops);
            return false;
        }
        true
    }

    /// Whether the client seat takes an answer to nothing the node asked (see
    /// the [module docs](self)): only while more than a copy's floor of free
    /// slots is left, so such answers never take the room the client's own
    /// answers have. One that isn't is dropped and counted here.
    pub(crate) fn takes_stray(&self, attached: &Attached) -> bool {
        let floor = copy_floor(attached.apdus.max_capacity());
        if attached.apdus.capacity() <= floor {
            add(&self.counts[attached.role as usize].full_drops);
            return false;
        }
        true
    }

    /// Hand `received` to the seat without waiting. A full queue drops and
    /// counts it. It comes back only when the seat's role is gone.
    pub(crate) fn offer(
        &self,
        attached: &Attached,
        received: ReceivedApdu,
    ) -> Option<ReceivedApdu> {
        let counts = &self.counts[attached.role as usize];
        match attached.apdus.try_send(received) {
            Ok(()) => {
                add(&counts.apdus);
                None
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                add(&counts.full_drops);
                None
            }
            Err(mpsc::error::TrySendError::Closed(received)) => Some(received),
        }
    }

    /// Hand a copy of `control` to the client seat, if one is attached,
    /// without waiting.
    pub(crate) fn offer_control(&self, control: &ReceivedNetworkControl) {
        self.with(|client, _| {
            if let Some(controls) = client.and_then(|client| client.controls.as_ref())
                && let Err(mpsc::error::TrySendError::Full(_)) = controls.try_send(control.clone())
            {
                add(&self.control_full_drops);
            }
        });
    }
}
