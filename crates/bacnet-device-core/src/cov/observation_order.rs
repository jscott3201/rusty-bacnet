//! Local completion order for whole observations: unconfirmed reports complete
//! once sent (#826), confirmed ones once acknowledged (#896).
use crate::cov::{CovObservation, CovSubscriptionKey, CovTimeRemaining, Token};
use bacnet_time::Instant;

use super::{CovSubscriptionSnapshot, CovTable};

/// The observation one reference of a report carries, with the entry it was
/// captured from: the report's end commits it there, if that entry is still
/// live.
#[derive(Debug, Clone)]
pub(super) struct Completion {
    key: CovSubscriptionKey,
    generation: u64,
    observation: CovObservation,
}

impl Completion {
    pub(super) fn new(snapshot: &CovSubscriptionSnapshot, observation: CovObservation) -> Self {
        Self {
            key: snapshot.key.clone(),
            generation: snapshot.generation,
            observation,
        }
    }

    pub(super) fn key(&self) -> &CovSubscriptionKey {
        &self.key
    }
}

/// An unconfirmed report handed to the host, until its send ends.
#[derive(Debug, Clone)]
pub(super) struct Pending {
    token: Token,
    completion: Completion,
}

/// A completion ticket: its issue order. Issued tickets start at one; zero
/// is every entry's empty value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObservationTicket(u64);

impl ObservationTicket {
    /// Issue order: tickets start at one.
    pub(super) fn get(self) -> u64 {
        self.0
    }
}

/// Only a complete eligible preparation may obtain this internal completion authority.
#[derive(Debug, Clone, Copy)]
pub enum PreparedCovCompletion {
    /// Completes on the subscriber's Ack, and only while this ticket is the
    /// coordinate's outstanding report (see [`CovTable::begin_confirmed`]).
    Confirmed(ObservationTicket),
    /// Completes once the transport accepts the send, if no newer send did.
    Unconfirmed(ObservationTicket),
}

impl PreparedCovCompletion {
    /// Issue order of this completion's ticket.
    pub fn ticket(self) -> ObservationTicket {
        match self {
            Self::Confirmed(ticket) | Self::Unconfirmed(ticket) => ticket,
        }
    }
}

impl CovTable {
    /// Issue the next completion ticket for `snapshot`'s observation. Call it
    /// after the complete capture and qualification, still under the guard
    /// the observation was read under, so tickets follow the order the
    /// observations were taken in. `None` once the table's tickets are spent.
    pub fn prepare_completion(
        &mut self,
        snapshot: &CovSubscriptionSnapshot,
    ) -> Option<PreparedCovCompletion> {
        let ticket = ObservationTicket(self.issued.checked_add(1)?);
        self.issued = ticket.0;
        Some(if snapshot.issue_confirmed_notifications {
            PreparedCovCompletion::Confirmed(ticket)
        } else {
            PreparedCovCompletion::Unconfirmed(ticket)
        })
    }

    /// Commit a complete observation and its successful marker together.
    /// Issued-but-failed or cancelled work never advances this marker. A
    /// confirmed completion must carry its coordinate's outstanding report
    /// ticket; an older or replaced report changes nothing. The caller ends
    /// the flight once every reference is complete.
    pub fn complete_observation(
        &mut self,
        snapshot: &CovSubscriptionSnapshot,
        completion: PreparedCovCompletion,
        value: CovObservation,
        now: Instant,
    ) -> bool {
        if self.id != snapshot.table {
            return false;
        }
        let (confirmed, ticket) = match completion {
            PreparedCovCompletion::Confirmed(ticket) => (true, ticket),
            PreparedCovCompletion::Unconfirmed(ticket) => (false, ticket),
        };
        if confirmed != snapshot.issue_confirmed_notifications {
            return false;
        }
        let token = Token {
            flight: snapshot.flight,
            ticket: ticket.get(),
        };
        self.complete(token, confirmed, Completion::new(snapshot, value), now)
    }

    /// Commit `completion` under `token` at `now`, if the entry it was
    /// captured from is live there (same generation and flight, lifetime
    /// left), a confirmed report still holds the coordinate, and no later
    /// ticket of the entry completed first.
    pub(super) fn complete(
        &mut self,
        token: Token,
        confirmed: bool,
        completion: Completion,
        now: Instant,
    ) -> bool {
        if !self.may_complete(token, &completion, now) {
            return false;
        }
        // Every reference of one report shares its ticket, so the mark stays
        // until the flight ends after the last of them.
        if confirmed && !self.flight_holds(token.flight, token.ticket, now) {
            return false;
        }
        let entry = self.subs.get_mut(&completion.key).expect("live entry");
        entry.last_successful_ticket = token.ticket;
        entry.subscription.last_notified_observation = Some(completion.observation);
        true
    }

    /// Whether `completion`'s entry is live at `now` and has completed no
    /// ticket from `token` on.
    fn may_complete(&self, token: Token, completion: &Completion, now: Instant) -> bool {
        self.subs.get(&completion.key).is_some_and(|entry| {
            entry.generation == completion.generation
                && entry.flight == token.flight
                && token.ticket > entry.last_successful_ticket
                && CovTimeRemaining::at(entry.expires_at, now)
                    .wire_seconds()
                    .is_some()
        })
    }

    /// Let go of the unconfirmed reports that can no longer complete at
    /// `now`: their entry gone, replaced, expired, or completed by a later
    /// report. A host reports every send's end, so the list holds the
    /// reports whose sends are under way, and besides them only those a
    /// host dropped between evaluation and the send (a stop, a panic). Such
    /// a report goes at the first evaluation after a later report of its
    /// subscription completes, or the subscription ends, renews or expires,
    /// or with the table.
    pub(super) fn prune_pending(&mut self, now: Instant) {
        if self.pending.is_empty() {
            return;
        }
        let mut pending = core::mem::take(&mut self.pending);
        pending.retain(|report| self.may_complete(report.token, &report.completion, now));
        self.pending = pending;
    }

    /// Hand an unconfirmed report of `snapshot` to the host under `ticket`:
    /// its token commits `observation` once [`delivered`](Self::delivered)
    /// says it was sent.
    pub(super) fn begin_unconfirmed(
        &mut self,
        ticket: ObservationTicket,
        snapshot: &CovSubscriptionSnapshot,
        observation: CovObservation,
    ) -> Token {
        let token = Token {
            flight: snapshot.flight,
            ticket: ticket.get(),
        };
        self.pending.push(Pending {
            token,
            completion: Completion::new(snapshot, observation),
        });
        token
    }

    /// End an unconfirmed report `token` names, committing its observation
    /// if it was sent. False when no unconfirmed report has that token.
    pub(super) fn deliver_unconfirmed(
        &mut self,
        token: Token,
        outcome: super::Delivery,
        now: Instant,
    ) -> bool {
        // Usually nothing is pending: a confirmed report's end, or the last
        // send of a fanout, finds out at once.
        if self.pending.is_empty() {
            return false;
        }
        let Some(at) = self.pending.iter().position(|report| report.token == token) else {
            return false;
        };
        let report = self.pending.swap_remove(at);
        if outcome == super::Delivery::Sent {
            self.complete(token, false, report.completion, now);
        }
        true
    }
}

#[cfg(test)]
mod tests;
