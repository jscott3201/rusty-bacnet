//! [`Node`]: the mutable half of a B/IP node, which owns its management
//! exchanges, a foreign device's registration and the broadcasts held for
//! it, and the timers behind them.

use core::net::SocketAddrV4;

use bacnet_bip_codec::{BvlcResultCode, Bvll, BvllError};
use bacnet_time::{Duration, Instant, earliest};

use crate::LocalSubnet;

use super::actions::{Action, Actions};
use super::config::{ConfigError, Foreign, Link, Mode, Retry};
use super::counters::{NodeCounters, RegisterAttempt, add, bump};
use super::exchange::{
    Early, ExchangeId, ExchangeRefusal, Exchanges, Key, LocalFailure, Operation, Phase,
};
use super::receive::{Datagram, Delivery, Discard, ForBbmd, Policy, Received, Response};
use super::registration::{
    Admission, AttemptOutcome, BroadcastDecision, HeldVerdict, Lifecycle, RegistrationEvent,
    RegistrationSnapshot,
};
use super::send::{Outgoing, SendRefusal};

/// A received datagram's fate, with responses matched to this node's own
/// exchanges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inbound<'d> {
    /// Hand an NPDU up ([`Received::Deliver`]).
    Deliver(Delivery<'d>),
    /// Send a NAK ([`Received::Nak`]).
    Nak {
        /// The request's source.
        to: SocketAddrV4,
        /// The request's own NAK.
        code: BvlcResultCode,
        /// The request was malformed.
        malformed: bool,
    },
    /// The BBMD's share ([`Received::Bbmd`]).
    Bbmd(ForBbmd<'d>),
    /// Dropped ([`Received::Discarded`]).
    Discarded(Discard),
    /// The response answers `exchange`. A malformed table ACK still
    /// completes the exchange it fits, with the codec's error, and so does
    /// one [`Policy::accept_table`] refused, as received.
    Matched {
        /// The exchange.
        exchange: ExchangeId,
        /// The response.
        message: Result<Bvll<'d>, BvllError>,
        /// Whether the codec decoded it and [`Policy::accept_table`] took
        /// it ([`Response::accepted`]). Only an accepted response counts as
        /// an answer; the caller fails the exchange otherwise.
        accepted: bool,
        /// Whether the exchange ended now. When it beat
        /// [`Node::exchange_sent`], the caller keeps the response until
        /// that returns `true`.
        settled: bool,
    },
    /// A second response for an exchange already holding one that beat its
    /// send. The first is kept; drop this one.
    Duplicate {
        /// The exchange.
        exchange: ExchangeId,
    },
    /// The response answers no live exchange. Well-formed ones count as
    /// unmatched, malformed ones as malformed ([`NodeCounters`]).
    Unmatched {
        /// Who sent it.
        peer: SocketAddrV4,
        /// A well-formed BVLC-Result's code.
        code: Option<BvlcResultCode>,
        /// It came within one response timeout of an exchange it fits
        /// ending: probably that exchange's answer, too late.
        late: bool,
    },
}

/// The mutable part of a B/IP node: up to `X` management exchanges at once
/// (at least 1, at most 256), a foreign device's registration, the
/// broadcasts held for it, and their timers.
///
/// Sans-IO: each call takes the current time, nothing reads a clock or a
/// socket, and what the caller must do comes out of
/// [`poll_action`](Node::poll_action), which it polls until
/// [`Action::Idle`] after every call. Sleep until
/// [`next_deadline`](Node::next_deadline) and call
/// [`on_deadline`](Node::on_deadline) with the time actually woken; every
/// other call first catches up on deadlines passed, so the order the caller
/// makes them in at one instant doesn't change the outcome. Time never
/// runs backwards: an earlier `now` than the last is taken as the last.
#[derive(Debug, Clone)]
pub struct Node<const X: usize = 4> {
    link: Link,
    exchanges: Exchanges<X>,
    lifecycle: Option<Lifecycle>,
    actions: Actions<X>,
    counters: NodeCounters,
    /// The newest Register-Foreign-Device attempt and its exchange.
    latest_register: Option<(ExchangeId, RegisterAttempt)>,
    now: Instant,
}

impl<const X: usize> Node<X> {
    /// A node on `link`, starting at `now`. A foreign device's first
    /// registration attempt is due at once.
    pub fn new(link: Link, now: Instant) -> Result<Self, ConfigError> {
        const { assert!(X >= 1 && X <= 256, "a node needs 1 to 256 exchange slots") };
        link.validate()?;
        let lifecycle = match &link.mode {
            Mode::Foreign(foreign) => Some(Lifecycle::new(foreign, now)),
            Mode::Normal | Mode::Bbmd => None,
        };
        let mut node = Self {
            link,
            exchanges: Exchanges::new(),
            lifecycle,
            actions: Actions::new(),
            counters: NodeCounters::default(),
            latest_register: None,
            now,
        };
        node.advance(now);
        Ok(node)
    }

    /// The node's link.
    #[must_use]
    pub const fn link(&self) -> &Link {
        &self.link
    }

    /// [`Link::receive`], then [`resolve`](Node::resolve) a response.
    pub fn receive<'d>(
        &mut self,
        datagram: &Datagram<'d>,
        now: Instant,
        policy: &mut impl Policy,
    ) -> Inbound<'d> {
        let now = self.catch_up(now, false);
        let inbound = match self.link.receive(datagram, now, policy) {
            Received::Deliver(delivery) => Inbound::Deliver(delivery),
            Received::Nak {
                to,
                code,
                malformed,
            } => Inbound::Nak {
                to,
                code,
                malformed,
            },
            Received::Bbmd(share) => Inbound::Bbmd(share),
            Received::Discarded(reason) => Inbound::Discarded(reason),
            Received::Response(response) => self.match_response(response, now),
        };
        self.advance(now);
        inbound
    }

    /// Match a response to the live exchange it answers.
    ///
    /// A table ACK answers its table read; a NAK, the operation it names
    /// ([`BvlcFunction::nak`](bacnet_bip_codec::BvlcFunction::nak)); a
    /// success, the peer's one live operation a success answers. A
    /// malformed BVLC-Result never completes anything. A
    /// Distribute-Broadcast-To-Network NAK from a foreign device's own BBMD
    /// that matches nothing says the BBMD has no registration for it (Annex
    /// J.4.5), and with [`Retry::Backoff`] pulls the next attempt forward;
    /// [`Retry::AtRenewal`] keeps its schedule.
    pub fn resolve<'d>(&mut self, response: Response<'d>, now: Instant) -> Inbound<'d> {
        // An attempt that falls due now goes after the match: it isn't sent
        // yet, so the response can't be its answer.
        let now = self.catch_up(now, false);
        let inbound = self.match_response(response, now);
        self.advance(now);
        inbound
    }

    fn match_response<'d>(&mut self, response: Response<'d>, now: Instant) -> Inbound<'d> {
        let Response {
            peer,
            function,
            message,
            accepted,
        } = response;
        let code = match message {
            Ok(Bvll::BvlcResult(code)) => Some(code),
            _ => None,
        };
        let valid = message.is_ok() && accepted;
        let key = if valid {
            Key::of(function, code)
        } else {
            bump(&mut self.counters.malformed_responses);
            if let Some(index) = self.exchanges.first_with(peer, function) {
                let operation = self.exchanges.slot(index).operation;
                bump(&mut self.counters.exchange_mut(operation).malformed_responses);
            }
            // Only a table ACK completes an exchange malformed.
            Operation::of_ack(function).map(Key::Operation)
        };
        let Some(index) = key.and_then(|key| self.exchanges.find(peer, key)) else {
            if !valid {
                return Inbound::Unmatched {
                    peer,
                    code: None,
                    late: false,
                };
            }
            return self.unmatched(peer, code, key, now);
        };
        let exchange = self.exchanges.id(index);
        let slot = self.exchanges.slot_mut(index);
        if slot.phase == Phase::Admitted {
            if slot.early.is_some() {
                return Inbound::Duplicate { exchange };
            }
            slot.early = Some(Early { code, valid });
            return Inbound::Matched {
                exchange,
                message,
                accepted: valid,
                settled: false,
            };
        }
        self.settle(index, code, valid, now);
        Inbound::Matched {
            exchange,
            message,
            accepted: valid,
            settled: true,
        }
    }

    /// A well-formed response that answers no live exchange.
    fn unmatched<'d>(
        &mut self,
        peer: SocketAddrV4,
        code: Option<BvlcResultCode>,
        key: Option<Key>,
        now: Instant,
    ) -> Inbound<'d> {
        let late = key.is_some_and(|key| self.exchanges.is_late(peer, key, now));
        bump(&mut self.counters.unmatched_responses);
        if late {
            bump(&mut self.counters.late_responses);
        }
        if code.is_some() {
            self.counters.last_unmatched_result = code;
        }
        if code == Some(BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK)
            && let (Mode::Foreign(foreign), Some(lifecycle)) =
                (&self.link.mode, &mut self.lifecycle)
            && foreign.bbmd == peer
            && foreign.retry == Retry::Backoff
        {
            bump(&mut self.counters.renewal_hints);
            lifecycle.hint(now);
        }
        Inbound::Unmatched { peer, code, late }
    }

    /// End the exchange in `index` with a response: `code` for a
    /// BVLC-Result, `None` for a table ACK; `valid` when it was accepted.
    fn settle(&mut self, index: usize, code: Option<BvlcResultCode>, valid: bool, now: Instant) {
        let exchange = self.exchanges.id(index);
        let slot = self.exchanges.retire(index, now);
        if !valid {
            return;
        }
        let counters = self.counters.exchange_mut(slot.operation);
        match code {
            Some(code) => {
                bump(&mut counters.results);
                counters.last_result = Some(code);
            }
            None => bump(&mut counters.acknowledgements),
        }
        let Some(code) = code else { return };
        if slot.operation != Operation::RegisterForeignDevice {
            return;
        }
        let accepted = code == BvlcResultCode::SUCCESSFUL_COMPLETION;
        if accepted {
            bump(&mut self.counters.registrations_accepted);
        } else {
            bump(&mut self.counters.registrations_refused);
        }
        let outcome = if accepted {
            AttemptOutcome::Accepted
        } else {
            AttemptOutcome::Refused
        };
        self.record_register(exchange, outcome, Some(code));
        if !slot.automatic {
            return;
        }
        let Some(lifecycle) = &mut self.lifecycle else {
            return;
        };
        let (event, verdict) = if accepted {
            lifecycle.accepted(now)
        } else {
            let verdict = lifecycle.refused(now);
            (Some(RegistrationEvent::Refused { code }), verdict)
        };
        if let Some(event) = event {
            self.actions.event(event);
        }
        self.held_verdict(verdict);
    }

    /// Whether a foreign device sends, holds or refuses a broadcast at
    /// `now`. Other modes always send.
    ///
    /// Until the first registration attempt is answered or times out, up to
    /// [`Foreign::hold`](super::Foreign::hold) broadcasts are held, and one
    /// more is refused with [`SendRefusal::HoldFull`]. Then they are all
    /// sent; the ones sent while no lease is live are counted.
    pub fn admit_broadcast(&mut self, now: Instant) -> Result<Admission, SendRefusal> {
        let now = self.advance(now);
        let Some(lifecycle) = &mut self.lifecycle else {
            return Ok(Admission::Send);
        };
        match lifecycle.admit_broadcast(now) {
            BroadcastDecision::Hold => {
                bump(&mut self.counters.broadcasts_held);
                Ok(Admission::Hold)
            }
            BroadcastDecision::Send { lapsed } => {
                if lapsed {
                    bump(&mut self.counters.broadcasts_unregistered);
                }
                Ok(Admission::Send)
            }
            BroadcastDecision::Full => {
                bump(&mut self.counters.broadcasts_hold_full);
                Err(SendRefusal::HoldFull)
            }
        }
    }

    /// Admit a manual management exchange: `request` to `peer`, answered
    /// within `timeout` of its send. Encode and send the returned message,
    /// then report it with [`exchange_sent`](Node::exchange_sent) or
    /// [`exchange_failed`](Node::exchange_failed).
    ///
    /// A foreign device refuses a request a success answers to its own
    /// BBMD ([`ExchangeRefusal::ManagedRegistration`]): the node owns that
    /// registration. A normal node or a BBMD may send any.
    pub fn begin_exchange<'r>(
        &mut self,
        peer: SocketAddrV4,
        request: Bvll<'r>,
        timeout: Duration,
        now: Instant,
    ) -> Result<(ExchangeId, Outgoing<'r>), ExchangeRefusal> {
        self.advance(now);
        let operation = Operation::of(request.function()).ok_or(ExchangeRefusal::NotARequest)?;
        if let Mode::Foreign(foreign) = &self.link.mode
            && foreign.bbmd == peer
            && operation.answered_by_success()
        {
            return Err(ExchangeRefusal::ManagedRegistration);
        }
        let ttl = match request {
            Bvll::RegisterForeignDevice { ttl } => ttl,
            _ => 0,
        };
        let exchange = self.admit(peer, operation, timeout, false, ttl)?;
        Ok((
            exchange,
            Outgoing {
                to: peer,
                message: request,
            },
        ))
    }

    fn admit(
        &mut self,
        peer: SocketAddrV4,
        operation: Operation,
        timeout: Duration,
        automatic: bool,
        ttl: u16,
    ) -> Result<ExchangeId, ExchangeRefusal> {
        let exchange = self
            .exchanges
            .admit(peer, operation, timeout, automatic, ttl)
            .inspect_err(|_| bump(&mut self.counters.exchange_mut(operation).busy))?;
        if operation == Operation::RegisterForeignDevice {
            bump(&mut self.counters.registration_attempts);
            let attempt = RegisterAttempt {
                bbmd: peer,
                ttl,
                outcome: AttemptOutcome::Pending,
                result: None,
            };
            self.latest_register = Some((exchange, attempt));
            self.counters.last_registration_attempt = Some(attempt);
        }
        Ok(exchange)
    }

    /// The request of `exchange` was sent at `now`; its response timeout
    /// starts. `true` when a response that beat the send settles it now.
    /// A stale or already reported exchange changes nothing.
    pub fn exchange_sent(&mut self, exchange: ExchangeId, now: Instant) -> bool {
        let now = self.advance(now);
        let Some(index) = self.exchanges.live(exchange) else {
            return false;
        };
        let slot = self.exchanges.slot_mut(index);
        if slot.phase != Phase::Admitted {
            return false;
        }
        slot.phase = Phase::Sent {
            deadline: now + slot.timeout,
        };
        let (operation, automatic, early) = (slot.operation, slot.automatic, slot.early.take());
        bump(&mut self.counters.exchange_mut(operation).sent);
        if automatic && let Some(lifecycle) = &mut self.lifecycle {
            lifecycle.sent();
        }
        let settled = early.is_some();
        if let Some(early) = early {
            self.settle(index, early.code, early.valid, now);
        }
        self.advance(now);
        settled
    }

    /// The caller ended `exchange` itself: its send failed, or it gave up.
    /// A stale exchange changes nothing.
    pub fn exchange_failed(&mut self, exchange: ExchangeId, why: LocalFailure, now: Instant) {
        let now = self.advance(now);
        let Some(index) = self.exchanges.live(exchange) else {
            return;
        };
        let slot = self.exchanges.retire(index, now);
        let outcome = match why {
            LocalFailure::SendFailed => {
                bump(&mut self.counters.exchange_mut(slot.operation).local_errors);
                AttemptOutcome::LocalError
            }
            LocalFailure::Cancelled => AttemptOutcome::Cancelled,
        };
        self.ended_unanswered(exchange, slot.operation, slot.automatic, outcome, now);
        self.advance(now);
    }

    /// An exchange ended without an answer.
    fn ended_unanswered(
        &mut self,
        exchange: ExchangeId,
        operation: Operation,
        automatic: bool,
        outcome: AttemptOutcome,
        now: Instant,
    ) {
        if operation == Operation::RegisterForeignDevice {
            self.record_register(exchange, outcome, None);
        }
        if !automatic {
            return;
        }
        if let Some(lifecycle) = &mut self.lifecycle {
            let verdict = lifecycle.failed(now, Some(outcome));
            self.actions.event(RegistrationEvent::Unanswered);
            self.held_verdict(verdict);
        }
    }

    /// Update the newest Register-Foreign-Device attempt, if `exchange` is
    /// it.
    fn record_register(
        &mut self,
        exchange: ExchangeId,
        outcome: AttemptOutcome,
        result: Option<BvlcResultCode>,
    ) {
        if let Some((latest, attempt)) = &mut self.latest_register
            && *latest == exchange
        {
            attempt.outcome = outcome;
            attempt.result = result;
            self.counters.last_registration_attempt = Some(*attempt);
        }
    }

    fn held_verdict(&mut self, verdict: Option<HeldVerdict>) {
        let Some(HeldVerdict { release, count }) = verdict else {
            return;
        };
        let counter = if release {
            &mut self.counters.broadcasts_released
        } else {
            &mut self.counters.broadcasts_discarded
        };
        add(counter, count);
        self.actions.held(release);
    }

    /// The earliest time the node has something to do without being asked:
    /// an exchange's response timeout, a registration attempt, or a lease's
    /// end.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        let exchange = self.exchanges.next_deadline().map(|(_, deadline)| deadline);
        let lifecycle = self.lifecycle.as_ref();
        earliest([
            exchange,
            lifecycle.and_then(Lifecycle::lapse_at),
            lifecycle.and_then(Lifecycle::due_at),
        ])
    }

    /// Do what is due at `now`, the time the caller actually woke.
    pub fn on_deadline(&mut self, now: Instant) {
        self.advance(now);
    }

    /// Catch up to `now`: handle every deadline at or before it, earliest
    /// first; at one instant, timeouts, then the lease's end, then a due
    /// attempt. Returns the time used, never earlier than the last.
    fn advance(&mut self, now: Instant) -> Instant {
        self.catch_up(now, true)
    }

    /// [`advance`](Self::advance), leaving a due attempt unstarted unless
    /// `start_due`.
    fn catch_up(&mut self, now: Instant, start_due: bool) -> Instant {
        let now = now.max(self.now);
        self.now = now;
        loop {
            let timeout = self.exchanges.next_deadline();
            let lifecycle = self.lifecycle.as_ref();
            let lapse = lifecycle.and_then(Lifecycle::lapse_at);
            let due = lifecycle.and_then(Lifecycle::due_at).filter(|_| start_due);
            let first = earliest([timeout.map(|(_, at)| at), lapse, due]);
            match first {
                Some(at) if at <= now => {}
                _ => return now,
            }
            if let Some((index, _)) = timeout.filter(|(_, at)| Some(*at) == first) {
                self.time_out(index, now);
            } else if lapse == first {
                if let Some(lifecycle) = &mut self.lifecycle {
                    lifecycle.lapse();
                }
                bump(&mut self.counters.lapses);
                self.actions.event(RegistrationEvent::Lapsed);
            } else {
                self.start_attempt(now);
            }
        }
    }

    fn time_out(&mut self, index: usize, now: Instant) {
        let exchange = self.exchanges.id(index);
        let slot = self.exchanges.retire(index, now);
        bump(&mut self.counters.exchange_mut(slot.operation).timeouts);
        if !slot.automatic {
            self.actions.ended(exchange);
        }
        self.ended_unanswered(
            exchange,
            slot.operation,
            slot.automatic,
            AttemptOutcome::TimedOut,
            now,
        );
    }

    /// Admit the automatic registration attempt that is due.
    fn start_attempt(&mut self, now: Instant) {
        let Mode::Foreign(foreign) = self.link.mode else {
            return;
        };
        let timeout = foreign.effective_response_timeout();
        let operation = Operation::RegisterForeignDevice;
        match self.admit(foreign.bbmd, operation, timeout, true, foreign.ttl) {
            Ok(exchange) => {
                if let Some(lifecycle) = &mut self.lifecycle {
                    lifecycle.admitted(now);
                }
                self.actions.register(exchange, foreign.bbmd, foreign.ttl);
            }
            Err(_) => {
                if let Some(lifecycle) = &mut self.lifecycle {
                    let verdict = lifecycle.failed(now, None);
                    self.held_verdict(verdict);
                }
            }
        }
    }

    /// Register a foreign device with `foreign` from `now` on: another
    /// BBMD, time-to-live or timing. The lifecycle restarts with an attempt
    /// to the new BBMD due at once, which the next
    /// [`poll_action`](Node::poll_action) hands out. The automatic attempt
    /// in flight ends as cancelled, so a late answer to it counts as
    /// unmatched. A lease proven by the same BBMD stands, since its entry
    /// does; one from another BBMD is dropped, and broadcasts are held
    /// again, as at start, until that BBMD's first answer
    /// ([`admit_broadcast`](Node::admit_broadcast)). Manual exchanges carry
    /// on.
    ///
    /// Refused, changing nothing, outside foreign mode
    /// ([`ConfigError::NotForeign`]) or with a registration
    /// [`Link::validate`] refuses.
    pub fn retarget(&mut self, foreign: Foreign, now: Instant) -> Result<(), ConfigError> {
        let Mode::Foreign(old) = self.link.mode else {
            return Err(ConfigError::NotForeign);
        };
        let mut link = self.link;
        link.mode = Mode::Foreign(foreign);
        link.validate()?;
        // Timeouts and a lapse due by now happen on the old registration; an
        // attempt due by now is not started for it.
        let now = self.catch_up(now, false);
        if let Some(index) = self.exchanges.automatic() {
            let exchange = self.exchanges.id(index);
            self.exchanges.retire(index, now);
            self.actions.forget_register(exchange);
            self.record_register(exchange, AttemptOutcome::Cancelled, None);
        }
        if let Some(lifecycle) = &mut self.lifecycle {
            lifecycle.retarget(&foreign, now, old.bbmd == foreign.bbmd);
        }
        self.link = link;
        self.advance(now);
        Ok(())
    }

    /// Move the node to another [`LocalSubnet`]: where its broadcasts go
    /// and which destination counts as one. A B/IP-M node that joins,
    /// changes or leaves its multicast group (Annex J.8) does this, and so
    /// must every copy of the link the host decides on. Nothing else
    /// changes: exchanges and a foreign device's registration carry on.
    ///
    /// Refused, changing nothing, with a link [`Link::validate`] refuses.
    pub fn set_subnet(&mut self, subnet: LocalSubnet) -> Result<(), ConfigError> {
        let mut link = self.link;
        link.subnet = subnet;
        link.validate()?;
        self.link = link;
        Ok(())
    }

    /// The next thing to do, or [`Action::Idle`].
    pub fn poll_action(&mut self) -> Action {
        self.actions.poll()
    }

    /// A foreign device's registration as of `now`; `None` in the other
    /// modes.
    #[must_use]
    pub fn registration(&self, now: Instant) -> Option<RegistrationSnapshot> {
        self.lifecycle
            .as_ref()
            .map(|lifecycle| lifecycle.snapshot(now.max(self.now)))
    }

    /// The counts since the counters were last taken, which then start
    /// again from zero: add them to the host's own totals, as wide as it
    /// needs. Each count is a `u32` that saturates between takes.
    #[must_use = "the counts are gone once taken; add them to your totals"]
    pub fn take_counters(&mut self) -> NodeCounters {
        core::mem::take(&mut self.counters)
    }

    /// The counts since the counters were last taken, as
    /// [`take_counters`](Self::take_counters) would return them, without
    /// taking them: for a look between takes. A node whose counters are
    /// never taken counts from its start.
    #[must_use]
    pub const fn counters(&self) -> NodeCounters {
        self.counters
    }
}
