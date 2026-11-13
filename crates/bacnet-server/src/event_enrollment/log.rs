//! The server's log lines for an Event Enrollment pass: what the pass
//! reports, logged where the server drives it.

use super::{EventEnrollmentEvaluationReport, SuppressedDelivery, SuppressionCause};

pub(crate) fn log_evaluation_report(report: &EventEnrollmentEvaluationReport) {
    for result in &report.reliability_results {
        tracing::debug!(
            enrollment = %result.enrollment_oid,
            monitored = ?result.monitored_oid,
            previous = ?result.previous_reliability,
            new = ?result.new_reliability,
            state_change = ?result.state_change,
            distribute = result.distribute,
            cause = ?result.cause,
            "Event enrollment: reliability committed"
        );
    }
    for diagnostic in report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.outcome.is_failure())
    {
        tracing::warn!(
            enrollment = %diagnostic.enrollment_oid,
            stage = ?diagnostic.stage,
            outcome = ?diagnostic.outcome,
            "Event enrollment: internal commit failed"
        );
    }
}

/// Log each committed transition whose notification can't be built: a
/// refused payload as the commit once logged it, and a history that
/// doesn't read back as the intrinsic path logs it.
pub(crate) fn log_suppressed_deliveries(suppressed: &[SuppressedDelivery]) {
    for delivery in suppressed {
        match (delivery.cause, delivery.event_type) {
            (SuppressionCause::History(coordinate), _) => tracing::debug!(
                enrollment = %delivery.enrollment_oid,
                ?coordinate,
                "Committed Event Enrollment history projection rejected; suppressing distribution"
            ),
            (SuppressionCause::Payload, None) => tracing::debug!(
                enrollment = %delivery.enrollment_oid,
                "Committed Event Enrollment reliability payload rejected; suppressing distribution"
            ),
            (SuppressionCause::Payload, Some(event_type)) => tracing::debug!(
                enrollment = %delivery.enrollment_oid,
                ?event_type,
                "Committed Event Enrollment payload rejected; suppressing distribution"
            ),
        }
    }
}

#[cfg(test)]
#[path = "log_tests.rs"]
mod tests;
