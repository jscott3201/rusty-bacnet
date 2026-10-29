//! Local completion order for whole observations: unconfirmed reports complete
//! once sent (#826), confirmed ones once acknowledged (#896).
use super::{CovObservation, CovSubscriptionSnapshot, CovSubscriptionTable};
use std::sync::atomic::{AtomicU64, Ordering};

/// Existing table identity also owns one checked counter. No extra per-reference Arc.
#[derive(Debug, Default)]
pub(super) struct ObservationOwner {
    issued: AtomicU64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ObservationTicket(u64);

impl ObservationTicket {
    /// Issued tickets start at one; zero is every marker's empty value.
    pub(super) fn get(self) -> u64 {
        self.0
    }
}

/// Only a complete eligible preparation may obtain this internal completion authority.
#[derive(Debug, Clone, Copy)]
pub(crate) enum PreparedCovCompletion {
    /// Completes on the subscriber's Ack, and only while this ticket is the
    /// coordinate's outstanding report (see [`super::confirmed`]).
    Confirmed(ObservationTicket),
    /// Completes once the transport accepts the send, if no newer send did.
    Unconfirmed(ObservationTicket),
}
impl PreparedCovCompletion {
    /// Issue order of this completion's ticket.
    pub(crate) fn ticket(self) -> ObservationTicket {
        match self {
            Self::Confirmed(ticket) | Self::Unconfirmed(ticket) => ticket,
        }
    }
}
impl CovSubscriptionSnapshot {
    /// Call synchronously after complete capture/qualification, before any later await.
    /// For shared ordinary capture, prepare each applicable reference under that view.
    pub(crate) fn prepare_completion(&self) -> Option<PreparedCovCompletion> {
        let ticket = self
            .owner
            .issued
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .ok()
            .map(|n| ObservationTicket(n + 1))?;
        Some(if self.issue_confirmed_notifications {
            PreparedCovCompletion::Confirmed(ticket)
        } else {
            PreparedCovCompletion::Unconfirmed(ticket)
        })
    }
}
impl CovSubscriptionTable {
    /// Commit a complete observation and successful marker together under the caller's
    /// table write guard. Issued-but-failed/canceled work never advances this marker.
    /// A confirmed completion must carry its coordinate's outstanding report
    /// ticket; an older or replaced report changes nothing. The caller ends the
    /// flight once every reference is complete.
    pub(crate) fn complete_observation(
        &mut self,
        snapshot: &CovSubscriptionSnapshot,
        completion: PreparedCovCompletion,
        value: CovObservation,
    ) -> bool {
        if !self.is_current(snapshot) {
            return false;
        }
        let entry = self.subs.get_mut(snapshot.key()).expect("current entry");
        let ticket = match (snapshot.issue_confirmed_notifications, completion) {
            // Every reference of one report shares its ticket, so the mark stays
            // until the flight ends after the last of them.
            (true, PreparedCovCompletion::Confirmed(ticket)) if entry.flight.holds(ticket) => {
                ticket
            }
            (false, PreparedCovCompletion::Unconfirmed(ticket)) => ticket,
            _ => return false,
        };
        if ticket.0 <= entry.last_successful_ticket {
            return false;
        }
        entry.last_successful_ticket = ticket.0;
        entry.subscription.last_notified_observation = Some(value);
        true
    }
    /// Test setup still uses real reservation and completion, not a baseline
    /// bypass. A confirmed reference goes through its own outstanding report.
    #[cfg(test)]
    pub(crate) fn complete_for_test(
        &mut self,
        snapshot: &CovSubscriptionSnapshot,
        value: CovObservation,
    ) -> bool {
        let Some(completion) = snapshot.prepare_completion() else {
            return false;
        };
        let _flight = if snapshot.issue_confirmed_notifications {
            let Ok(flight) = self.begin_confirmed(completion, [snapshot]) else {
                return false;
            };
            Some(flight)
        } else {
            None
        };
        self.complete_observation(snapshot, completion, value)
    }
}

#[cfg(test)]
mod tests;
