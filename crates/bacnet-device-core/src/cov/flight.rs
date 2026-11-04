//! Outstanding confirmed reports, their acknowledgment and the pause after a
//! failure (#896), as plain state in the table.
//!
//! A confirmed COV notification completes its references' baselines only when
//! the subscriber acknowledges it: [`CovTable::begin_confirmed`] keeps the
//! observation each reference carried, and [`CovTable::delivered`] commits
//! them on the Ack. Until then its coordinate has one outstanding report,
//! marked with that report's ticket. The coordinate is an ordinary or Single subscription, or a whole
//! COV-multiple context: every notification to a context has to carry all the
//! timestamped changes queued for it (Clauses 13.1, 13.16.3.1.2.3 and
//! 13.17.1.1.5), so one reference's report cannot be outstanding while a
//! sibling's goes out.
//!
//! Fanouts skip a busy coordinate, so its later changes wait. The host's
//! worker reports how the report ended through [`CovTable::delivered`]: the
//! Ack clears the mark, and the host then evaluates the coordinate's
//! references again and reports, in one notification, whatever changed
//! meanwhile.
//!
//! A report that runs out of retries or is answered with an Error clears its
//! mark and holds its coordinate off for one full retry cycle. Fanouts inside
//! the hold-off skip the coordinate and schedule nothing. The first fanout
//! after it reports the change again; for a context it hands every reference
//! to one follow-up, since the failed report's changes may sit on other
//! objects. A subscriber that stopped answering, or keeps refusing, therefore
//! costs at most one delivery attempt per hold-off however often its objects
//! change, and cannot keep the in-flight slots to itself. A report that was
//! never sent, because DeviceCommunicationControl ended it at a retry (Clause
//! 16.1) or its transaction closed, clears the mark without a hold-off when
//! its worker reports it: the subscriber did not fail.
//!
//! Each report also carries a deadline, the retry budget plus one timeout,
//! at which it ends itself if no outcome arrived. That is for a worker that
//! can report nothing: one aborted, as a host's shutdown aborts them, keeps
//! the mark until then, and the coordinate is freed at the deadline, with
//! no hold-off and its references owed a fresh look
//! ([`CovTable::expire`]). A live worker stops half a timeout before the
//! deadline, however long its sends stall, and reports a failure. Should a worker stalled past the
//! deadline still be running, the next report on the coordinate fences it
//! ([`Fenced::tokens`](super::Fenced::tokens)) as it begins.
//!
//! The standard delivers a confirmed notification with the usual APDU timeout and
//! retries (Clause 5.4.4) and asks nothing further once they end. Reporting again
//! after a hold-off is local policy.
//!
//! A replaced ordinary or Single subscription starts with a fresh flight. A
//! context gets one when its route changes, and when a re-subscription that
//! lists references arrives while it is busy, so the initial report is not held
//! behind the old one (Clauses 13.14.2 and 13.16.2). The old flight is fenced:
//! its outstanding report's token is handed to the host
//! ([`Fenced::tokens`](super::Fenced::tokens)) so its worker stops retrying, and
//! the report can neither complete nor unmark the coordinate.
//!
//! A fenced report's outcome no longer counts, yet it may have reached the
//! subscriber. So when the old flight owed a follow-up (its report was
//! outstanding, or had failed and was holding off or owed), every reference of
//! the context is evaluated again, and the untimestamped references that report
//! carried and the re-subscription kept first forget their baselines (#923).
//! The follow-up then reports their current value even if it went back to the
//! baseline, which the subscriber would otherwise keep showing. References the
//! report did not carry keep their acknowledged baselines, so the follow-up does
//! not grow to the whole context. Timestamped references need no reset: the
//! fenced report's history returns to their queue, and a change back is captured
//! as one more change, so the subscriber ends at the current value.
//!
//! Without a fence the same gap remains, and is accepted: a failed report keeps
//! its baselines, so if it did arrive (only its Acks were lost) and the value
//! goes back before the follow-up, nothing is sent again. A fence after the owed
//! follow-up has been taken clears nothing either.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use core::borrow::Borrow;

use crate::cov::{CovObservation, CovSubscriptionKey, MultipleContextKey, SubscriberEndpoint};
use bacnet_time::{Duration, Instant};

use super::observation_order::{Completion, PreparedCovCompletion};
use super::{CovSubscriptionSnapshot, CovTable};

/// How long a confirmed report may take: the APDU timeout and the retries
/// after the first attempt (Clause 5.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlightTiming {
    /// How long each attempt waits for the answer.
    pub timeout: Duration,
    /// Attempts after the first.
    pub retries: u8,
}

impl Default for FlightTiming {
    /// Three seconds and three retries, the server's defaults.
    fn default() -> Self {
        Self::new(Duration::from_secs(3), 3)
    }
}

impl FlightTiming {
    /// Timing with `timeout` per attempt and `retries` retries.
    pub const fn new(timeout: Duration, retries: u8) -> Self {
        Self { timeout, retries }
    }

    /// The hold-off after a failure: one full retry cycle, the first attempt
    /// and every retry.
    pub fn hold_off(self) -> Duration {
        self.timeout.saturating_mul(u32::from(self.retries) + 1)
    }

    /// How long after it begins a report ends itself: the retry cycle plus
    /// one timeout of margin, so a live worker always reports first.
    pub fn deadline(self) -> Duration {
        self.timeout.saturating_mul(u32::from(self.retries) + 2)
    }

    /// How long after it begins a live sender stops waiting for an answer
    /// and reports a failure: the retry cycle and half a timeout, half a
    /// timeout before the [deadline](Self::deadline), however late its
    /// first attempt went out.
    pub fn cutoff(self) -> Duration {
        let half = Duration::from_nanos(self.timeout.as_nanos() >> 1);
        self.hold_off().saturating_add(half)
    }
}

/// One report: the token its outcome comes back with through
/// [`CovTable::delivered`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Token {
    pub(super) flight: u64,
    pub(super) ticket: u64,
}

#[cfg(test)]
impl Token {
    /// A token for tests that need one without a table.
    pub(crate) fn for_test(ticket: u64) -> Self {
        Self { flight: 0, ticket }
    }
}

/// How a report ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// An unconfirmed report went to the link: its observation completes.
    /// A confirmed report ends only by its answer, so this changes nothing
    /// for one.
    Sent,
    /// The subscriber acknowledged it.
    Acknowledged,
    /// Its retries ran out, or the subscriber answered with an Error: its
    /// coordinate holds off for one retry cycle.
    Failed,
    /// No attempt counts: DeviceCommunicationControl withdrew it, its
    /// transaction closed, or, unconfirmed, its send failed. A confirmed
    /// report's mark clears at once. A worker aborted at shutdown reports
    /// nothing; its report keeps the mark until its deadline.
    NotSent,
}

/// Why [`CovTable::begin_confirmed`] refused a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeginRefusal {
    /// A reference was removed, replaced, moved to another route or expired
    /// since it was captured. Its live state deserves a fresh look.
    NotCurrent,
    /// The coordinate already has an outstanding report, is holding off after
    /// a failure, or was acknowledged since it was captured. That outcome owns
    /// the follow-up.
    Busy,
}

/// The confirmed-report state of one coordinate. A flight the table holds
/// none of is idle.
#[derive(Debug, Default, Clone)]
pub(super) struct FlightState {
    /// Ticket of the outstanding report, or zero for none.
    ticket: u64,
    /// When the outstanding report ends itself.
    deadline: Option<Instant>,
    /// End of the hold-off after a failed report. Once it has passed, a
    /// context still owes one follow-up of all its references.
    hold_until: Option<Instant>,
    /// References the outstanding report carried, kept while a failed one is
    /// holding off or owed, and cleared once it is acknowledged or its owed
    /// follow-up is taken.
    carried: BTreeSet<CovSubscriptionKey>,
    /// The observation each reference of the outstanding report carried:
    /// its Ack commits them.
    completes: Vec<Completion>,
}

impl FlightState {
    /// A report is outstanding at `now`: begun, not ended, before its deadline.
    fn outstanding(&self, now: Instant) -> bool {
        self.ticket != 0 && self.deadline.is_none_or(|deadline| now < deadline)
    }

    /// The deadline of a report still marked outstanding, if any.
    pub(super) fn outstanding_deadline(&self) -> Option<Instant> {
        self.deadline.filter(|_| self.ticket != 0)
    }

    /// No outstanding report and no hold-off in force at `now`.
    fn idle(&self, now: Instant) -> bool {
        !self.outstanding(now) && self.hold_until.is_none_or(|until| now >= until)
    }

    /// End of a hold-off still in force at `now`, with no report outstanding.
    fn holding_until(&self, now: Instant) -> Option<Instant> {
        self.hold_until
            .filter(|until| !self.outstanding(now) && now < *until)
    }

    /// If a report is outstanding, or a failed one is still holding off or
    /// owed, the references it carried.
    fn owed_carried(&self, now: Instant) -> Option<&BTreeSet<CovSubscriptionKey>> {
        (self.outstanding(now) || self.hold_until.is_some()).then_some(&self.carried)
    }

    /// Nothing outstanding, held off or carried: the same as no state.
    fn is_idle_default(&self) -> bool {
        self.ticket == 0 && self.hold_until.is_none() && self.carried.is_empty()
    }
}

impl CovTable {
    /// A flight no entry has used yet.
    pub(super) fn new_flight(&mut self) -> u64 {
        self.next_flight += 1;
        self.next_flight
    }

    fn flight_idle(&self, flight: u64, now: Instant) -> bool {
        self.flights
            .get(&flight)
            .is_none_or(|state| state.idle(now))
    }

    /// Drop a flight's state once it holds nothing.
    fn tidy_flight(&mut self, flight: u64) {
        if self
            .flights
            .get(&flight)
            .is_some_and(FlightState::is_idle_default)
        {
            self.flights.remove(&flight);
        }
    }

    /// A replaced flight: its outstanding report's worker stops.
    pub(super) fn fence_flight(&mut self, flight: u64) {
        if let Some(state) = self.flights.remove(&flight)
            && state.ticket != 0
        {
            self.fenced.tokens.push(Token {
                flight,
                ticket: state.ticket,
            });
        }
    }

    /// End every report whose deadline has passed at `now` as if its worker
    /// had reported it unsent, and owe its coordinate's references a fresh
    /// look.
    pub(super) fn end_abandoned(&mut self, now: Instant) {
        let abandoned: Vec<_> = self
            .flights
            .iter()
            .filter(|(_, state)| state.ticket != 0 && !state.outstanding(now))
            .map(|(flight, state)| Token {
                flight: *flight,
                ticket: state.ticket,
            })
            .collect();
        for token in abandoned {
            if let Some(state) = self.flights.get_mut(&token.flight) {
                state.ticket = 0;
                state.deadline = None;
                state.carried.clear();
                state.completes.clear();
            }
            self.tidy_flight(token.flight);
            let keys: Vec<_> = self
                .subs
                .values()
                .filter(|entry| entry.flight == token.flight)
                .map(|entry| entry.key.clone())
                .collect();
            self.fenced.revisit.extend(keys);
            self.fenced.tokens.push(token);
        }
    }

    /// The live entry `snapshot` was taken from, whatever its expiry.
    pub(super) fn live_entry(
        &self,
        snapshot: &CovSubscriptionSnapshot,
    ) -> Option<&CovSubscriptionSnapshot> {
        if self.id != snapshot.table {
            return None;
        }
        self.subs.get(snapshot.key()).filter(|entry| {
            entry.generation == snapshot.generation && entry.flight == snapshot.flight
        })
    }

    /// Whether `ticket` is the outstanding report of `flight` at `now`.
    pub(super) fn flight_holds(&self, flight: u64, ticket: u64, now: Instant) -> bool {
        self.flights
            .get(&flight)
            .is_some_and(|state| state.ticket == ticket && state.outstanding(now))
    }

    /// Whether a fanout may report this ordinary or Single subscription at
    /// `now`: no confirmed report of it is outstanding or holding it off,
    /// and none was acknowledged after `snapshot` was taken, so the
    /// snapshot's baseline is still the live one. Always true for
    /// unconfirmed subscriptions.
    pub fn confirmed_idle(&self, snapshot: &CovSubscriptionSnapshot, now: Instant) -> bool {
        if !snapshot.issue_confirmed_notifications {
            return true;
        }
        // A snapshot that is no longer live is dropped by the liveness checks.
        self.live_entry(snapshot).is_none_or(|entry| {
            self.flight_idle(entry.flight, now)
                && entry.last_successful_ticket == snapshot.last_successful_ticket
        })
    }

    /// The same for a whole COV-multiple context: its flight is idle and no
    /// live snapshot among `snapshots` predates an acknowledgment. Always
    /// true for an unconfirmed context.
    pub fn context_idle(
        &self,
        context: &MultipleContextKey,
        snapshots: &[CovSubscriptionSnapshot],
        now: Instant,
    ) -> bool {
        if !context.confirmed {
            return true;
        }
        self.multiple_context_references(context)
            .next()
            .is_none_or(|entry| self.flight_idle(entry.flight, now))
            && snapshots
                .iter()
                .all(|snapshot| self.confirmed_idle(snapshot, now))
    }

    /// End of the hold-off a failed report put on a confirmed context, while
    /// it is in force at `now`. The timestamped backstop retries the context
    /// then.
    pub fn context_hold_until(
        &self,
        context: &MultipleContextKey,
        now: Instant,
    ) -> Option<Instant> {
        if !context.confirmed {
            return None;
        }
        let entry = self.multiple_context_references(context).next()?;
        self.flights.get(&entry.flight)?.holding_until(now)
    }

    /// Whether an idle context still owes the follow-up of a failed report,
    /// whose hold-off has passed by `now`. Taking it clears it: the caller
    /// fans every reference of the context out, since the failed report's
    /// changes may sit on objects other than the one that fanned out now.
    pub fn take_owed_context(&mut self, context: &MultipleContextKey, now: Instant) -> bool {
        if !context.confirmed {
            return false;
        }
        let Some(flight) = self
            .multiple_context_references(context)
            .next()
            .map(|entry| entry.flight)
        else {
            return false;
        };
        let Some(state) = self.flights.get_mut(&flight) else {
            return false;
        };
        let owed = !state.outstanding(now) && state.hold_until.is_some_and(|until| now >= until);
        if owed {
            state.hold_until = None;
            state.carried.clear();
            self.tidy_flight(flight);
        }
        owed
    }

    /// Mark the coordinate of one confirmed notification's references
    /// outstanding at `now`, under the report's single ticket, and keep the
    /// observation each one carries for the Ack to commit. The report ends
    /// itself at `now` plus the [deadline](FlightTiming::deadline) if no
    /// outcome arrives through [`delivered`](Self::delivered).
    pub fn begin_confirmed<S: Borrow<CovSubscriptionSnapshot>>(
        &mut self,
        completion: PreparedCovCompletion,
        references: impl IntoIterator<Item = (S, CovObservation)>,
        now: Instant,
    ) -> Result<Token, BeginRefusal> {
        let PreparedCovCompletion::Confirmed(ticket) = completion else {
            debug_assert!(false, "a confirmed report prepares a confirmed completion");
            return Err(BeginRefusal::Busy);
        };
        let mut flight: Option<u64> = None;
        let mut carried = BTreeSet::new();
        let mut completes = Vec::new();
        for (snapshot, observation) in references {
            let snapshot = snapshot.borrow();
            if !self.is_current(snapshot, now) {
                return Err(BeginRefusal::NotCurrent);
            }
            if !self.confirmed_idle(snapshot, now) {
                return Err(BeginRefusal::Busy);
            }
            let current = self.subs.get(snapshot.key()).expect("current entry").flight;
            // Live references of one report share their subscription's or
            // context's flight.
            debug_assert!(flight.is_none_or(|known| known == current));
            flight.get_or_insert(current);
            carried.insert(snapshot.key().clone());
            completes.push(Completion::new(snapshot, observation));
        }
        let Some(flight) = flight else {
            return Err(BeginRefusal::NotCurrent);
        };
        let deadline = now.saturating_add(self.timing.deadline());
        let state = self.flights.entry(flight).or_default();
        // A report still marked here passed its deadline, or the coordinate
        // would be busy. Its worker may yet be alive, stalled in a send, so
        // it is fenced before the new report takes the mark: one report at
        // a time per coordinate.
        if state.ticket != 0 {
            self.fenced.tokens.push(Token {
                flight,
                ticket: state.ticket,
            });
        }
        state.ticket = ticket.get();
        state.deadline = Some(deadline);
        state.carried = carried;
        state.completes = completes;
        self.due_by(Some(deadline));
        Ok(Token {
            flight,
            ticket: ticket.get(),
        })
    }

    /// End the report `token` names, as `outcome` says, at `now`. A report
    /// that was fenced, removed with its coordinate, or that passed its
    /// deadline is no longer known, and its outcome changes nothing.
    ///
    /// An unconfirmed report that was [`Sent`](Delivery::Sent) commits its
    /// observation, unless a later report of its subscription completed
    /// first. An [`Acknowledged`](Delivery::Acknowledged) confirmed report
    /// commits the observations it carried, and returns the references owed
    /// a fresh look: changes to them were held while it was outstanding.
    /// They are those it completed or, for a COV-multiple context, every
    /// reference of the context. Every other outcome returns none.
    pub fn delivered(
        &mut self,
        token: Token,
        outcome: Delivery,
        now: Instant,
    ) -> Vec<CovSubscriptionKey> {
        if self.deliver_unconfirmed(token, outcome, now) {
            return Vec::new();
        }
        let hold_off = self.timing.hold_off();
        let Some(state) = self.flights.get_mut(&token.flight) else {
            return Vec::new();
        };
        if state.ticket != token.ticket || !state.outstanding(now) || outcome == Delivery::Sent {
            return Vec::new();
        }
        let completes = core::mem::take(&mut state.completes);
        let mut owed = Vec::new();
        if outcome == Delivery::Acknowledged {
            // Committed while the report still holds the coordinate.
            let context = completes
                .first()
                .and_then(|completion| completion.key().multiple_context().cloned());
            for completion in completes {
                let key = completion.key().clone();
                if self.complete(token, true, completion, now) {
                    owed.push(key);
                }
            }
            if let Some(context) = context {
                owed = self
                    .multiple_context_references(&context)
                    .map(|entry| entry.key().clone())
                    .collect();
            }
        }
        let state = self.flights.get_mut(&token.flight).expect("checked above");
        state.ticket = 0;
        state.deadline = None;
        match outcome {
            Delivery::Failed => state.hold_until = Some(now.saturating_add(hold_off)),
            Delivery::Acknowledged | Delivery::NotSent | Delivery::Sent => {
                state.hold_until = None;
                state.carried.clear();
            }
        }
        self.tidy_flight(token.flight);
        owed
    }

    /// Choose the flight a Multiple context keeps after this admission. A
    /// route change always starts a new one, as does a re-subscription
    /// listing references while the context is busy. Returns it, with the
    /// flight it replaces, if any.
    pub(super) fn context_flight(
        &mut self,
        context: &MultipleContextKey,
        route: &SubscriberEndpoint,
        lists_references: bool,
        now: Instant,
    ) -> (u64, Option<u64>) {
        let current = self
            .multiple_context_references(context)
            .next()
            .map(|entry| (entry.endpoint() == *route, entry.flight));
        match current {
            Some((true, flight)) if !lists_references || self.flight_idle(flight, now) => {
                (flight, None)
            }
            Some((_, replaced)) => (self.new_flight(), Some(replaced)),
            None => (self.new_flight(), None),
        }
    }

    /// Fence a flight a fresh one replaced, once the references `listed` by
    /// the replacing request are published. If its report was outstanding,
    /// or failed and still owed a follow-up, nothing on the old flight can
    /// finish the job, yet the report may have reached the subscriber: every
    /// reference, kept or relisted, is evaluated again, and each kept
    /// untimestamped one the report carried forgets its baseline first, so
    /// the follow-up reports its current value (#923). Listed references
    /// have no baseline either, so whichever of this follow-up and the
    /// initial report goes first carries them as first reports, and the
    /// other finds the context busy.
    pub(super) fn fence_context_flight(
        &mut self,
        context: &MultipleContextKey,
        replaced: u64,
        listed: &BTreeSet<CovSubscriptionKey>,
        now: Instant,
    ) {
        let carried = self
            .flights
            .get(&replaced)
            .and_then(|state| state.owed_carried(now))
            .cloned();
        if let Some(carried) = carried {
            debug_assert!(context.confirmed, "only confirmed contexts owe follow-ups");
            // Measured against a baseline the subscriber may have left behind,
            // a carried value that went back to it would never be reported again.
            for entry in self.subs.values_mut() {
                if entry.key.multiple_context() == Some(context)
                    && !entry.timestamped
                    && carried.contains(&entry.key)
                    && !listed.contains(&entry.key)
                {
                    entry.subscription.last_notified_observation = None;
                }
            }
            let keys: Vec<_> = self
                .multiple_context_references(context)
                .map(|entry| entry.key().clone())
                .collect();
            self.fenced.revisit.extend(keys);
        }
        self.fence_flight(replaced);
    }
}

#[cfg(test)]
mod tests;
