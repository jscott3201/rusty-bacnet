//! Local completion order for whole observations: unconfirmed reports complete
//! once sent (#826), confirmed ones once acknowledged (#896).
use bacnet_device_core::cov::CovObservation;
use bacnet_time::Instant;

use super::{CovSubscriptionSnapshot, CovTable};

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
        if !self.is_current(snapshot, now) {
            return false;
        }
        let flight = self.subs.get(snapshot.key()).expect("current entry").flight;
        let ticket = match (snapshot.issue_confirmed_notifications, completion) {
            // Every reference of one report shares its ticket, so the mark stays
            // until the flight ends after the last of them.
            (true, PreparedCovCompletion::Confirmed(ticket))
                if self.flight_holds(flight, ticket.get(), now) =>
            {
                ticket
            }
            (false, PreparedCovCompletion::Unconfirmed(ticket)) => ticket,
            _ => return false,
        };
        let entry = self.subs.get_mut(snapshot.key()).expect("current entry");
        if ticket.0 <= entry.last_successful_ticket {
            return false;
        }
        entry.last_successful_ticket = ticket.0;
        entry.subscription.last_notified_observation = Some(value);
        true
    }
}

#[cfg(test)]
mod tests;
