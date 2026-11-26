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
//! The number itself comes only from the answers to the client's own
//! questions, when every one names it, and for a network the client
//! reaches through a router only after a router check; an announcement it
//! did not ask for that names another number is a cue to ask (#2826,
//! [`number_rounds`]).
//!
//! The client also answers the question when it holds the number. It is
//! not a router, so the same clause lets it hold its answer to a broadcast
//! question for a random wait and skip it when another node answers first
//! (#2597, [`DelayedAnswer`]); a question sent to the client alone is
//! answered at once.
use super::device_events::{self, TableNotices};
use super::number_check::{Routed, RouterCheck};
use super::*;
use bacnet_network::layer::QueueAdmissionCounters;
use bacnet_network::layer::ReceivedNetworkReport;
use bacnet_network::network_number::{DelayedAnswer, NumberControl, number_is_reply};
use bacnet_npdu::NetworkMessageType;
use bacnet_types::network_number::NetworkNumber;
use number_rounds::{Decision, Heard, Learning, Question};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use tokio::sync::{Notify, watch};
use tokio::time::Instant;

#[path = "number_rounds.rs"]
mod number_rounds;

/// How long the answers to a question count, for a client whose APDU
/// timeout is `apdu_timeout` (#2826): [`number_rounds::window_for`].
pub(super) fn window_for(apdu_timeout: Duration) -> Duration {
    number_rounds::window_for(apdu_timeout)
}

use crate::poll_gate::PollGate;

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
#[derive(Debug)]
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
    /// Held across a send, so two needs at once ask once. A need takes it
    /// only as it is polled (#2759).
    sending: PollGate,
    /// Wakes the owner to ask for the dispatch task, which never waits on
    /// Number egress.
    wanted: Notify,
    /// The client's latest question, which went or is going: the answers
    /// heard within a window after it count (#2826).
    asked_at: SyncMutex<Option<Question>>,
    /// Windows whose answers named more than one number (#2826).
    disagreements: AtomicU64,
    /// Controls the Number owner's queue turned away, full (#2826).
    turned_away: AtomicU64,
    /// The network layer's control queue's counts, where this client can
    /// read them: `None` on a seat.
    control_drops: Option<QueueAdmissionCounters>,
}

impl Asking {
    pub(super) fn new(numbers: bool, control_drops: Option<QueueAdmissionCounters>) -> Self {
        Self {
            numbers,
            seated: false,
            asked: AtomicBool::new(false),
            sends: AtomicU8::new(OPEN),
            sending: PollGate::default(),
            wanted: Notify::new(),
            asked_at: SyncMutex::new(None),
            disagreements: AtomicU64::new(0),
            turned_away: AtomicU64::new(0),
            control_drops,
        }
    }

    /// Send a question, noting first when it goes and how many controls
    /// were lost by then, so the answers that come in the window after it
    /// count (#2826), and give the send's own result. A send that fails, or
    /// is cut off before it finishes, puts the note back as it was.
    pub(super) async fn noting<R>(
        &self,
        send: impl Future<Output = Result<R, Error>>,
    ) -> Result<R, Error> {
        let question = Question {
            at: Instant::now(),
            losses: self.losses(),
        };
        let before = self.asked_at.lock().replace(question);
        let mut note = Note {
            asking: self,
            question,
            before,
            went: false,
        };
        let sent = send.await;
        note.went = sent.is_ok();
        sent
    }

    /// The client's latest question.
    fn asked_at(&self) -> Option<Question> {
        *self.asked_at.lock()
    }

    /// Controls lost so far, to the network layer's control queue or the
    /// Number owner's own (#2826). What is lost before them, in the
    /// socket's buffer or on the link, isn't counted.
    pub(super) fn losses(&self) -> u64 {
        let layer = self.control_drops.as_ref().map_or(0, |drops| {
            let counts = drops.snapshot();
            counts.full_drops.saturating_add(counts.closed_drops)
        });
        layer.saturating_add(self.turned_away.load(Ordering::Relaxed))
    }

    /// The Number owner's queue turned a control away, full.
    pub(super) fn turned_away(&self) {
        self.turned_away.fetch_add(1, Ordering::Relaxed);
    }

    /// [`BACnetClient::network_number_disagreements`].
    pub(super) fn disagreements(&self) -> u64 {
        self.disagreements.load(Ordering::Relaxed)
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
        if needed() && self.ask(network).await {
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
        let Some(_one) = self.sending.try_lock() else {
            return;
        };
        if needed() && self.ask(network).await {
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
        let Some(_one) = self.sending.try_lock() else {
            return;
        };
        if needed() {
            self.ask(network).await;
        }
    }

    /// An announcement the client did not ask for named another number than
    /// it holds (#2826): ask, whether the number is known or not, unless a
    /// question is going out already. Never waits on the send lock.
    /// Whether a question went.
    async fn cue<T: TransportPort + 'static>(&self, network: &NetworkLayer<T>) -> bool {
        if self.is_sealed() {
            return false;
        }
        let Some(_one) = self.sending.try_lock() else {
            return false;
        };
        let went = self.ask(network).await;
        if went {
            self.asked.store(true, Ordering::Release);
        }
        went
    }

    /// Ask this network's number with a local broadcast
    /// What-Is-Network-Number, which carries no DNET and is never routed
    /// (Clause 6.4.19). Whether it went: a send that fails is logged and
    /// left for the next need.
    async fn ask<T: TransportPort + 'static>(&self, network: &NetworkLayer<T>) -> bool {
        let ask =
            network.send_network_message(NetworkMessageType::WHAT_IS_NETWORK_NUMBER, &[], None);
        match self.noting(ask).await {
            Ok(()) => true,
            Err(error) => {
                debug!(%error, "What-Is-Network-Number broadcast failed");
                false
            }
        }
    }
}

impl ClientOptions {
    /// How long the answers to the client's What-Is-Network-Number count
    /// (#2826): see `window_for`.
    pub(super) fn number_window(&self, config: &ClientConfig) -> Duration {
        #[cfg(feature = "test-support")]
        if let Some(window) = self.number_window {
            return window;
        }
        window_for(Duration::from_millis(config.apdu_timeout_ms))
    }

    /// Test support: take the answers to the client's
    /// What-Is-Network-Number for `window` after each question, in place of
    /// the window its APDU timeout and the longest held answer set (#2826),
    /// so a wire test on the wall clock learns a number in well under that.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_number_window(mut self, window: Duration) -> Self {
        self.number_window = Some(window);
        self
    }
}

/// A question's note while its send runs: put back as it was unless the
/// send finished and went.
struct Note<'a> {
    asking: &'a Asking,
    question: Question,
    before: Option<Question>,
    went: bool,
}

impl Drop for Note<'_> {
    fn drop(&mut self) {
        if self.went {
            return;
        }
        let mut asked = self.asking.asked_at.lock();
        if *asked == Some(self.question) {
            *asked = self.before;
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
    /// Peers sent a segmented request's whole first window at once (#2742),
    /// after the table and the asking, so neither moves. Only a segmented
    /// request proposing a window above one touches it.
    pub(super) whole_first_window: SyncMutex<Option<Box<super::first_window::WholeFirstWindow>>>,
    /// Wakes the purge task when the dispatch task has held a routed I-Am
    /// that came through another station than its row's router, so the
    /// task checks the path (#2823). Last, so nothing before it moves.
    pub(super) route_cue: Notify,
}

impl Discovery {
    /// `control_drops`: the network layer's control queue's counts, which a
    /// seated client can't read.
    pub(super) fn new(
        numbers: bool,
        seated: bool,
        control_drops: Option<QueueAdmissionCounters>,
    ) -> Self {
        Self {
            table: SyncMutex::new(DeviceTable::new()),
            asking: Asking {
                seated,
                ..Asking::new(numbers, control_drops)
            },
            whole_first_window: SyncMutex::new(None),
            route_cue: Notify::new(),
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
/// subscribers, the link's returns, when the transport reports them, how
/// long the answers to a question count, and what the router check before
/// a routed network's number is taken needs: the client's network reports,
/// its routed paths and the check's wait, the APDU timeout (#2826).
pub(super) struct Owner {
    pub(super) discovery: Arc<Discovery>,
    pub(super) device_tx: broadcast::Sender<DeviceEvent>,
    pub(super) collision_tx: broadcast::Sender<DeviceCollisionEvent>,
    pub(super) link_returns: Option<watch::Receiver<u64>>,
    pub(super) window: Duration,
    pub(super) reports: broadcast::Sender<ReceivedNetworkReport>,
    pub(super) paths: Arc<RoutedPathLimits>,
    pub(super) check_wait: Duration,
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

/// The owner's number and what follows a change of it: the discovery
/// table it folds, the table's subscribers, and the throttled logs.
struct Held<'a> {
    state: NetworkNumber,
    table: &'a SyncMutex<DeviceTable>,
    notices: TableNotices<'a>,
    /// A changed number: the table follows each change, the log once an
    /// interval.
    changes: WarnThrottle,
    /// A window that decided nothing for a disagreement (#2826).
    splits: WarnThrottle,
}

impl Held<'_> {
    /// Act on what a closed window of answers decided (#2826): count and
    /// log a split; take an agreed number, after the router check when the
    /// client reaches that network through a router, unless a router
    /// routes there or may have said so.
    async fn decided<T: TransportPort + 'static>(
        &mut self,
        decision: Decision,
        check: &RouterCheck<'_, T>,
    ) {
        let (number, flag) = match decision {
            Decision::Unanswered => return,
            Decision::Void => {
                debug!(
                    "a What-Is-Network-Number window's question may not have gone; it decides \
                     nothing"
                );
                return;
            }
            Decision::Lost => {
                return self.refused(
                    check.asking,
                    None,
                    "an answer to What-Is-Network-Number may have been lost",
                );
            }
            Decision::Split => {
                return self.refused(
                    check.asking,
                    None,
                    "the answers to What-Is-Network-Number named different numbers",
                );
            }
            Decision::Agreed { number, flag } => (number, flag),
        };
        if number != self.state.snapshot().0 && check.needed(self.table, number) {
            match check.routed(number).await {
                Routed::Nowhere => {}
                Routed::Here => {
                    return self.refused(
                        check.asking,
                        Some(number),
                        "a router on this link routes to the network the answers named",
                    );
                }
                Routed::Unknown => {
                    return self.refused(
                        check.asking,
                        Some(number),
                        "a router's answer for the network the answers named may have been lost",
                    );
                }
            }
        }
        self.apply(number, flag, check.network);
    }

    /// A window decided nothing, for `why`: count it, and log it once an
    /// interval. `named` is the number its answers agreed on, if they did.
    fn refused(&self, asking: &Asking, named: Option<u16>, why: &'static str) {
        asking.disagreements.fetch_add(1, Ordering::Relaxed);
        warn_throttled!(
            self.splits,
            number = self.state.snapshot().0,
            named,
            why,
            "{why}; this network's number stays as it was"
        );
    }

    /// Apply `number` with `flag` by Clause 6.4.20's precedence. A number
    /// that changes is published and folds the discovery table.
    fn apply<T: TransportPort + 'static>(
        &mut self,
        number: u16,
        flag: u8,
        network: &NetworkLayer<T>,
    ) {
        let before = self.state.snapshot().0;
        self.state.observe(number, flag);
        let after = self.state.snapshot().0;
        if after == before {
            return;
        }
        if before != 0 {
            warn_throttled!(
                self.changes,
                previous = before,
                number = after,
                "this network's number changed; rows made local under the previous number \
                 are dropped"
            );
        }
        let this_network = network.local_network_number();
        device_events::learned_number(self.table, this_network, self.state, self.notices);
    }
}

/// A Number control and when the client heard it: an answer is judged by
/// when it came, not when the owner took it from its queue (#2826).
#[derive(Debug, Clone, Copy)]
pub(super) struct Received {
    pub(super) control: NumberControl,
    pub(super) at: Instant,
}

impl Received {
    /// `control`, heard now.
    pub(super) fn now(control: NumberControl) -> Self {
        Self {
            control,
            at: Instant::now(),
        }
    }
}

pub(super) fn spawn<T: TransportPort + 'static>(
    network: &Arc<NetworkLayer<T>>,
    owner: Owner,
) -> (mpsc::Sender<Received>, JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel(CAPACITY);
    let network = Arc::clone(network);
    let task = tokio::spawn(async move {
        let Owner {
            discovery,
            device_tx,
            collision_tx,
            mut link_returns,
            window,
            reports,
            paths,
            check_wait,
        } = owner;
        let asking = &discovery.asking;
        let check = RouterCheck {
            network: &network,
            reports: &reports,
            asking,
            paths: &paths,
            wait: check_wait,
        };
        let mut held = Held {
            state: NetworkNumber::default(),
            table: &discovery.table,
            notices: (&device_tx, &collision_tx),
            changes: WarnThrottle::default(),
            splits: WarnThrottle::default(),
        };
        // The answer to a broadcast What-Is-Network-Number, held for its
        // random wait and dropped when another node answers first
        // (Clause 6.4.19, #2597). It lives in this task alone.
        let mut answer = DelayedAnswer::new();
        // The window a question opens, and the hold on questions that
        // unasked announcements call for (#2826): in this task alone too.
        let mut learning = Learning::new(window);
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
                    announce(&network, held.state).await;
                    continue;
                }
                // Not while controls wait in the queue: one that came
                // before the close is an answer still to count.
                () = learning.closes(), if rx.is_empty() => {
                    let decision = learning.close(asking.losses(), asking.asked_at());
                    held.decided(decision, &check).await;
                    continue;
                }
            };
            let Received { control, at: now } = control;
            match control {
                NumberControl::NumberIs { number, flag } => {
                    // A window that closed before this came is decided
                    // first, so its answers and later ones never mix.
                    if learning.due(now) {
                        let decision = learning.close(asking.losses(), asking.asked_at());
                        held.decided(decision, &check).await;
                    }
                    let ours = held.state.snapshot().0;
                    match learning.heard((ours, number, flag), asking.asked_at(), now) {
                        Heard::Answer => {}
                        // The number held: only its quality can change.
                        Heard::Same => {
                            held.state.observe(number, flag);
                        }
                        Heard::Cue { ask } => {
                            debug!(number, ask, "an unasked Network-Number-Is is a cue");
                            // Only a question that went takes the hold.
                            if ask && asking.cue(&network).await {
                                learning.cue_asked(Instant::now());
                            }
                        }
                    }
                    // After the state took the announcement in, so one
                    // naming the number held counts as agreeing.
                    answer.heard(held.state, number, flag);
                }
                NumberControl::WhatIs { broadcast } => {
                    if answer.asked(held.state, broadcast) {
                        announce(&network, held.state).await;
                    }
                }
            }
        }
    });
    (tx, task)
}
