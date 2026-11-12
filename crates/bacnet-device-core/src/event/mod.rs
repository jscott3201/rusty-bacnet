//! Intrinsic event reporting (Clause 13.2), behind the `event-reporting`
//! feature, as staged functions a host calls under its database guard:
//!
//! - [`after_write`] evaluates one changed object's detector and commits
//!   the transition it proposes; [`sweep`] does so for every object once a
//!   pass, advancing Time_Delay and turning a Reliability change into a
//!   FAULT transition. Both commit through [`commit_intrinsic_transition`]:
//!   Event_State, Acked_Transitions, the timestamp and the message text at
//!   once (Clause 13.2.2.1.4), before Event_Enable decides whether the
//!   transition goes out (Clause 13.2.5).
//! - [`distribute_event`] builds a transition's EventNotification (Table
//!   13-3), resolves its priority and Ack_Required from the Notification
//!   Class, selects the recipients whose days, window and transitions
//!   match, and records it in the device's Event Log objects. The host
//!   sends it once its guard is dropped.
//! - The timestamps a commit and a notification carry
//!   ([`stage_event_timestamp`], [`sample_event_timestamp`]), the
//!   committed message text ([`intrinsic_event_message_text`]) and the
//!   Event Values ([`project_intrinsic_payload`],
//!   [`project_event_enrollment_payload`]).
//! - AcknowledgeAlarm ([`handle_acknowledge_alarm`], Clause 13.5) and the
//!   ACK_NOTIFICATION it may send, GetEventInformation
//!   ([`handle_get_event_information_configured`], Clauses 13.2.4 and
//!   13.12), and the fault pass ([`fault_pass`]).
//!
//! No lock, clock, async or formatting: what a step would have logged comes
//! back as a value ([`IntrinsicCommitRefusal`], [`DistributionOutcome`],
//! [`EventSummaryProblem`], [`FaultPass::failures`]). `bacnet-server` runs
//! its intrinsic reporting on these, once a second for the sweep, so
//! Time_Delay still counts sweeps.

mod acknowledge;
mod commit;
mod distribute;
mod event_information;
mod fault;
mod intrinsic;
mod message;
mod payload;
mod profile;
mod timestamp;

pub use acknowledge::{
    AcceptedAcknowledgeAlarm, AcknowledgmentNotificationContext, handle_acknowledge_alarm,
};
pub use commit::{
    IntrinsicCommitRefusal, MessageTexts, commit_intrinsic_transition,
    read_committed_history_snapshot,
};
pub use distribute::{DistributionOutcome, EventDistribution, distribute_event};
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub use event_information::handle_get_event_information_with_budget;
pub use event_information::{
    EventInformationFailure, EventSummaryFault, EventSummaryProblem, GetEventInformationBudget,
    handle_get_event_information, handle_get_event_information_configured,
};
pub use fault::{FaultPass, ReliabilityChange, fault_pass};
pub use intrinsic::{IntrinsicCommit, after_write, next_due, sweep};

pub use message::intrinsic_event_message_text;
pub use payload::{
    CapturedReferencedValue, CapturedStatusFlags, CommittedNotificationPayload,
    EventEnrollmentProjectionSnapshot, capture_status_flags, decode_reference_value,
    project_event_enrollment_payload, project_intrinsic_payload,
};
pub use profile::{
    CommittedHistorySnapshot, CommittedIntrinsicTransition, NotificationConstruction,
    NotificationHistorySource, NotificationTransition,
};
pub use timestamp::{
    EventTimestampSample, SampledEventClock, StagedEventTimestamp, confirm_event_timestamp,
    sample_event_timestamp, stage_event_timestamp,
};
