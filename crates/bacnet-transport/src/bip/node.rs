//! The hosted B/IP node: [`bacnet_bip_core::node`] on Tokio (#1622).
//!
//! The engine's [`Link`] decides every receive and send: the receive loop
//! and the send paths copy it out from behind a read lock that only a
//! foreign device's move to another BBMD writes (#1825), so it is never
//! contended in practice, and decide on the copy. Its [`Node`] holds this
//! node's own management exchanges and a foreign device's registration,
//! behind a std mutex that only a response, a foreign device's broadcast, an
//! exchange and the driver's timer take, and never across an await. After
//! every call the node's actions are drained under that lock: an exchange
//! that timed out fails its waiter at once, and a send the node asks for (a
//! registration attempt, or the broadcasts it held for one) waits for the
//! driver task, which makes every such send. A call made where nothing can
//! be awaited, as when a request is dropped, so never has to send.
//!
//! Two clocks, which never meet: the node runs on Tokio time from the start
//! ([`tokio_clock`]), which paused-time tests rely on, and the BBMD on std
//! time, as before. No time on one is compared with a
//! time on the other, and the management rate limit keeps its own std
//! clock, as before.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddrV4;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Duration;

use bacnet_bip_codec::{BvlcFunction, Bvll, BvllError};
use bacnet_bip_core::node::{
    Action, Admission, ConfigError, ExchangeId, Foreign, Inbound, Link, LocalFailure, Mode, Node,
    NodeCounters, Policy, RegistrationEvent, Response, SendRefusal,
};
use bacnet_bip_core::Instant as CoreInstant;
use bacnet_bip_core::LocalSubnet;
use bacnet_time::HostClock;
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use tokio::sync::{oneshot, Notify};
use tracing::{debug, info, warn};

use super::client_snapshot::{ClientRecord, ForeignRegistrationSnapshot};
use super::foreign_device::HELD_BROADCASTS;
use super::rate_limit::ManagementRateLimiter;
use super::send_counters::SendRefusals;
use super::BipSocket;
use crate::bbmd;
use crate::bvll::{bvll_error, encode_bvll, BvllMessage};
use crate::runtime_clock::tokio_clock;

/// Exchange slots, which the management helpers and a foreign device's
/// registration share. Each holds one exchange per peer and operation, so
/// requests to different peers, or for different operations, run at the
/// same time (#1622); only the same request to the same peer, or a second
/// one a success answers, waits its turn.
const SLOTS: usize = 8;

/// The engine as the hosted transport runs it.
pub(super) type HostNode = Node<SLOTS>;

/// The host's rules [`Link::receive`] consults: the management rate limit,
/// and the size limit on a table ACK.
pub(super) struct HostPolicy<'a> {
    limiter: &'a Mutex<ManagementRateLimiter>,
    /// Why [`accept_table`](Policy::accept_table) refused an ACK, which
    /// the request waiting for it fails with.
    refusal: Option<Error>,
}

impl<'a> HostPolicy<'a> {
    pub(super) fn new(limiter: &'a Mutex<ManagementRateLimiter>) -> Self {
        Self {
            limiter,
            refusal: None,
        }
    }

    /// Why the policy refused a table ACK, if it did.
    pub(super) fn into_refusal(self) -> Option<Error> {
        self.refusal
    }
}

impl Policy for HostPolicy<'_> {
    fn admit_management(
        &mut self,
        source: SocketAddrV4,
        _function: BvlcFunction,
        _now: CoreInstant,
    ) -> bool {
        let mut limiter = self.limiter.lock().unwrap_or_else(PoisonError::into_inner);
        limiter.check_now(source.ip().octets())
    }

    fn accept_table(&mut self, ack: &Bvll<'_>) -> bool {
        let checked = match *ack {
            Bvll::ReadBroadcastDistributionTableAck(table) => bbmd::bdt_from_wire(table).map(drop),
            Bvll::ReadForeignDeviceTableAck(table) => bbmd::fdt_from_wire(table).map(drop),
            _ => Ok(()),
        };
        match checked {
            Ok(()) => true,
            Err(error) => {
                self.refusal = Some(error);
                false
            }
        }
    }
}

/// What a manual request gets: the response, or why there is none.
pub(super) type Outcome = Result<BvllMessage, Error>;

/// A manual request waiting for its response.
pub(super) struct Waiter {
    pub(super) tx: oneshot::Sender<Outcome>,
    /// For the error a timeout reports.
    pub(super) timeout: Duration,
    /// A response that beat the request's send, kept until the node says
    /// the send settles it.
    pub(super) early: Option<Outcome>,
}

/// A registration attempt the node asked to send.
#[derive(Debug, Clone, Copy)]
struct Attempt {
    exchange: ExchangeId,
    to: SocketAddrV4,
    ttl: u16,
}

/// What the node lock guards.
pub(super) struct NodeState {
    pub(super) node: HostNode,
    /// Manual requests, by exchange.
    pub(super) waiters: HashMap<ExchangeId, Waiter>,
    /// A registration attempt for the driver to send.
    register: Option<Attempt>,
    /// When the automatic attempt in flight started.
    attempt_started: Option<CoreInstant>,
    /// Broadcasts held for the first registration.
    held: VecDeque<Bytes>,
    /// Held broadcasts released, for the driver to send.
    release: Vec<Bytes>,
    /// Broadcasts made while released ones still wait for the driver, which
    /// sends them next, so none overtakes another. At most
    /// [`HELD_BROADCASTS`]; one more is refused as a full hold.
    queued: Vec<Bytes>,
    /// The driver is sending broadcasts it took from `release` and `queued`.
    releasing: bool,
    /// A refusal was logged at warn, and the next ones go at debug until a
    /// success or the lease's end.
    refusal_warned: bool,
    /// Set by stop: no request starts, and nothing more is counted.
    pub(super) stopped: bool,
}

impl NodeState {
    /// Do what the node asks, and add its counts to `record`.
    fn drain(&mut self, now: CoreInstant, record: &ClientRecord) {
        loop {
            match self.node.poll_action() {
                Action::Idle => break,
                Action::Register { exchange, to, ttl } => {
                    self.attempt_started = Some(now);
                    self.register = Some(Attempt { exchange, to, ttl });
                }
                Action::ExchangeEnded { exchange } => {
                    if let Some(waiter) = self.waiters.remove(&exchange) {
                        let _ = waiter.tx.send(Err(Error::Timeout(waiter.timeout)));
                    }
                }
                Action::ReleaseHeld => self.release.extend(self.held.drain(..)),
                Action::DiscardHeld => self.held.clear(),
                Action::Registration(event) => self.log_registration(event),
            }
        }
        let counters = self.node.take_counters();
        if !self.stopped {
            record.absorb(&counters);
        }
    }

    /// Hand `exchange`'s waiter its outcome, if it still waits.
    pub(super) fn complete(&mut self, exchange: ExchangeId, outcome: Outcome) {
        if let Some(waiter) = self.waiters.remove(&exchange) {
            let _ = waiter.tx.send(outcome);
        }
    }

    /// Log a change in a foreign device's registration: a line when it
    /// becomes registered, when the BBMD starts refusing it, and when the
    /// lease lapses; renewals, losses and further refusals in a row at
    /// debug. A success or a lapse ends a run of refusals, so the next
    /// refusal warns again: a BBMD that keeps refusing warns once, not at
    /// every attempt.
    fn log_registration(&mut self, event: RegistrationEvent) {
        let Mode::Foreign(foreign) = self.node.link().mode else {
            return;
        };
        let bbmd = foreign.bbmd;
        match event {
            RegistrationEvent::Registered { since, until } => {
                self.refusal_warned = false;
                let lease = Duration::from_nanos(until.saturating_duration_since(since).as_nanos());
                info!(%bbmd, ?lease, "B/IP foreign device registered with its BBMD");
            }
            RegistrationEvent::Refused { code } if self.refusal_warned => {
                debug!(%bbmd, %code, "B/IP BBMD refused the foreign-device registration");
            }
            RegistrationEvent::Refused { code } => {
                self.refusal_warned = true;
                warn!(%bbmd, %code, "B/IP BBMD refused the foreign-device registration");
            }
            RegistrationEvent::Lapsed => {
                self.refusal_warned = false;
                warn!(%bbmd, "B/IP foreign-device registration lapsed, with no renewal answered");
            }
            RegistrationEvent::Renewed { .. } => {
                self.refusal_warned = false;
                debug!(%bbmd, "Foreign registration renewed");
            }
            RegistrationEvent::Unanswered => {
                debug!("Foreign registration attempt did not return a matched result");
            }
            other => debug!(?other, "Foreign registration event"),
        }
    }
}

/// The node of one started transport, shared by its receive loop, its send
/// paths, its driver task and the requests in flight.
pub(super) struct NodeHandle {
    /// A copy of the node's link, for the receive and send decisions. Only
    /// [`retarget`](Self::retarget) and [`set_subnet`](Self::set_subnet)
    /// write it, under the node lock, after
    /// the node's own copy changed: lock order is the node lock, then this.
    link: RwLock<Link>,
    clock: HostClock,
    socket: Arc<BipSocket>,
    record: Arc<ClientRecord>,
    /// Where the sends the link refuses are counted.
    refusals: Arc<SendRefusals>,
    state: Mutex<NodeState>,
    /// Tells the driver that the node changed.
    wake: Notify,
    /// Runs once in a broadcast send, between the link's decision and the
    /// node's admission: where a test moves the node.
    #[cfg(test)]
    pub(super) before_admit: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl NodeHandle {
    /// The node on `link`, sending from `socket`, counting its exchanges
    /// into `record` and the sends its link refuses into `refusals`. A
    /// foreign device's first registration attempt is admitted at once and
    /// sent when the driver first runs, which is also when what the node
    /// counted for it reaches `record`, as with the registration worker
    /// before the engine.
    pub(super) fn new(
        link: Link,
        socket: Arc<BipSocket>,
        record: Arc<ClientRecord>,
        refusals: Arc<SendRefusals>,
    ) -> Result<Self, Error> {
        let clock = tokio_clock();
        let node = HostNode::new(link, clock.now()).map_err(config_error)?;
        let handle = Self {
            link: RwLock::new(link),
            clock,
            socket,
            record,
            refusals,
            state: Mutex::new(NodeState {
                node,
                waiters: HashMap::new(),
                register: None,
                attempt_started: None,
                held: VecDeque::new(),
                release: Vec::new(),
                queued: Vec::new(),
                releasing: false,
                refusal_warned: false,
                stopped: false,
            }),
            wake: Notify::new(),
            #[cfg(test)]
            before_admit: Mutex::new(None),
        };
        Ok(handle)
    }

    /// The node's link as it stands: its address, subnet and mode. A copy,
    /// so no lock is held while the caller decides on it.
    pub(super) fn link(&self) -> Link {
        *self.link.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether [`stop`](Self::stop) has run.
    pub(super) fn is_stopped(&self) -> bool {
        self.lock().stopped
    }

    /// Whether this node, a foreign device, would take `foreign` as its
    /// registration ([`retarget`](Self::retarget)), changing nothing.
    pub(super) fn check_retarget(&self, foreign: Foreign) -> Result<(), Error> {
        let mut link = self.link();
        if !matches!(link.mode, Mode::Foreign(_)) {
            return Err(config_error(ConfigError::NotForeign));
        }
        link.mode = Mode::Foreign(foreign);
        link.validate().map_err(config_error)
    }

    /// Register with `foreign`'s BBMD and time-to-live from now on
    /// ([`Node::retarget`]): the attempt the driver hasn't sent yet is
    /// dropped, and one to the new BBMD goes at once. Refused, changing
    /// nothing, as the node refuses it. A stopped node has nothing to move:
    /// the next start builds a new one.
    pub(super) fn retarget(&self, foreign: Foreign) -> Result<(), Error> {
        self.with(|state, now| {
            if state.stopped {
                return Ok(());
            }
            state.node.retarget(foreign, now).map_err(config_error)?;
            // The node cancelled the attempt this would have sent; its
            // replacement comes with the drain after this call.
            state.register = None;
            state.attempt_started = None;
            state.refusal_warned = false;
            *self.link.write().unwrap_or_else(PoisonError::into_inner) = *state.node.link();
            info!(bbmd = %foreign.bbmd, ttl = foreign.ttl, "B/IP foreign device moved to a new BBMD registration");
            Ok(())
        })
    }

    /// Send broadcasts to `subnet` and take them from it from now on
    /// ([`Node::set_subnet`]): a B/IP-M node joining, changing or leaving its
    /// group (#941). A stopped node changes nothing.
    pub(super) fn set_subnet(&self, subnet: LocalSubnet) -> Result<(), Error> {
        self.with(|state, _| {
            if state.stopped {
                return Ok(());
            }
            state.node.set_subnet(subnet).map_err(config_error)?;
            *self.link.write().unwrap_or_else(PoisonError::into_inner) = *state.node.link();
            Ok(())
        })
    }

    /// The node's clock.
    pub(super) fn clock(&self) -> HostClock {
        self.clock
    }

    /// The socket every send leaves from.
    pub(super) fn socket(&self) -> &Arc<BipSocket> {
        &self.socket
    }

    /// The error for a send the link refused, counted.
    pub(super) fn refuse_send(&self, refusal: SendRefusal) -> Error {
        self.refusals.count(refusal);
        send_refusal(refusal)
    }

    fn lock(&self) -> MutexGuard<'_, NodeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Call into the node now, under the lock, then drain what it asks for
    /// and wake the driver.
    pub(super) fn with<R>(&self, call: impl FnOnce(&mut NodeState, CoreInstant) -> R) -> R {
        let mut state = self.lock();
        let now = self.clock.now();
        let result = call(&mut state, now);
        state.drain(now, &self.record);
        drop(state);
        self.wake.notify_one();
        result
    }

    /// Match a response against this node's exchanges, and hand a manual
    /// request its outcome. `frame` is the whole datagram; `refusal` is
    /// why the policy refused a table ACK.
    pub(super) fn resolve(&self, response: Response<'_>, frame: &[u8], refusal: Option<Error>) {
        let function = response.function;
        self.with(|state, now| match state.node.resolve(response, now) {
            Inbound::Matched {
                exchange,
                message,
                accepted,
                settled,
            } => {
                if !state.waiters.contains_key(&exchange) {
                    return;
                }
                let outcome = if accepted {
                    Ok(owned(function, frame))
                } else {
                    Err(refused(message.err(), refusal))
                };
                if settled {
                    state.complete(exchange, outcome);
                } else if let Some(waiter) = state.waiters.get_mut(&exchange) {
                    waiter.early = Some(outcome);
                }
            }
            Inbound::Unmatched { peer, code, late } => {
                debug!(%peer, ?code, late, %function, "Received unmatched BVLC response");
            }
            _ => {}
        });
    }

    /// Whether a foreign device's broadcast of `npdu` goes now, from the
    /// caller, and on which link: the one the node decided under, which a
    /// move to another BBMD may have changed since the caller read it
    /// (#1825). Otherwise (`None`) the node keeps it: held until it
    /// releases or drops it, or queued behind released broadcasts the
    /// driver hasn't sent yet, so that broadcasts go in the order they were
    /// made. That queue takes as many as the hold; one more is refused and
    /// counted as a full hold.
    pub(super) fn admit_broadcast(&self, npdu: &[u8]) -> Result<Option<Link>, Error> {
        self.with(|state, now| {
            let admission = state
                .node
                .admit_broadcast(now)
                .map_err(|refusal| self.refuse_send(refusal))?;
            let copy = || Bytes::copy_from_slice(npdu);
            let waiting = state.releasing || !state.release.is_empty() || !state.queued.is_empty();
            match admission {
                Admission::Hold => state.held.push_back(copy()),
                Admission::Send if waiting => {
                    if state.queued.len() >= usize::from(HELD_BROADCASTS) {
                        if !state.stopped {
                            let mut counters = NodeCounters::default();
                            counters.broadcasts_hold_full = 1;
                            self.record.absorb(&counters);
                        }
                        return Err(self.refuse_send(SendRefusal::HoldFull));
                    }
                    state.queued.push(copy());
                }
                Admission::Send => return Ok(Some(*state.node.link())),
            }
            Ok(None)
        })
    }

    /// Fill in a foreign device's registration as of now: the time until
    /// its next automatic attempt (while one is in flight, the latest the
    /// next can go, one renewal interval after it started), and its lease.
    /// Nothing changes in the other modes, or once the node has stopped: a
    /// status handle can read it in the moment before the transport clears
    /// its status, and a stopped node holds no lease.
    pub(super) fn fill_registration(&self, foreign: &mut ForeignRegistrationSnapshot) {
        let state = self.lock();
        if state.stopped {
            return;
        }
        let now = self.clock.now();
        let Some(snapshot) = state.node.registration(now) else {
            return;
        };
        let renewal = match state.node.link().mode {
            Mode::Foreign(foreign) => Some(foreign.effective_renewal()),
            Mode::Normal | Mode::Bbmd => None,
        };
        let next = snapshot
            .next_attempt
            .or_else(|| Some(state.attempt_started? + renewal?));
        let until =
            |at: CoreInstant| Duration::from_nanos(at.saturating_duration_since(now).as_nanos());
        foreign.next_attempt_in = next.map(until);
        foreign.registered = snapshot.registered;
        foreign.lease_remaining = snapshot
            .lease_remaining
            .map(|left| Duration::from_nanos(left.as_nanos()));
    }

    /// What this node's exchanges came to, as the transport reports them.
    #[cfg(test)]
    pub(super) fn snapshot(&self) -> super::BvlcClientSnapshot {
        let mut snapshot = self.record.snapshot();
        self.fill_registration(&mut snapshot.foreign_registration);
        snapshot
    }

    /// Stop: requests in flight fail as their channel closes, none starts,
    /// and nothing more is sent or counted. Broadcasts still held or queued
    /// count as discarded, so every held broadcast counts as released or
    /// discarded, and every queued one as sent or discarded.
    pub(super) fn stop(&self) {
        let mut state = self.lock();
        let kept = state.held.len() + state.queued.len();
        let discarded = u32::try_from(kept).unwrap_or(u32::MAX);
        if discarded > 0 && !state.stopped {
            let mut counters = NodeCounters::default();
            counters.broadcasts_discarded = discarded;
            self.record.absorb(&counters);
        }
        state.stopped = true;
        state.waiters.clear();
        state.register = None;
        state.held.clear();
        state.release.clear();
        state.queued.clear();
        state.releasing = false;
    }

    /// The sends this node's link refused, as counted.
    #[cfg(test)]
    pub(super) fn send_counters(&self) -> super::SendCounters {
        self.refusals.snapshot()
    }

    /// Send what the node asks for, and wake it at its deadlines, until the
    /// task is aborted.
    pub(super) async fn drive(self: Arc<Self>) {
        // What the node asked for at its start.
        self.with(|_, _| ());
        loop {
            // A registration attempt goes before the broadcasts released
            // with it, so the BBMD sees the device register before it is
            // asked to distribute for it.
            let attempt = self.lock().register.take();
            if let Some(attempt) = attempt {
                self.send_attempt(attempt).await;
            }
            self.send_releases().await;
            // A deadline past the host clock's range never comes.
            let deadline = self.lock().node.next_deadline();
            let wake = deadline.and_then(|at| self.clock.to_std(at));
            let timer = async {
                match wake {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending().await,
                }
            };
            // Neither future holds a lock, and either may be dropped.
            tokio::select! {
                biased;
                () = self.wake.notified() => {}
                () = timer => self.with(|state, now| state.node.on_deadline(now)),
            }
        }
    }

    async fn send_attempt(&self, attempt: Attempt) {
        let mut buf = BytesMut::with_capacity(6);
        let message = Bvll::RegisterForeignDevice { ttl: attempt.ttl };
        let sent = match encode_bvll(&mut buf, &message) {
            Ok(()) => self
                .socket
                .send_to(&buf, attempt.to)
                .await
                .map(drop)
                .map_err(Error::Transport),
            Err(error) => Err(error),
        };
        match sent {
            Ok(()) => {
                self.with(|state, now| state.node.exchange_sent(attempt.exchange, now));
            }
            Err(error) => {
                debug!(bbmd = %attempt.to, %error, "Foreign registration attempt was not sent");
                self.with(|state, now| {
                    let failure = LocalFailure::SendFailed;
                    state.node.exchange_failed(attempt.exchange, failure, now);
                });
            }
        }
    }

    /// Send the broadcasts in `release`, then those queued behind them, in
    /// order, with any queued while they go. Under the lock, `releasing` is
    /// cleared only once nothing is left, so a broadcast made after that
    /// goes from its caller, after these.
    async fn send_releases(&self) {
        loop {
            let batch = {
                let mut state = self.lock();
                let mut batch = std::mem::take(&mut state.release);
                batch.append(&mut state.queued);
                state.releasing = !batch.is_empty();
                batch
            };
            if batch.is_empty() {
                return;
            }
            for npdu in batch {
                self.send_released(&npdu).await;
            }
        }
    }

    /// Send a broadcast the node kept.
    async fn send_released(&self, npdu: &[u8]) {
        let Ok((outgoing, _)) = self.link().broadcast(npdu) else {
            return;
        };
        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        if let Err(error) = encode_bvll(&mut buf, &outgoing.message) {
            debug!(%error, "Failed to encode a held broadcast");
            return;
        }
        if let Err(error) = self.socket.send_to(&buf, outgoing.to).await {
            debug!(%error, "Failed to send a held broadcast");
        }
    }
}

/// An owned copy of the response in `frame`, a whole datagram whose header
/// the codec checked.
fn owned(function: BvlcFunction, frame: &[u8]) -> BvllMessage {
    BvllMessage {
        function,
        payload: Bytes::copy_from_slice(
            frame.get(bacnet_bip_codec::HEADER_LENGTH..).unwrap_or(&[]),
        ),
    }
}

/// Why a matched response doesn't answer: the codec's error, or the
/// policy's.
fn refused(decode: Option<BvllError>, policy: Option<Error>) -> Error {
    match (decode, policy) {
        (Some(error), _) => bvll_error(error),
        (None, Some(error)) => error,
        (None, None) => Error::Encoding("BVLC response refused".into()),
    }
}

pub(super) fn config_error(error: ConfigError) -> Error {
    Error::Encoding(error.to_string())
}

fn send_refusal(refusal: SendRefusal) -> Error {
    Error::Encoding(refusal.to_string())
}
