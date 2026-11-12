//! Intrinsic reporting's driver: the evaluation after a write or a local
//! change, and the periodic sweep that advances Time_Delay and turns a
//! Reliability change into a FAULT transition (Clause 13.2.2).

use alloc::vec::Vec;
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::primitives::ObjectIdentifier;

use super::{CommittedIntrinsicTransition, IntrinsicCommitRefusal, commit_intrinsic_transition};

/// What one object's intrinsic evaluation committed, or why it committed
/// nothing to distribute.
pub type IntrinsicCommit = Result<CommittedIntrinsicTransition, IntrinsicCommitRefusal>;

/// Evaluate intrinsic reporting on one object a write or the host changed,
/// and commit what it proposes.
///
/// The detector fires at once only when `Time_Delay` is 0. For a nonzero
/// delay the probe seeds a pending transition and proposes nothing, so this
/// returns `None`, and [`sweep`] advances the countdown. A committed
/// transition changes Status_Flags, so the caller owes the object a COV
/// look afterwards.
pub fn after_write(db: &mut ObjectDatabase, oid: &ObjectIdentifier) -> Option<IntrinsicCommit> {
    // Event_Algorithm_Inhibit follows its reference as of now (#1329).
    db.follow_event_algorithm_inhibit(oid);
    let outcome = db
        .get_mut(oid)
        .and_then(|object| object.event_reporting_mut())
        .and_then(|reporting| reporting.evaluate_intrinsic_reporting())?;
    Some(commit_intrinsic_transition(db, oid, outcome))
}

/// One periodic pass over every object: follow each Event_Algorithm_Inhibit
/// reference, advance each detector's countdown and commit each transition
/// it proposes, in the database's order.
///
/// `before_commit` runs for each object just before its proposal commits,
/// under the same borrow, so a host can keep what it must from before the
/// change (the server snapshots Life Safety state and owes the object a COV
/// look). Event_Enable gates distribution only (Clause 12.12), so every
/// proposal commits; the caller sends those whose `distribute` is set and
/// whose Event Values projected.
pub fn sweep(
    db: &mut ObjectDatabase,
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
            .and_then(|reporting| reporting.tick_intrinsic_reporting())
        else {
            continue;
        };
        before_commit(db, oid);
        committed.push((oid, commit_intrinsic_transition(db, &oid, outcome)));
    }
    committed
}
