use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::EventStateChange;
use bacnet_types::enums::EventType;
use bacnet_types::primitives::ObjectIdentifier;

use super::{evaluate_event_enrollments_for_delivery, EventEnrollmentEvaluationReport};

/// A state transition detected during event enrollment evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct EventEnrollmentTransition {
    /// The EventEnrollment object that detected the transition.
    pub enrollment_oid: ObjectIdentifier,
    /// The monitored object whose property triggered the transition.
    pub monitored_oid: ObjectIdentifier,
    /// The detected state change. `from == to` is a genuine same-state
    /// transition emitted by applicable event algorithms.
    pub change: EventStateChange,
    /// The event type that was evaluated.
    pub event_type: EventType,
    /// Whether `Event_Enable` permits distributing a notification for this
    /// transition. The transition is reported and `Event_State` persisted
    /// either way; a cleared bit suppresses only the outbound notification.
    pub distribute: bool,
}

/// Evaluate all EventEnrollment objects in the database.
///
/// For each active enrollment, reads the monitored property, evaluates the
/// configured algorithm, applies the Time_Delay / Time_Delay_Normal
/// countdown (seconds, converted by the module's delay helper), executes the
/// transition actions for every indicated transition that fires — same-state
/// included — and returns the fired transitions.
///
/// This convenience returns only event transitions. Use
/// [`evaluate_event_enrollments_report`] for committed Reliability results and
/// observation/commit diagnostics from the same evaluation pass.
///
/// `interval_secs` is the driving task's evaluation period in wall-clock
/// seconds; the lifecycle passes its (clamped to >= 1)
/// `event_enrollment_interval_secs`. The conversion is never-fire-early, and
/// the pending countdown retains no residual seconds: in-memory state plus
/// builder-config interval means no mid-run rescale exists.
pub fn evaluate_event_enrollments(
    db: &mut ObjectDatabase,
    interval_secs: u64,
) -> Vec<EventEnrollmentTransition> {
    evaluate_event_enrollments_report(db, interval_secs).transitions
}

/// Evaluate all EventEnrollment objects and return the complete report.
///
/// Includes committed Reliability results and typed evaluation, observation,
/// and commit diagnostics. Only results whose complete object-owned commit
/// succeeds appear in `transitions` or `reliability_results`.
pub fn evaluate_event_enrollments_report(
    db: &mut ObjectDatabase,
    interval_secs: u64,
) -> EventEnrollmentEvaluationReport {
    evaluate_event_enrollments_for_delivery(db, interval_secs).report
}
