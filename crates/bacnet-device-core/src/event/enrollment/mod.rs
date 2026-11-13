//! Event Enrollment, algorithmic reporting (#2319, behind
//! `event-enrollment`): the evaluation pass over every Event Enrollment
//! object, `evaluate_event_enrollments_for_delivery`, with the event and
//! fault algorithms it runs, the reads of each enrollment's parameters and
//! monitored reference, and the commit of what it finds. Moved from
//! `bacnet-server`, which drives the pass on its task, logs what it reports
//! and sends what it commits. The pass needs `std` until it keeps its
//! per-pass state in ordered maps (#2319 PR 2b); the algorithms don't.
//!
//! A host calls the pass with its database guard held and gets an
//! `EventEnrollmentEvaluationBatch`: the `EventEnrollmentEvaluationReport`
//! of what changed, the committed transitions to distribute
//! (`CommittedEventEnrollmentDelivery`) and the ones whose payload was
//! refused (`SuppressedDelivery`), for it to log.

// Without `std` nothing calls them yet: the pass that does needs `std`
// until #2319 PR 2b.
#[cfg_attr(not(feature = "std"), allow(dead_code))]
mod algorithms;
#[cfg_attr(not(feature = "std"), allow(dead_code))]
mod fault;
#[cfg_attr(not(feature = "std"), allow(dead_code))]
mod reference;

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
#[cfg_attr(not(feature = "std"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalConfigurationReadError {
    /// The configuration can't be used: a required property is missing, a
    /// value doesn't decode, the fault algorithm isn't one the evaluator
    /// supports, or a Status_Flags reference names another device.
    Malformed,
    /// The read failed some other way, for now.
    Unavailable,
}

/// Classify a failed read of a required property: UNKNOWN_PROPERTY makes
/// the configuration malformed, anything else leaves it unavailable.
#[cfg_attr(not(feature = "std"), allow(dead_code))]
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
