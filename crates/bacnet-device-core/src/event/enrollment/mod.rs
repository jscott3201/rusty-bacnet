//! Event Enrollment, algorithmic reporting (#2319, behind
//! `event-enrollment`). Where intrinsic reporting is built into an object
//! type, an Event Enrollment is an object of its own: it watches a property
//! of another object and runs an event algorithm over what it reads. This
//! module holds the evaluation pass over every Event Enrollment object,
//! `evaluate_event_enrollments_for_delivery`, with the event and fault
//! algorithms it runs, the reads of each enrollment's parameters and
//! monitored reference, and the commit of what it finds. It moved from
//! `bacnet-server`, which drives the pass on its task, logs what it reports
//! and sends what it commits. It needs `alloc`, not `std`: a pass keeps
//! its per-enrollment state in B-tree maps (#2319 PR 2b).
//!
//! # Calling the pass
//!
//! A host calls the pass with its database guard held, once per interval,
//! and gets an `EventEnrollmentEvaluationBatch`: the
//! `EventEnrollmentEvaluationReport` of what changed, the committed
//! transitions with what their notification needs
//! (`CommittedEventEnrollmentDelivery`: the payload and the history the
//! commit wrote, read back under the same guard) and the ones whose payload
//! or history couldn't be built (`SuppressedDelivery`), for it to log. A
//! delivery is not yet a send: it may be a transition the enrollment's
//! `Event_Enable` leaves out, so the host sends only those whose
//! `distribute()` is true, as the server does, and
//! `CommittedEventEnrollmentDelivery::into_notification` gives the
//! notification to send, with nothing more to read from the database.
//!
//! A pass works through the enrollments in the database's order, the order
//! they were added in (`ObjectDatabase::find_by_type`), never by
//! identifier: an enrollment replaced under the same identifier keeps its
//! place, and one removed and added again goes last. The pass follows their
//! inhibit references in that order, then commits in it. Every list in the batch comes out in that order, and
//! without a Device clock the commits take event sequence numbers in it.
//!
//! # Algorithms
//!
//! The pass evaluates OUT_OF_RANGE, FLOATING_LIMIT, CHANGE_OF_STATE,
//! CHANGE_OF_BITSTRING and CHANGE_OF_VALUE.
//!
//! # Time delays (#163)
//!
//! The `Time_Delay` in `Event_Parameters` (pTimeDelay, Table 12-15) holds
//! back each indicated transition to an offnormal state. The enrollment's
//! optional `Time_Delay_Normal` (pTimeDelayNormal, Table 12-14), or
//! pTimeDelay when it is absent (Clause 13.3), holds back the return to
//! NORMAL. Both count the seconds a condition has lasted (13.3.1), and so
//! does the pass: a pending countdown, kept in memory on the enrollment
//! object, starts at the delay over the host's interval, rounded up, and
//! drops by one each pass. Rounding up means it never fires early: at a
//! 10-second interval a 5-second delay fires on the second pass, about 10
//! seconds in, rather than after five passes. Otherwise the countdown acts
//! as the intrinsic detectors' does (`bacnet_objects::event`, #120, #225),
//! in code of its own: the condition clearing cancels it, the same target
//! never restarts it, and a new target starts it again.
//!
//! The server fixes its interval when it is built and the countdown isn't
//! saved, so no countdown is rescaled while running. After a restart the
//! pass starts again from the confirmed `Event_State` with a fresh
//! rounded-up count, as the intrinsic detectors do.
//!
//! # Transition actions (#166)
//!
//! An indicated transition runs its actions (Clause 13.2.2.1.4) even when
//! its target is the state the enrollment is already in. The state the
//! algorithm returned goes into `Event_State`, and the matching
//! `Acked_Transitions` bit is set or cleared from the `Ack_Required` of the
//! enrollment's Notification Class (Clause 13.2.3). `Event_Time_Stamps` and
//! `Event_Message_Texts` commit in the same step as `Event_State` and
//! `Acked_Transitions`. An enrollment has no message text of its own here,
//! so a transition's text is its entry in `Event_Message_Texts_Config`, sent
//! as written with no substitution codes, or none when that entry is empty
//! (#1507). Each committed transition carries a `distribute` flag from
//! `Event_Enable`, and the host's sending repeats none of these actions.
//!
//! # Event_Algorithm_Inhibit (#1507)
//!
//! The inhibit holds the algorithm back as it does for intrinsic reporting
//! (the private `algorithms::inhibited_arm`). A pass that can't read the
//! monitored value at all has a gap in what it saw and changes no state,
//! inhibited or not, so a forced return to NORMAL waits for the next pass
//! that reads it, and its notification carries that value. An enrollment
//! whose parameters the pass doesn't evaluate (Opaque, Extended, or an
//! algorithm with no evaluator here) still gets the forced NORMAL, since
//! leaving an offnormal state under the inhibit doesn't depend on the
//! algorithm (Clause 13.2.2.1.2). With `Event_Algorithm_Inhibit_Ref` set,
//! the inhibit is the referenced property's value as of its last refresh:
//! each write of the enrollment, the host's own refresh (the server's runs
//! every second) and the start of each pass.

mod algorithms;
mod fault;
mod reference;

mod pass;
pub use pass::{
    CommittedEventEnrollmentDelivery, CommittedEventEnrollmentResult,
    EventEnrollmentEvaluationBatch, EventEnrollmentEvaluationDiagnostic,
    EventEnrollmentEvaluationOutcome, EventEnrollmentEvaluationReport,
    EventEnrollmentEvaluationStage, EventEnrollmentReliabilityCause,
    EventEnrollmentReliabilityResult, EventEnrollmentTransition, SuppressedDelivery,
    SuppressionCause, evaluate_event_enrollments_for_delivery,
};

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{Error, RemoteError};

/// Why an enrollment's own configuration couldn't be read for a pass.
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
