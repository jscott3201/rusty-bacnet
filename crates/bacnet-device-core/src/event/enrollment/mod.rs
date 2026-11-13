//! Event Enrollment's algorithms (#2319, behind `event-enrollment`): the
//! event algorithms an Event Enrollment object evaluates its monitored
//! value with, its fault algorithms, and the reads of its configuration and
//! monitored reference, moved from `bacnet-server`'s evaluator unchanged.
//! The pass that drives them is still the server's.
//!
//! The modules are internal until the pass moves here (#2319 PR 2a): they
//! are public for `bacnet-server` alone, hidden from the documentation, and
//! nothing in them is for a host to call.

#[doc(hidden)]
pub mod algorithms;
#[doc(hidden)]
pub mod fault;
#[doc(hidden)]
pub mod reference;

#[cfg(feature = "std")]
mod pass;
#[cfg(feature = "std")]
pub use pass::{
    CommittedEventEnrollmentDelivery, CommittedEventEnrollmentResult,
    EventEnrollmentEvaluationBatch, EventEnrollmentEvaluationDiagnostic,
    EventEnrollmentEvaluationOutcome, EventEnrollmentEvaluationReport,
    EventEnrollmentEvaluationStage, EventEnrollmentReliabilityCause,
    EventEnrollmentReliabilityResult, EventEnrollmentTransition, SuppressedDelivery,
    evaluate_event_enrollments_for_delivery,
};

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{Error, RemoteError};

/// Why an enrollment's own configuration couldn't be read for a pass.
/// Internal until the pass moves here.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalConfigurationReadError {
    /// The configuration can't be used: a required property is missing, a
    /// value doesn't decode, the fault algorithm isn't one the evaluator
    /// supports, or a Status_Flags reference names another device.
    Malformed,
    /// The read failed some other way, for now.
    Unavailable,
}

/// Classify a failed read of a required property: UNKNOWN_PROPERTY makes
/// the configuration malformed, anything else leaves it unavailable.
pub(crate) fn classify_required_property_read_error(error: &Error) -> LocalConfigurationReadError {
    if matches!(
        error,
        Error::Remote(RemoteError::Error { class, code })
            if *class == ErrorClass::PROPERTY
                && *code == ErrorCode::UNKNOWN_PROPERTY
    ) {
        LocalConfigurationReadError::Malformed
    } else {
        LocalConfigurationReadError::Unavailable
    }
}

#[cfg(test)]
mod tests;
