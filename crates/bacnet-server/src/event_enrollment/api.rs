use bacnet_objects::database::ObjectDatabase;

use super::{
    EventEnrollmentEvaluationReport, EventEnrollmentTransition,
    evaluate_event_enrollments_for_delivery,
};

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
    let evaluation = evaluate_event_enrollments_for_delivery(db, interval_secs);
    super::log_suppressed_deliveries(&evaluation.suppressed);
    evaluation.report
}
