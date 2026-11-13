//! Event Enrollment algorithmic evaluation. The pass and its commit are
//! bacnet-device-core's (`event::enrollment`, #2319); the server drives the
//! pass on its task, logs what it reports, and keeps the public wrappers.

mod api;
mod log;

pub use api::{evaluate_event_enrollments, evaluate_event_enrollments_report};
pub(crate) use bacnet_device_core::event::enrollment::{
    CommittedEventEnrollmentDelivery, CommittedEventEnrollmentResult, SuppressedDelivery,
    evaluate_event_enrollments_for_delivery,
};
pub use bacnet_device_core::event::enrollment::{
    EventEnrollmentEvaluationDiagnostic, EventEnrollmentEvaluationOutcome,
    EventEnrollmentEvaluationReport, EventEnrollmentEvaluationStage,
    EventEnrollmentReliabilityCause, EventEnrollmentReliabilityResult, EventEnrollmentTransition,
};
pub(crate) use log::{log_evaluation_report, log_suppressed_deliveries};
