//! The Staging capability: the target writes a Staging object's stage
//! changes queue.

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
}
