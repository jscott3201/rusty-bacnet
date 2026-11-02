//! The Staging capability: the target writes a Staging object's stage
//! changes queue.

use crate::prelude::*;
use crate::schedule::ScheduleWrite;
use crate::staging::StagingWritePlan;

/// A Staging object's queued target writes and their outcome.
///
/// The object returns itself from
/// [`BACnetObject::staging_source`](crate::traits::BACnetObject::staging_source).
/// The bundled server takes a plan while it holds the database's write
/// guard, makes the plan's writes after releasing it, checking the
/// generation again under the guard that applies each one, and reports how
/// the plan ended. Not public API yet (#2179).
#[doc(hidden)]
pub trait StagingSource {
    /// Take the target writes the last stage change queued, if any.
    fn take_staging_write_plan(&mut self) -> Option<StagingWritePlan>;

    /// The current stage-change generation. A plan whose generation no
    /// longer matches is stale and makes no further writes.
    fn staging_generation(&self) -> u64;

    /// Commit how the plan of `generation` fared to Reliability. Returns
    /// whether readable state changed; a stale generation is ignored, so
    /// older work cannot fault a newer stage change.
    fn complete_staging_write_plan(&mut self, generation: u64, success: bool) -> bool;

    /// The NULLs that give up the slots a change of the object's targets or
    /// priority left behind (#2289), each taken once. The bundled server
    /// writes them under the guard of each target write the object's plans
    /// make, before that write, even when the plan has gone stale, so one
    /// lands before any later write of the object to the same slot. Each is
    /// a [`relinquish`](ScheduleWrite::relinquish) write, checked for
    /// ownership as on removal. An object out of service owes none until it
    /// returns. The default owes none.
    fn take_owed_relinquishes(&mut self) -> Vec<ScheduleWrite> {
        Vec::new()
    }

    /// The NULLs that give up every priority slot the object's stage writes
    /// filled, and those still owed, for its removal from the database
    /// (#2265). Each is a
    /// [`relinquish`](ScheduleWrite::relinquish) write, so the bundled
    /// server sends it only to a slot whose Value_Source still names this
    /// object. Each is taken once. The default owes none.
    fn take_relinquish_writes(&mut self) -> Vec<ScheduleWrite> {
        Vec::new()
    }
}
