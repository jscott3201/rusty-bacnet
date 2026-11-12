//! Intrinsic event reporting (Clause 13.2), behind the `event-reporting`
//! feature: the timestamps a transition commits and a notification carries
//! ([`stage_event_timestamp`], [`sample_event_timestamp`]), the message text
//! an intrinsic transition commits ([`intrinsic_event_message_text`]), the
//! Event Values a committed transition reports (Table 13-3,
//! [`project_intrinsic_payload`], [`project_event_enrollment_payload`]), and
//! the profile a transition is distributed by ([`NotificationTransition`]).
//!
//! Plain functions over a borrowed `ObjectDatabase`: no lock, clock or
//! async. A host calls them under its own database guard and sends what
//! they build after the guard drops. `bacnet-server` runs its intrinsic
//! reporting on them.

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
pub use event_information::{
    EventInformationFailure, EventSummaryFault, EventSummaryProblem, GetEventInformationBudget,
    handle_get_event_information, handle_get_event_information_configured,
    handle_get_event_information_with_budget,
};
pub use fault::{FaultPass, ReliabilityChange, fault_pass};
pub use intrinsic::{IntrinsicCommit, after_write, sweep};

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
