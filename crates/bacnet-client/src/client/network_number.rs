//! One bounded serial owner; raw Reject/APDU dispatch never awaits Number egress.
//!
//! The client asks for its network's number on first need, never at start
//! (#2572): before its first Who-Is, whose answers are about to fill the
//! discovery table, and when it first stores a routed row while the number
//! is unknown, the row that can turn out to be a local device's twin. A
//! client that only names devices by address never asks. After that first
//! ask, each return of the link while the number is still unknown asks once
//! more. Clause 6.4.19 lets any node ask with a local broadcast and has it
//! keep the answer rather than repeat the question, so a known number is
//! never asked for again.
//!
//! The client also answers the question when it holds the number. It is
//! not a router, so the same clause lets it hold its answer to a broadcast
//! question for a random wait and skip it when another node answers first
//! (#2597, [`DelayedAnswer`]); a question sent to the client alone is
//! answered at once.
use super::device_events::{self, TableNotices};
use super::*;
use bacnet_network::network_number::{DelayedAnswer, NumberControl, number_is_reply};
use bacnet_npdu::NetworkMessageType;
use bacnet_types::network_number::NetworkNumber;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use tokio::sync::{Notify, watch};

pub(super) const CAPACITY: usize = 256;

/// [`Asking::sends`]: the client sends.
const OPEN: u8 = 0;
/// [`Asking::sends`]: the node this client is seated on restricts
/// initiation (DeviceCommunicationControl, Clause 16.1): the client
/// initiates no service, but its network-layer messages, which are no
/// service, still go out.
const RESTRICTED: u8 = 1;
/// [`Asking::sends`]: the seat this client ran on has closed: its node
/// stopped or dropped it, or the client was stopped. Nothing more is sent,
/// the question for the network's number included.
const SEALED: u8 = 2;

/// Why [`Asking::sends`] refuses a send.
pub(super) enum Refusal {
    /// DeviceCommunicationControl restricts the node.
    Restricted,
    /// The seat has closed.
    Sealed,
}

/// When the client asks for its network's number: shared by the request
/// methods, which ask before a Who-Is, the dispatch task, which wants an
/// ask when it stores a routed row, and the Number owner, which makes that
/// ask and the ones a link return calls for.
#[derive(Debug, Default)]
pub(super) struct Asking {
    /// The link takes the local Number controls: without them nothing is
    /// asked, and no owner runs.
    numbers: bool,
    /// The client runs on a seat (#2617): the node owns the network layer,
    /// so the client's stop leaves it running. Kept here, beside the other
    /// flag, so neither the client nor this allocation grows for it.
    pub(super) seated: bool,
    /// A question went out. A send that failed doesn't count, so the next
    /// need asks again.
    asked: AtomicBool,
    /// [`OPEN`], [`RESTRICTED`] or [`SEALED`]: whether this client may send
    /// what keeps no transaction (#2617). Never other than open for a client
    /// with a transport of its own.
    sends: AtomicU8,
    /// Held across a send, so two needs at once ask once.
    sending: tokio::sync::Mutex<()>,
    /// Wakes the owner to ask for the dispatch task, which never waits on
    /// Number egress.
    wanted: Notify,
}

impl Asking {
    pub(super) fn new(numbers: bool) -> Self {
        Self {
            numbers,
            ..Self::default()
        }
    }

    /// The client's seat closed: see [`SEALED`].
    pub(super) fn seal(&self) {
        self.sends.store(SEALED, Ordering::Release);
    }

    /// Whether the client's seat has closed.
    pub(super) fn is_sealed(&self) -> bool {
        self.sends.load(Ordering::Relaxed) == SEALED
    }

    /// Follow the node's DeviceCommunicationControl state (#2617). A sealed
    /// client stays sealed.
    pub(super) fn restrict(&self, restricted: bool) {
        let (from, to) = if restricted {
            (OPEN, RESTRICTED)
        } else {
            (RESTRICTED, OPEN)
        };
        let _ = self
            .sends
            .compare_exchange(from, to, Ordering::AcqRel, Ordering::Relaxed);
    }

    /// Whether this client may initiate a service now. One load and a test,
    /// as the seal's check always was; the reason is read only on refusal.
    #[inline]
    pub(super) fn refusal(&self) -> Option<Refusal> {
        match self.sends.load(Ordering::Relaxed) {
            OPEN => None,
            RESTRICTED => Some(Refusal::Restricted),
            _ => Some(Refusal::Sealed),
        }
    }

    /// Wait until the dispatch task wants the number asked for.
    pub(super) async fn wanted(&self) {
        self.wanted.notified().await;
    }

    /// Ask now, unless the link takes no Number controls, the number is
    /// known, or the question went out already. Awaited before a Who-Is,
    /// so the question goes ahead of it; a send that fails is logged and
    /// doesn't hold the Who-Is back.
    pub(super) async fn first_need<T: TransportPort + 'static>(&self, network: &NetworkLayer<T>) {
        let needed = || {
            !self.asked.load(Ordering::Acquire) && network.local_network_number().get().is_none()
        };
        if !self.numbers || self.is_sealed() || !needed() {
            return;
        }
        let _one = self.sending.lock().await;
        if needed() && ask(network).await {
            self.asked.store(true, Ordering::Release);
        }
    }

    /// The dispatch task stored a routed row while the number is unknown:
    /// the owner asks, once, if nothing has. Never waits.
    pub(super) fn want(&self) {
        if self.numbers && !self.asked.load(Ordering::Acquire) {
            self.wanted.notify_one();
        }
    }

    /// A caller's own What-Is-Network-Number went out
    /// ([`BACnetClient::what_is_network_number`]): it counts as the
    /// question, so the next need doesn't ask again.
    pub(super) fn asked_by_caller(&self) {
        self.asked.store(true, Ordering::Release);
    }

    /// The owner's ask for a routed row stored while the number is unknown.
    /// It never waits on the send lock, so the owner's controls keep
    /// moving: a busy lock means a question is going out already, and one
    /// that fails is asked again at the next need.
    pub(super) async fn owner_need<T: TransportPort + 'static>(&self, network: &NetworkLayer<T>) {
        let needed = || {
            !self.asked.load(Ordering::Acquire) && network.local_network_number().get().is_none()
        };
        if self.is_sealed() {
            return;
        }
        let Ok(_one) = self.sending.try_lock() else {
            return;
        };
        if needed() && ask(network).await {
            self.asked.store(true, Ordering::Release);
        }
    }

    /// The link is back: ask again if a question went out before and the
    /// number is still unknown. A link return never starts the asking, and
    /// the owner doesn't wait for a question already going out.
    pub(super) async fn again<T: TransportPort + 'static>(&self, network: &NetworkLayer<T>) {
        let needed =
            || self.asked.load(Ordering::Acquire) && network.local_network_number().get().is_none();
        if self.is_sealed() || !needed() {
            return;
        }
        let Ok(_one) = self.sending.try_lock() else {
            return;
        };
        if needed() {
            ask(network).await;
        }
    }
}

/// The discovery table, and beside it in the same allocation the asking
/// that storing a routed row can call for. The client and its dispatch
/// task hold this one `Arc` where they held the table's alone, so neither
/// grows a field on the request path, and the table sits first, where it
/// always sat; a request's lookups reach it unchanged (#2572).
#[repr(C)]
#[derive(Debug)]
pub(super) struct Discovery {
    pub(super) table: SyncMutex<DeviceTable>,
    pub(super) asking: Asking,
}

impl Discovery {
    pub(super) fn new(numbers: bool, seated: bool) -> Self {
        Self {
            table: SyncMutex::new(DeviceTable::new()),
            asking: Asking {
                seated,
                ..Asking::new(numbers)
            },
        }
    }
}

impl std::ops::Deref for Discovery {
    type Target = SyncMutex<DeviceTable>;

    fn deref(&self) -> &Self::Target {
        &self.table
    }
}

/// What the owner keeps beside the layer: the discovery table its learning
/// folds and the asking it shares with the client, the table's
/// subscribers, and the link's returns, when the transport reports them.
pub(super) struct Owner {
    pub(super) discovery: Arc<Discovery>,
    pub(super) device_tx: broadcast::Sender<DeviceEvent>,
    pub(super) collision_tx: broadcast::Sender<DeviceCollisionEvent>,
    pub(super) link_returns: Option<watch::Receiver<u64>>,
}

/// Ask this network's number with a local broadcast What-Is-Network-Number,
/// which carries no DNET and is never routed (Clause 6.4.19). Whether it
/// went: a send that fails is logged and left for the next need.
async fn ask<T: TransportPort + 'static>(network: &NetworkLayer<T>) -> bool {
    let ask = network.send_network_message(NetworkMessageType::WHAT_IS_NETWORK_NUMBER, &[], None);
    match ask.await {
        Ok(()) => true,
        Err(error) => {
            debug!(%error, "What-Is-Network-Number broadcast failed");
            false
        }
    }
}

/// Broadcast the Network-Number-Is that answers a question, when the
/// number is known. A send that fails is logged: the asker asks again.
async fn announce<T: TransportPort + 'static>(network: &NetworkLayer<T>, state: NetworkNumber) {
    if let Some(npdu) = number_is_reply(state)
        && let Err(error) = network.transport().send_broadcast(&npdu).await
    {
        debug!(%error, "Network-Number-Is broadcast failed");
    }
}

/// Wait for the link to come back; forever once the transport has no more
/// returns to report, or never had any.
pub(super) async fn link_returned(returns: &mut Option<watch::Receiver<u64>>) {
    let back = match returns.as_mut() {
        Some(returns) => returns.changed().await.is_ok(),
        None => false,
    };
    if !back {
        *returns = None;
        std::future::pending::<()>().await;
    }
}

pub(super) fn spawn<T: TransportPort + 'static>(
    network: &Arc<NetworkLayer<T>>,
    owner: Owner,
) -> (mpsc::Sender<NumberControl>, JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel(CAPACITY);
    let network = Arc::clone(network);
    let task = tokio::spawn(async move {
        let Owner {
            discovery,
            device_tx,
            collision_tx,
            mut link_returns,
        } = owner;
        let (table, asking) = (&discovery.table, &discovery.asking);
        let notices: TableNotices<'_> = (&device_tx, &collision_tx);
        let mut state = NetworkNumber::default();
        // The answer to a broadcast What-Is-Network-Number, held for its
        // random wait and dropped when another node answers first
        // (Clause 6.4.19, #2597). It lives in this task alone.
        let mut answer = DelayedAnswer::new();
        // Two routers that disagree flip the number on every announcement:
        // the table follows each flip, the log once an interval.
        let changes = WarnThrottle::default();
        loop {
            // No branch holds a lock, and each arm finishes its work, the
            // send included, before the next turn polls any of them.
            let control = tokio::select! {
                control = rx.recv() => match control {
                    Some(control) => control,
                    None => break,
                },
                () = asking.wanted.notified() => {
                    asking.owner_need(&network).await;
                    continue;
                }
                () = link_returned(&mut link_returns) => {
                    asking.again(&network).await;
                    continue;
                }
                () = answer.due() => {
                    answer.taken();
                    announce(&network, state).await;
                    continue;
                }
            };
            match control {
                NumberControl::NumberIs { number, flag } => {
                    let before = state.snapshot().0;
                    state.observe(number, flag);
                    let after = state.snapshot().0;
                    if after != before {
                        if before != 0 {
                            warn_throttled!(
                                changes,
                                previous = before,
                                number = after,
                                "this network's number changed; rows made local under the \
                                 previous number are dropped"
                            );
                        }
                        let this_network = network.local_network_number();
                        device_events::learned_number(table, this_network, state, notices);
                    }
                    // After the state took the announcement in, so a
                    // number just adopted from it counts as agreeing.
                    answer.heard(state, number, flag);
                }
                NumberControl::WhatIs { broadcast } => {
                    if answer.asked(state, broadcast) {
                        announce(&network, state).await;
                    }
                }
            }
        }
    });
    (tx, task)
}
