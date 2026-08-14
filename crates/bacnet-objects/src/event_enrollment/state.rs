use bacnet_types::enums::{EventState, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// Effective object, property, and optional array index that own an Event
/// Enrollment object's private evaluation state.
pub type EventEnrollmentMonitoredSource = (ObjectIdentifier, PropertyIdentifier, Option<u32>);

/// A delayed Event Enrollment transition, counting down its delay.
///
/// The enrollment counterpart of the intrinsic detectors'
/// [`PendingTransition`](crate::event::PendingTransition), kept as a distinct
/// type because the driving mechanism differs: the server evaluator advances
/// `remaining` once per *evaluation pass* (the `event_enrollment_task`
/// interval, configurable via #133), whereas the intrinsic detectors tick on
/// a fixed one-second task and seed from per-write probes. Clause 13.2.4
/// semantics are shared — the observable `Event_State` holds at the confirmed
/// state while the countdown runs, a reverted condition cancels without
/// firing, and a redundant qualifying observation never re-seeds — but the
/// two implementations do not share code across the objects/server boundary.
///
/// In-memory only: like the intrinsic detectors' pending state and baselines,
/// this is not persisted; a device restart re-evaluation starts from the
/// confirmed `Event_State`, which is the same restart semantics the
/// intrinsic-reporting path ships.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventEnrollmentPending {
    /// The event state the algorithm indicated and will enter when the
    /// countdown elapses.
    pub state: EventState,
    /// Evaluation passes remaining before the transition fires; seeded with
    /// the direction-appropriate delay (pTimeDelay for offnormal targets,
    /// pTimeDelayNormal — else pTimeDelay — for NORMAL), converted from
    /// seconds by the evaluator as `ceil(delay_secs / interval_secs)`.
    pub remaining: u32,
    /// Identity of the indicating condition, per algorithm. CHANGE_OF_STATE
    /// discriminates by the matched alarm value because Clause 13.3.2
    /// conditions (a)/(c) key on *which* value the monitored value equals
    /// ("remains equal to that value for pTimeDelay"); CHANGE_OF_BITSTRING by
    /// the masked monitored bytes. Algorithms whose delay applies to the
    /// threshold condition itself (OUT_OF_RANGE, FLOATING_LIMIT,
    /// CHANGE_OF_VALUE) use `0` — the target alone identifies them.
    pub condition: u64,
    /// Fingerprint of the `Event_Parameters` (framed encoding) plus the
    /// effective `Time_Delay_Normal` in force when this countdown was seeded.
    /// The evaluator re-reads its parameters every pass; a mismatch cancels
    /// the in-flight countdown and re-gates from the current parameters —
    /// no partial countdown is resumed across a parameter change.
    pub params_fingerprint: u64,
}

/// Algorithm-side evaluation state owned by an Event Enrollment object.
///
/// Not BACnet properties: none of the three slots maps to a Clause 12.12
/// property (nor to the Table 12-14 `Time_Delay_Normal`, which is
/// configuration and lives on the object directly). Clause 13.3 assigns the
/// baseline's initialization and the countdown's existence to local matters,
/// so they are reachable only through the internal trait channel
/// ([`BACnetObject::enrollment_eval_state_internal`](crate::traits::BACnetObject::enrollment_eval_state_internal) /
/// [`BACnetObject::set_enrollment_eval_state_internal`](crate::traits::BACnetObject::set_enrollment_eval_state_internal)),
/// mirroring the `set_event_state_internal` precedent (issue #130).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventEnrollmentEvalState {
    /// Delayed transition in flight, if any.
    pub pending: Option<EventEnrollmentPending>,
    /// CHANGE_OF_VALUE detection baseline (Clause 13.3.3: "the value of the
    /// monitored value when a transition to NORMAL is indicated shall be used
    /// in evaluation of the conditions until the next transition to NORMAL is
    /// indicated"). `None` before the first sample; the first observed value
    /// initializes it without indicating a transition ("the initialization of
    /// the value used in evaluation before the first transition to NORMAL is
    /// indicated is a local matter" — the policy chosen here).
    pub cov_baseline: Option<PropertyValue>,
    /// The monitored value that caused the last transition to OFFNORMAL, for
    /// CHANGE_OF_STATE condition (c) (Clause 13.3.2: a re-indication is
    /// indicated only when the monitored value equals an alarm value
    /// "different from the value that caused the last transition to
    /// OFFNORMAL").
    pub last_offnormal_value: Option<u32>,
}

pub(super) enum EventEnrollmentWriteRollback {
    Detection {
        enabled: bool,
        event_state: u32,
        acked_transitions: u8,
        monitored_reference: Option<EventEnrollmentMonitoredSource>,
        evaluation: EventEnrollmentEvalState,
    },
    TimeDelayNormal(Option<u32>),
}
