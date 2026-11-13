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
/// builder-config interval means no mid-run rescale exists. The core takes
/// the interval as a `u32`; one above `u32::MAX` goes in as `u32::MAX`,
/// which rounds every delay to the same passes.
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
    let evaluation = evaluate_event_enrollments_for_delivery(db, core_interval(interval_secs));
    super::log_suppressed_deliveries(&evaluation.suppressed);
    evaluation.report
}

/// The server's interval in seconds as the core takes it, saturating at
/// `u32::MAX`. Saturating changes no delay: every Time_Delay is at most
/// `u32::MAX` seconds, so above that any non-zero delay is one pass either
/// way, and zero is none.
pub(crate) fn core_interval(interval_secs: u64) -> u32 {
    u32::try_from(interval_secs).unwrap_or(u32::MAX)
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
