//! Event Enrollment algorithmic evaluation. The pass and its commit are
//! bacnet-device-core's (`event::enrollment`, #2319), and so are their docs:
//! the algorithms run, the time delays, the transition actions and the
//! inhibit. The server drives the pass on its task, logs what it reports,
//! and keeps the public wrappers.

mod api;
mod log;

pub(crate) use api::core_interval;
pub use api::{evaluate_event_enrollments, evaluate_event_enrollments_report};
pub use bacnet_device_core::event::enrollment::{
    EventEnrollmentEvaluationDiagnostic, EventEnrollmentEvaluationOutcome,
    EventEnrollmentEvaluationReport, EventEnrollmentEvaluationStage,
    EventEnrollmentReliabilityCause, EventEnrollmentReliabilityResult, EventEnrollmentTransition,
};
pub(crate) use bacnet_device_core::event::enrollment::{
    SuppressedDelivery, SuppressionCause, evaluate_event_enrollments_for_delivery,
};
pub(crate) use log::{log_evaluation_report, log_suppressed_deliveries};
