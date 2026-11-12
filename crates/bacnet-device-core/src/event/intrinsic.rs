//! Intrinsic reporting's driver: the evaluation after a write or a local
//! change, the sweep that fires the transitions whose Time_Delay has ended
//! and turns a Reliability change into a FAULT transition (Clause 13.2.2),
//! and [`next_due`], the instant the next delay ends.

use alloc::vec::Vec;
use bacnet_objects::database::ObjectDatabase;
use bacnet_time::Instant;
use bacnet_types::primitives::ObjectIdentifier;

use super::{CommittedIntrinsicTransition, IntrinsicCommitRefusal, commit_intrinsic_transition};

/// What one object's intrinsic evaluation committed, or why it committed
/// nothing to distribute.
pub type IntrinsicCommit = Result<CommittedIntrinsicTransition, IntrinsicCommitRefusal>;

/// Evaluate intrinsic reporting on one object a write or the host changed,
/// and commit what it proposes.
///
/// The detector fires at once only when `Time_Delay` is 0. For a nonzero
/// delay it seeds a pending transition due `Time_Delay` seconds after
/// `now` and proposes nothing, so this returns `None`; the [`sweep`] at or
/// after its due instant ([`next_due`]) fires it. A committed transition changes
/// Status_Flags, so the caller owes the object a COV look afterwards.
pub fn after_write(
    db: &mut ObjectDatabase,
    oid: &ObjectIdentifier,
    now: Instant,
) -> Option<IntrinsicCommit> {
    // Event_Algorithm_Inhibit follows its reference as of now (#1329).
    db.follow_event_algorithm_inhibit(oid);
    let outcome = db
        .get_mut(oid)
        .and_then(|object| object.event_reporting_mut())
        .and_then(|reporting| reporting.evaluate_intrinsic_reporting(now))?;
    Some(commit_intrinsic_transition(db, oid, outcome))
}

/// One pass over every object at `now`: follow each Event_Algorithm_Inhibit
/// reference, fire each pending transition due at or before `now`, seed one
/// for a condition no write reported, and commit each transition proposed,
/// in the database's order. A host runs it periodically (the server once a
/// second) and at [`next_due`].
///
/// `before_commit` runs for each object that proposes a transition, after
/// its detector ran and before the proposal commits, under the same borrow:
/// it sees the object's Event_State, Status_Flags and history as they were
/// before the commit. That order is part of the contract: a host keeps what
/// it must from before the change there (the server snapshots Life Safety
/// state and owes the object a COV look). Event_Enable gates distribution
/// only (Clause 13.2.5), so every proposal commits; the caller sends those
/// whose `distribute` is set and whose Event Values projected.
pub fn sweep(
    db: &mut ObjectDatabase,
    now: Instant,
    mut before_commit: impl FnMut(&ObjectDatabase, ObjectIdentifier),
) -> Vec<(ObjectIdentifier, IntrinsicCommit)> {
    let mut committed = Vec::new();
    for oid in db.list_objects() {
        // Event_Algorithm_Inhibit follows its reference once a sweep at
        // least, under this same borrow (#1329).
        db.follow_event_algorithm_inhibit(&oid);
        let Some(outcome) = db
            .get_mut(&oid)
            .and_then(|object| object.event_reporting_mut())
            .and_then(|reporting| reporting.tick_intrinsic_reporting(now))
        else {
            continue;
        };
        before_commit(db, oid);
        committed.push((oid, commit_intrinsic_transition(db, &oid, outcome)));
    }
    committed
}

/// The earliest instant after `after` at which an object's pending
/// transition is due: when a host should run [`sweep`] next, if it sweeps no
/// sooner anyway. Pass the `now` of the sweep just run. `None` with nothing
/// due after it.
///
/// A due instant at or before `after` is left out. The sweep at `after`
/// has seen it, and it is still pending only because it was seeded there
/// (a Time_Delay of 0 the sweep noticed, which fires at the next sweep) or
/// because its commit failed; either way the host's own cadence retries
/// it. Returning it would keep a host from waking for any later due
/// instant, or wake it over and over for one it can't fire.
pub fn next_due(db: &ObjectDatabase, after: Instant) -> Option<Instant> {
    bacnet_time::earliest(db.iter_objects().map(|(_, object)| {
        object
            .event_reporting()
            .and_then(|reporting| reporting.next_due())
            .filter(|due| *due > after)
    }))
}

#[cfg(test)]
#[path = "intrinsic_tests.rs"]
mod tests;
