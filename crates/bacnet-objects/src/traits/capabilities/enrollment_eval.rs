//! The Event Enrollment evaluation capability: the private state the
//! server's algorithmic evaluator keeps on an enrollment between passes,
//! and the commit that joins Reliability to an event transition.

use bacnet_types::error::Error;

use super::not_supported;
use crate::event::EventTransitionCommitError;
use crate::event_enrollment::{
    EventEnrollmentEvalState, EventEnrollmentMonitoredSource, EventEnrollmentReliabilityCommit,
};

/// An Event Enrollment that the host's evaluator drives.
///
/// The object returns itself from
/// [`BACnetObject::enrollment_eval_mut`](crate::traits::BACnetObject::enrollment_eval_mut),
/// and from [`enrollment_eval`](crate::traits::BACnetObject::enrollment_eval)
/// for the snapshots the bundled server takes under a shared database guard
/// before it evaluates. An object that lends one lends the other, and lending
/// it is what says the object keeps evaluation state.
///
/// None of this is a BACnet property, so it bypasses the network property
/// model: the pending countdown, the CHANGE_OF_VALUE baseline and the last
/// OFFNORMAL value are local matters (Clause 13.3). Not public API yet
/// (#2179).
#[doc(hidden)]
pub trait EnrollmentEval {
    /// Snapshot the evaluation state: the pending (delayed) transition, the
    /// CHANGE_OF_VALUE baseline (Clause 13.3.3) and the monitored value
    /// behind the latest OFFNORMAL transition.
    fn eval_state(&self) -> EventEnrollmentEvalState;

    /// Store the evaluation state the evaluator derived from a prior
    /// snapshot and this pass.
    ///
    /// Implementations keep Clause 13.2.2.1 by construction: no transition
    /// while Event_Detection_Enable is FALSE, so a write arriving then is
    /// refused rather than queued.
    fn set_eval_state(&mut self, state: EventEnrollmentEvalState) -> Result<(), Error>;

    /// Snapshot the monitored source that owns the evaluation state.
    ///
    /// The outer `Option` says whether the object keeps the source itself;
    /// the inner one is empty before a source is established. With the
    /// default, `None`, the bundled server keeps the source in its object
    /// database instead.
    fn eval_source(&self) -> Option<Option<EventEnrollmentMonitoredSource>> {
        None
    }

    /// Store or clear the monitored source that owns the evaluation state.
    /// The default, for an object that keeps none, refuses.
    fn set_eval_source(
        &mut self,
        _source: Option<EventEnrollmentMonitoredSource>,
    ) -> Result<(), Error> {
        Err(not_supported())
    }

    /// Commit Reliability and, when the commit carries one, its event
    /// transition atomically: `Reliability` with `Event_State`,
    /// `Acked_Transitions` and `Event_Time_Stamps`.
    ///
    /// Implementations stage every value before assigning any field and
    /// leave them all unchanged on error. The default fails closed with
    /// [`EventTransitionCommitError::Unsupported`].
    fn commit_reliability(
        &mut self,
        _commit: EventEnrollmentReliabilityCommit,
    ) -> Result<(), EventTransitionCommitError> {
        Err(EventTransitionCommitError::Unsupported)
    }
}

/// The two accessors of an object that lends itself as its
/// [`EnrollmentEval`], for inside its `BACnetObject` impl.
macro_rules! lend_enrollment_eval {
    () => {
        fn enrollment_eval(&self) -> Option<&dyn $crate::traits::EnrollmentEval> {
            Some(self)
        }

        fn enrollment_eval_mut(&mut self) -> Option<&mut dyn $crate::traits::EnrollmentEval> {
            Some(self)
        }
    };
}

pub(crate) use lend_enrollment_eval;
