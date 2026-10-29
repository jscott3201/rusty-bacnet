//! Intrinsic reporting — OUT_OF_RANGE event state machine.
//!
//! Per ASHRAE 135-2020 Clause 13.3.6 (Table 13-7), the OUT_OF_RANGE algorithm monitors
//! an analog present_value against HIGH_LIMIT and LOW_LIMIT with a DEADBAND
//! to prevent oscillation at the boundary.

use crate::prelude::*;
use core::ops::ControlFlow;

use bacnet_types::bitstring::{EventTransitionBits, LimitEnable};
use bacnet_types::enums::{EventState, EventType, NotifyType, Reliability};
use bacnet_types::primitives::BACnetTimeStamp;

pub(crate) mod history;
pub(crate) mod options;
pub(crate) mod state_reporting;

/// A detected change in event state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventStateChange {
    /// The previous event state.
    pub from: EventState,
    /// The new event state.
    pub to: EventState,
}

/// A fire-ready transition and its external distribution policy.
///
/// ASHRAE 135-2020 Clause 13.2.2.1.4 attaches four effects to every
/// transition: `Event_State` takes the new state, `Event_Time_Stamps` records
/// when it happened, `Event_Message_Texts` (on objects that have it) records
/// the message, and both alarm-acknowledgment and notification distribution
/// are told about it. `Event_Enable` gates none of the four; it only turns off
/// external distribution further downstream (Clause 13.2.5), so a cleared bit
/// must not suppress the three property updates, nor the alarm-acknowledgment
/// part of the hand-off.
///
/// The object intrinsic-reporting hooks return this value as an uncommitted
/// proposal. The server commits object-owned state and history before considering
/// distribution, including when `distribute == false`. A failed commit leaves the
/// proposal retryable. The value alone does not prove a committed object transition.
/// Standalone detector `probe` and `tick` methods may finalize detector-local state
/// according to their own contracts; they do not commit object event history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionOutcome {
    /// The proposed transition, to be committed regardless of `distribute`.
    pub change: EventStateChange,
    /// The Event Type selected by the detector's event algorithm, with the
    /// mandatory `CHANGE_OF_RELIABILITY` override for transitions to/from FAULT.
    pub event_type: EventType,
    /// The `Event_Enable` bit for this transition's direction.
    pub distribute: bool,
}

impl EventStateChange {
    /// Select the notification's Event Type for this transition.
    ///
    /// ASHRAE 135-2020 Clause 13.2.5.3 requires
    /// `CHANGE_OF_RELIABILITY` for every transition to or from FAULT.
    /// Any other transition reports the Event Type that belongs to the
    /// object's own configured algorithm (Clauses 13.8.1.1 and 13.9.1.1),
    /// which the caller passes as `algorithm`.
    pub fn event_type(&self, algorithm: EventType) -> EventType {
        if self.from == EventState::FAULT || self.to == EventState::FAULT {
            EventType::CHANGE_OF_RELIABILITY
        } else {
            algorithm
        }
    }

    /// Derive the event transition category from the state change.
    ///
    /// - `to == NORMAL` -> `ToNormal`
    /// - `to == FAULT` -> `ToFault`
    /// - Everything else (OFFNORMAL, HIGH_LIMIT, LOW_LIMIT) -> `ToOffnormal`
    pub fn transition(&self) -> EventTransition {
        EventTransition::for_target_state(self.to)
    }
}

/// Event transition category per ASHRAE 135-2020 Clause 13.2.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventTransition {
    /// Transition to an off-normal state (OFFNORMAL, HIGH_LIMIT, LOW_LIMIT, etc.).
    ToOffnormal,
    /// Transition to FAULT.
    ToFault,
    /// Transition to NORMAL.
    ToNormal,
}

/// Object-owned GetEnrollmentSummary event capability.
///
/// This internal projection identifies the object's one configured or implied
/// event algorithm and the coordinate of its latest successful commit. It does
/// not expose detector state or treat the CHANGE_OF_RELIABILITY notification
/// override as a second event algorithm.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnrollmentSummaryCapability {
    /// Configured or object-type-implied event algorithm.
    pub event_type: EventType,
    /// Latest successfully committed transition coordinate.
    pub last_transition: Option<EventTransition>,
}

impl EventTransition {
    /// Classify a destination event state under ASHRAE 135-2020 Clause 13.2.
    ///
    /// NORMAL and FAULT are the only states outside the offnormal category,
    /// as also specified by Clause 13.2.2.1.2. Therefore the residual case
    /// is deliberately TO_OFFNORMAL, not TO_FAULT.
    pub fn for_target_state(state: EventState) -> Self {
        if state == EventState::NORMAL {
            Self::ToNormal
        } else if state == EventState::FAULT {
            Self::ToFault
        } else {
            Self::ToOffnormal
        }
    }

    /// This transition's flag in a `BACnetEventTransitionBits` value, such as
    /// `Event_Enable`, `Acked_Transitions`, `Ack_Required` or a
    /// `BACnetDestination`'s transitions.
    pub fn bit_mask(self) -> EventTransitionBits {
        match self {
            EventTransition::ToOffnormal => EventTransitionBits::TO_OFFNORMAL,
            EventTransition::ToFault => EventTransitionBits::TO_FAULT,
            EventTransition::ToNormal => EventTransitionBits::TO_NORMAL,
        }
    }

    /// Positional index into the NotificationClass `PRIORITY` array, ordered
    /// `[TO_OFFNORMAL, TO_FAULT, TO_NORMAL]`. `ACK_REQUIRED` is a bit string:
    /// test it with [`EventTransition::bit_mask`].
    pub fn index(self) -> usize {
        match self {
            EventTransition::ToOffnormal => 0,
            EventTransition::ToFault => 1,
            EventTransition::ToNormal => 2,
        }
    }
}

/// All object-owned values needed to commit one event-state transition.
///
/// Callers stage the timestamp and optional message before invoking an
/// object's internal commit hook. The object then validates the transition
/// coordinate and source state before changing any of its event properties.
#[derive(Debug, Clone, PartialEq)]
pub struct EventTransitionCommit {
    /// The expected source state and exact destination state.
    pub change: EventStateChange,
    /// The destination state's transition coordinate.
    pub coordinate: EventTransition,
    /// Whether the referenced Notification Class requires acknowledgment.
    pub ack_required: bool,
    /// The timestamp CHOICE to store for `coordinate`.
    pub timestamp: BACnetTimeStamp,
    /// Replacement message for `coordinate`, or `None` when the property is absent.
    pub message_text: Option<String>,
}

/// Failure to atomically commit an event-state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventTransitionCommitError {
    /// The object does not implement the internal transition-commit channel.
    Unsupported,
    /// The supplied coordinate does not classify the exact destination state.
    CoordinateTargetMismatch {
        /// The coordinate supplied by the caller.
        coordinate: EventTransition,
        /// The exact destination state supplied by the caller.
        target: EventState,
    },
    /// The object's current state no longer matches the staged source state.
    CurrentStateMismatch {
        /// The source state expected by the staged transition.
        expected: EventState,
        /// The object's current state when the commit was attempted.
        actual: EventState,
    },
}

/// Pending (delayed) intrinsic-reporting transition state, shared by every
/// detector that honors [`OutOfRangeDetector::time_delay`] and its peers.
///
/// Per ASHRAE 135-2020 Clause 13.3, a transition to a new `EventState` is
/// delayed by `Time_Delay` seconds. While the delay counts down the
/// observable `event_state` stays at the *old* (confirmed) state; if the
/// triggering condition clears before the delay elapses the pending
/// transition is cancelled and no notification is sent.
///
/// The countdown advances once per elapsed wall-clock second via
/// the detector's `tick` (for example [`OutOfRangeDetector::tick`]), never per `probe` call — so a fast poll
/// loop writing the same out-of-range value cannot shorten the delay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTransition {
    /// The state the detector wants to transition to once the delay elapses.
    pub state: EventState,
    /// Seconds remaining; seeded with the direction-appropriate delay (see
    /// `delay_toward`) and decremented per tick.
    pub remaining: u32,
}

impl PendingTransition {
    /// Begin a pending transition to `state`, seeded with `delay` seconds.
    fn seed(state: EventState, delay: u32) -> Self {
        Self {
            state,
            remaining: delay,
        }
    }
}

/// Select the delay governing a transition toward `target`.
///
/// ASHRAE 135-2020 Clause 13.3 gives the event algorithms two independent
/// delays. pTimeDelay measures the required persistence of an offnormal
/// condition in seconds before indicating that state: it governs
/// every indication into an OFFNORMAL state, including offnormal→offnormal
/// re-indication (CHANGE_OF_STATE (c) at 13.3.2, OUT_OF_RANGE (d)/(g) at
/// 13.3.6, COMMAND_FAILURE (a) at 13.3.4). pTimeDelayNormal measures the
/// required persistence of a Normal condition in seconds and gates only the
/// sustained-condition return to
/// NORMAL (CHANGE_OF_STATE (b), COMMAND_FAILURE (b), OUT_OF_RANGE (e)/(h)).
///
/// An absent pTimeDelayNormal uses pTimeDelay as its fallback, so `None`
/// behaves exactly as `time_delay`, never as an error or a zero.
///
/// FAULT never reaches this selector: Clause 13.2.2 fault precedence runs
/// ahead of the event algorithm and carries no delay term.
fn delay_toward(time_delay: u32, time_delay_normal: Option<u32>, target: EventState) -> u32 {
    if target == EventState::NORMAL {
        time_delay_normal.unwrap_or(time_delay)
    } else {
        time_delay
    }
}

/// What an algorithm proposes while Event_Algorithm_Inhibit is TRUE, once
/// fault precedence has had its say (Clauses 13.2.2.1 and 13.2.2.1.5): its
/// own result is ignored and any countdown dropped, so a condition has to
/// last its whole delay again once the inhibit clears, and an offnormal
/// state goes back to NORMAL at once. `None` when already NORMAL.
fn inhibited_target(
    event_state: EventState,
    pending: &mut Option<PendingTransition>,
) -> Option<EventState> {
    *pending = None;
    (event_state != EventState::NORMAL).then_some(EventState::NORMAL)
}

/// What Clause 13.2.2's fault-precedence rule dictates for a single evaluation.
///
/// ASHRAE 135-2020 Clause 13.2.2 assigns normal/offnormal selection to the
/// event algorithm and fault selection to Reliability. A Reliability value
/// unequal to NO_FAULT_DETECTED overrides normal/offnormal detection,
/// making the object's event state FAULT.
///
/// **Whether FAULT holds is a standing condition; whether a transition fires is
/// an edge.** Clause 13.2.2.1 ties the Fault state to a reliability-evaluation
/// result unequal to NO_FAULT_DETECTED, so
/// the FAULT determination is re-derived from `reliability` on every evaluation
/// and is never latched. But the same clause's ToFault transition requires a
/// **change** in Reliability, which is an edge and cannot be derived from
/// the current value alone. That is what `fault_reliability` stores: the value in
/// force at the last entry to FAULT, and nothing else.
///
/// The distinction matters because the fault proposal runs at the head of both
/// detector paths, and the server drives the periodic path once per second.
/// Deciding re-entry from the standing condition rather than the edge would
/// emit a notification every second for as long as any object stayed faulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FaultPrecedence {
    /// Reliability is bad and FAULT does not hold yet: transition immediately.
    ///
    /// Clause 13.2.2.1 puts no delay on ToFault: the moment reliability
    /// evaluation yields any value other than NO_FAULT_DETECTED, the object
    /// goes to Fault and the usual transition actions run. `Time_Delay` belongs
    /// to the event algorithm (Clause 13.3.1 uses pTimeDelay to require sustained
    /// offnormal conditions before an indication), and
    /// the algorithm is precisely what fault detection takes precedence over.
    EnterFault,
    /// Reliability is bad, **unchanged**, and FAULT already holds: the standing
    /// condition is satisfied, so no transition fires — and the algorithm must
    /// not run. [`FaultPrecedence::ReenterFault`] takes precedence when the value
    /// differs; this variant is only the unchanged case.
    HoldFault,
    /// Reliability changed while FAULT already holds: execute the transition
    /// actions and re-enter FAULT.
    ///
    /// In Clause 13.2.2.1's state machine a Reliability change while in Fault
    /// is itself a ToFault transition, provided the new value is still a fault
    /// (not NO_FAULT_DETECTED), so the transition actions run again.
    ///
    /// Also selected when FAULT holds with no recorded value — a state this
    /// crate never produces but a downstream implementor can construct, since
    /// both fields are public. See the comment on that match arm.
    ReenterFault,
    /// Reliability recovered while in FAULT.
    ///
    /// Clause 13.2.2.1 leaves Fault for Normal, with its transition actions, as
    /// soon as reliability evaluation is back to NO_FAULT_DETECTED.
    /// **NORMAL specifically —
    /// not a state re-derived from the event algorithm.** Recovering straight
    /// into HIGH_LIMIT because the present value is still out of range would
    /// invent a transition the state machine does not define; the algorithm gets
    /// to move the object out of NORMAL afterwards, under its own conditions and
    /// its own `Time_Delay`.
    RecoverToNormal,
    /// No fault in play; the event algorithm determines the state.
    RunAlgorithm,
}

/// Apply Clause 13.2.2's fault-precedence rule to one evaluation.
///
/// Shared by every detector so the rule has a single definition: a detector
/// added later that forgets to consult it is a visible omission rather than a
/// silently missing clause.
pub(crate) fn fault_precedence(
    reliability: Reliability,
    fault_reliability: Option<Reliability>,
    current: EventState,
) -> FaultPrecedence {
    let faulted = reliability != Reliability::NO_FAULT_DETECTED;
    match (faulted, current == EventState::FAULT, fault_reliability) {
        (true, false, _) => FaultPrecedence::EnterFault,
        (true, true, Some(previous)) if previous == reliability => FaultPrecedence::HoldFault,
        (true, true, Some(_)) => FaultPrecedence::ReenterFault,
        // In FAULT with no recorded value. This breaks the field's invariant
        // (`Some` exactly while FAULT holds) and no path in this crate produces
        // it, but both `event_state` and `fault_reliability` are public on a
        // published crate, so a downstream implementor can construct it.
        //
        // Re-entering is the deliberate choice, because it is self-healing:
        // `ReenterFault` stores `Some(reliability)` and the invariant holds from
        // then on, at a cost of one transition that may report no real change.
        // Holding would be worse than it looks — it stores nothing, so the field
        // would stay `None` forever and every later genuine change would land
        // back on this arm and hold again, permanently disabling re-entry for
        // that detector.
        (true, true, None) => FaultPrecedence::ReenterFault,
        (false, true, _) => FaultPrecedence::RecoverToNormal,
        (false, false, _) => FaultPrecedence::RunAlgorithm,
    }
}

/// OUT_OF_RANGE event detector for analog objects.
///
/// Implements the OUT_OF_RANGE event state machine:
/// - NORMAL → HIGH_LIMIT when `present_value > high_limit` (if high_limit enabled)
/// - NORMAL → LOW_LIMIT when `present_value < low_limit` (if low_limit enabled)
/// - HIGH_LIMIT → NORMAL when `present_value < high_limit - deadband`
/// - LOW_LIMIT → NORMAL when `present_value > low_limit + deadband`
/// - HIGH_LIMIT → LOW_LIMIT when `present_value < low_limit`
/// - LOW_LIMIT → HIGH_LIMIT when `present_value > high_limit`
///
/// `Time_Delay` (and `Time_Delay_Normal` for the NORMAL direction) is
/// honored via the split [`Self::probe`] / [`Self::tick`] entry points: a
/// present-value write calls `probe`, which seeds a pending transition (or
/// fires immediately when the direction-appropriate delay is zero,
/// `delay_toward`); a one-second periodic task calls `tick` to advance the
/// countdown and fire on expiry.
#[derive(Debug, Clone)]
pub struct OutOfRangeDetector {
    /// High_Limit; a present value above it is a HIGH_LIMIT condition.
    pub high_limit: f32,
    /// Low_Limit; a present value below it is a LOW_LIMIT condition.
    pub low_limit: f32,
    /// Deadband applied when returning toward NORMAL, in the monitored value's units.
    pub deadband: f32,
    /// Which of the two limits are checked.
    pub limit_enable: LimitEnable,
    /// Instance number of the Notification Class object that distributes the events.
    pub notification_class: u32,
    /// Notify_Type: whether the transitions are reported as alarms or as events.
    pub notify_type: NotifyType,
    /// Event_Enable: the transitions whose notifications are distributed.
    pub event_enable: EventTransitionBits,
    /// Seconds an offnormal condition must persist before TO_OFFNORMAL fires, and the NORMAL
    /// delay too when `time_delay_normal` is `None`; 0 fires immediately. Fault transitions are
    /// never delayed.
    pub time_delay: u32,
    /// `Time_Delay_Normal` (property 356): the Clause 13.3.6 pTimeDelayNormal
    /// parameter — the number of seconds a return to normal has to be
    /// sustained before the detector confirms it. `None` is the not-configured case
    /// and uses `time_delay` as the fallback required for absent pTimeDelayNormal.
    pub time_delay_normal: Option<u32>,
    /// Current event state.
    pub event_state: EventState,
    /// Acked_Transitions: a set flag means that transition was acknowledged.
    pub acked_transitions: EventTransitionBits,
    /// Pending delayed transition, or `None` when no delay is in progress.
    pub pending: Option<PendingTransition>,
    /// Reliability value in force at the last entry to FAULT; `None` outside FAULT.
    pub fault_reliability: Option<Reliability>,
}

impl Default for OutOfRangeDetector {
    fn default() -> Self {
        Self {
            high_limit: 100.0,
            low_limit: 0.0,
            deadband: 1.0,
            limit_enable: LimitEnable::empty(),
            notification_class: 0,
            notify_type: NotifyType::ALARM,
            event_enable: EventTransitionBits::empty(),
            time_delay: 0,
            time_delay_normal: None,
            event_state: EventState::NORMAL,
            acked_transitions: EventTransitionBits::all(), // all acknowledged by default
            pending: None,
            fault_reliability: None,
        }
    }
}

impl OutOfRangeDetector {
    /// BACnet Event Type corresponding to this detector's event algorithm.
    pub const ALGORITHM: EventType = EventType::OUT_OF_RANGE;

    /// Evaluate the present value against configured limits.
    ///
    /// This is the per-write entry point: it seeds a pending delayed
    /// transition (or fires immediately when the direction-appropriate delay
    /// is zero) but never advances the countdown — so repeated writes to the
    /// same value do not shorten the delay. Returns `Some(TransitionOutcome)`
    /// whenever a transition fires; the outcome's `distribute` flag carries
    /// the `event_enable` bit rather than withholding the transition.
    pub fn evaluate(
        &mut self,
        present_value: f32,
        reliability: Reliability,
    ) -> Option<TransitionOutcome> {
        self.probe(present_value, reliability)
    }

    /// Apply Clause 13.2.2 fault precedence ahead of the event algorithm.
    ///
    /// `Break` means reliability governed this evaluation and the algorithm must
    /// not run; the payload is the transition it produced, if any. `Continue`
    /// hands the decision to the algorithm.
    ///
    /// Each detector carries its own copy because each reaches its own
    /// `event_state`, `pending`, and confirmation; the clause interpretation
    /// they share lives once, in [`fault_precedence`].
    fn fault_proposal(&self, reliability: Reliability) -> ControlFlow<Option<TransitionOutcome>> {
        match fault_precedence(reliability, self.fault_reliability, self.event_state) {
            FaultPrecedence::EnterFault | FaultPrecedence::ReenterFault => {
                ControlFlow::Break(self.proposal(EventState::FAULT))
            }
            FaultPrecedence::HoldFault => ControlFlow::Break(None),
            FaultPrecedence::RecoverToNormal => {
                ControlFlow::Break(self.proposal(EventState::NORMAL))
            }
            FaultPrecedence::RunAlgorithm => ControlFlow::Continue(()),
        }
    }

    /// Per-write probe: seed or cancel a pending transition, fire on zero delay.
    ///
    /// When the direction-appropriate delay (`delay_toward`) is zero the
    /// transition is confirmed immediately and `event_state` is updated,
    /// preserving the legacy instant-transition behavior. Otherwise a
    /// [`PendingTransition`] is seeded (or cleared if the condition
    /// reverted) and `None` is returned; the periodic [`Self::tick`]
    /// advances and eventually confirms it.
    pub fn probe(
        &mut self,
        present_value: f32,
        reliability: Reliability,
    ) -> Option<TransitionOutcome> {
        let outcome = self.propose(present_value, reliability, false);
        if let Some(ref outcome) = outcome {
            self.confirm_transition(&outcome.change, reliability);
        }
        outcome
    }

    /// Stage a per-write transition without confirming any transition-owned state.
    pub(crate) fn propose(
        &mut self,
        present_value: f32,
        reliability: Reliability,
        inhibited: bool,
    ) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_proposal(reliability) {
            return result;
        }
        if inhibited {
            return inhibited_target(self.event_state, &mut self.pending)
                .and_then(|target| self.proposal(target));
        }
        let desired = self.compute_new_state(present_value);
        if desired == self.event_state {
            // Condition reverted to the confirmed state: cancel any pending
            // transition without firing or notifying.
            self.pending = None;
            return None;
        }
        let delay = delay_toward(self.time_delay, self.time_delay_normal, desired);
        if delay == 0 {
            return self.proposal(desired);
        }
        // Nonzero delay: seed a pending transition only when there is none to
        // the same target. A redundant write of the same qualifying value must
        // NOT restart the countdown (ASHRAE 135-2020 Clause 13.3 times the
        // delay from when the condition began to hold); re-seeding here would
        // let writes faster than the 1s tick pin the transition forever. The
        // periodic `tick` advances it.
        if self.pending.as_ref().is_none_or(|p| p.state != desired) {
            self.pending = Some(PendingTransition::seed(desired, delay));
        }
        None
    }

    /// Periodic tick: advance the countdown and fire on expiry.
    ///
    /// Returns `Some(TransitionOutcome)` when the pending transition's delay
    /// elapses this tick, or `None` if still counting down / no pending
    /// transition / the condition reverted (which cancels the pending).
    pub fn tick(
        &mut self,
        present_value: f32,
        reliability: Reliability,
    ) -> Option<TransitionOutcome> {
        let outcome = self.tick_proposal(present_value, reliability, false);
        if let Some(ref outcome) = outcome {
            self.confirm_transition(&outcome.change, reliability);
        }
        outcome
    }

    /// Advance a pending countdown, leaving a fire-ready transition retryable.
    pub(crate) fn tick_proposal(
        &mut self,
        present_value: f32,
        reliability: Reliability,
        inhibited: bool,
    ) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_proposal(reliability) {
            return result;
        }
        if inhibited {
            return inhibited_target(self.event_state, &mut self.pending)
                .and_then(|target| self.proposal(target));
        }
        let desired = self.compute_new_state(present_value);
        if desired == self.event_state {
            self.pending = None;
            return None;
        }
        match &mut self.pending {
            Some(p) if p.state == desired => {
                if p.remaining > 1 {
                    p.remaining -= 1;
                    return None;
                }
                self.proposal(desired)
            }
            _ => {
                // Condition changed target mid-delay, or no pending yet: re-seed
                // with the delay for the CURRENT target's direction.
                let delay = delay_toward(self.time_delay, self.time_delay_normal, desired);
                self.pending = Some(PendingTransition::seed(desired, delay));
                None
            }
        }
    }

    /// Confirm a transition to `new_state`: mutate `event_state` and report the
    /// transition along with whether `Event_Enable` permits distributing it.
    ///
    /// The transition is always reported. A cleared `Event_Enable` bit sets
    /// `distribute` to false; it does not withhold the transition, because
    /// none of Clause 13.2.2.1.4's transition actions are `Event_Enable`-scoped
    /// — the property disables external distribution downstream, inside the
    /// event-notification-distribution process (Clause 13.2.5, and Clause 12.12
    /// which defines the property in those terms).
    fn proposal(&self, new_state: EventState) -> Option<TransitionOutcome> {
        let change = EventStateChange {
            from: self.event_state,
            to: new_state,
        };
        let transition_bit = EventTransition::for_target_state(new_state).bit_mask();
        let distribute = self.event_enable.contains(transition_bit);
        let event_type = change.event_type(Self::ALGORITHM);
        Some(TransitionOutcome {
            change,
            event_type,
            distribute,
        })
    }

    /// Finalize detector-local state only after the object commit kernel succeeds.
    pub(crate) fn confirm_transition(
        &mut self,
        change: &EventStateChange,
        reliability: Reliability,
    ) {
        self.event_state = change.to;
        self.pending = None;
        self.fault_reliability = if change.to == EventState::FAULT {
            Some(reliability)
        } else {
            None
        };
    }

    fn compute_new_state(&self, pv: f32) -> EventState {
        let high_enabled = self.limit_enable.contains(LimitEnable::HIGH_LIMIT_ENABLE);
        let low_enabled = self.limit_enable.contains(LimitEnable::LOW_LIMIT_ENABLE);

        match self.event_state {
            s if s == EventState::NORMAL => {
                // Check for HIGH_LIMIT violation first (higher priority)
                if high_enabled && pv > self.high_limit {
                    return EventState::HIGH_LIMIT;
                }
                if low_enabled && pv < self.low_limit {
                    return EventState::LOW_LIMIT;
                }
                EventState::NORMAL
            }
            s if s == EventState::HIGH_LIMIT => {
                // Can transition to LOW_LIMIT directly
                if low_enabled && pv < self.low_limit {
                    return EventState::LOW_LIMIT;
                }
                // Return to NORMAL with deadband
                if pv < self.high_limit - self.deadband {
                    return EventState::NORMAL;
                }
                EventState::HIGH_LIMIT
            }
            s if s == EventState::LOW_LIMIT => {
                // Can transition to HIGH_LIMIT directly
                if high_enabled && pv > self.high_limit {
                    return EventState::HIGH_LIMIT;
                }
                // Return to NORMAL with deadband
                if pv > self.low_limit + self.deadband {
                    return EventState::NORMAL;
                }
                EventState::LOW_LIMIT
            }
            _ => self.event_state, // No change for unknown states
        }
    }
}

// ---------------------------------------------------------------------------
// CHANGE_OF_STATE event detector
// ---------------------------------------------------------------------------

/// CHANGE_OF_STATE event detector for binary and multi-state objects.
///
/// Transitions to OFFNORMAL when the monitored value
/// matches any value in the `alarm_values` list. Returns to NORMAL when
/// the value no longer matches any alarm value.
///
/// `Time_Delay` is honored via the split [`Self::probe`] / [`Self::tick`]
/// entry points; see [`OutOfRangeDetector`] for the delay contract.
#[derive(Debug, Clone)]
pub struct ChangeOfStateDetector {
    /// Values that trigger an OFFNORMAL state.
    pub alarm_values: Vec<u32>,
    /// Instance number of the Notification Class object that distributes the events.
    pub notification_class: u32,
    /// Notify_Type: whether the transitions are reported as alarms or as events.
    pub notify_type: NotifyType,
    /// Event_Enable: the transitions whose notifications are distributed.
    pub event_enable: EventTransitionBits,
    /// Seconds an offnormal condition must persist before TO_OFFNORMAL fires, and the NORMAL
    /// delay too when `time_delay_normal` is `None`; 0 fires immediately. Fault transitions are
    /// never delayed.
    pub time_delay: u32,
    /// `Time_Delay_Normal` (property 356): the Clause 13.3.2 pTimeDelayNormal
    /// parameter — the number of seconds a return to normal has to be
    /// sustained before the detector confirms it. `None` is the not-configured case
    /// and uses `time_delay` as the fallback required for absent pTimeDelayNormal.
    pub time_delay_normal: Option<u32>,
    /// Current event state.
    pub event_state: EventState,
    /// Acked_Transitions: a set flag means that transition was acknowledged.
    pub acked_transitions: EventTransitionBits,
    /// Pending delayed transition, or `None` when no delay is in progress.
    pub pending: Option<PendingTransition>,
    /// Reliability value in force at the last entry to FAULT; `None` outside FAULT.
    pub fault_reliability: Option<Reliability>,
}

impl Default for ChangeOfStateDetector {
    fn default() -> Self {
        Self {
            alarm_values: Vec::new(),
            notification_class: 0,
            notify_type: NotifyType::ALARM,
            event_enable: EventTransitionBits::empty(),
            time_delay: 0,
            time_delay_normal: None,
            event_state: EventState::NORMAL,
            acked_transitions: EventTransitionBits::all(),
            pending: None,
            fault_reliability: None,
        }
    }
}

impl ChangeOfStateDetector {
    /// BACnet Event Type corresponding to this detector's event algorithm.
    pub const ALGORITHM: EventType = EventType::CHANGE_OF_STATE;

    /// Per-write entry point; see [`OutOfRangeDetector::evaluate`].
    pub fn evaluate(
        &mut self,
        present_value: u32,
        reliability: Reliability,
    ) -> Option<TransitionOutcome> {
        self.probe(present_value, reliability)
    }

    /// Clause 13.2.2 fault precedence; see [`OutOfRangeDetector::fault_proposal`].
    fn fault_proposal(&self, reliability: Reliability) -> ControlFlow<Option<TransitionOutcome>> {
        match fault_precedence(reliability, self.fault_reliability, self.event_state) {
            FaultPrecedence::EnterFault | FaultPrecedence::ReenterFault => {
                ControlFlow::Break(self.proposal(EventState::FAULT))
            }
            FaultPrecedence::HoldFault => ControlFlow::Break(None),
            FaultPrecedence::RecoverToNormal => {
                ControlFlow::Break(self.proposal(EventState::NORMAL))
            }
            FaultPrecedence::RunAlgorithm => ControlFlow::Continue(()),
        }
    }

    /// Per-write probe: seed or cancel a pending transition, fire on zero delay.
    pub fn probe(
        &mut self,
        present_value: u32,
        reliability: Reliability,
    ) -> Option<TransitionOutcome> {
        let outcome = self.propose(present_value, reliability, false);
        if let Some(ref outcome) = outcome {
            self.confirm_transition(&outcome.change, reliability);
        }
        outcome
    }

    /// Stage a per-write transition without confirming any transition-owned state.
    pub(crate) fn propose(
        &mut self,
        present_value: u32,
        reliability: Reliability,
        inhibited: bool,
    ) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_proposal(reliability) {
            return result;
        }
        if inhibited {
            return inhibited_target(self.event_state, &mut self.pending)
                .and_then(|target| self.proposal(target));
        }
        let desired = self.compute_new_state(present_value);
        if desired == self.event_state {
            self.pending = None;
            return None;
        }
        let delay = delay_toward(self.time_delay, self.time_delay_normal, desired);
        if delay == 0 {
            return self.proposal(desired);
        }
        // See [`OutOfRangeDetector::probe`]: do not restart an in-flight
        // countdown to the same target on a redundant qualifying write.
        if self.pending.as_ref().is_none_or(|p| p.state != desired) {
            self.pending = Some(PendingTransition::seed(desired, delay));
        }
        None
    }

    /// Periodic tick: advance the countdown and fire on expiry.
    pub fn tick(
        &mut self,
        present_value: u32,
        reliability: Reliability,
    ) -> Option<TransitionOutcome> {
        let outcome = self.tick_proposal(present_value, reliability, false);
        if let Some(ref outcome) = outcome {
            self.confirm_transition(&outcome.change, reliability);
        }
        outcome
    }

    /// Advance a pending countdown, leaving a fire-ready transition retryable.
    pub(crate) fn tick_proposal(
        &mut self,
        present_value: u32,
        reliability: Reliability,
        inhibited: bool,
    ) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_proposal(reliability) {
            return result;
        }
        if inhibited {
            return inhibited_target(self.event_state, &mut self.pending)
                .and_then(|target| self.proposal(target));
        }
        let desired = self.compute_new_state(present_value);
        if desired == self.event_state {
            self.pending = None;
            return None;
        }
        match &mut self.pending {
            Some(p) if p.state == desired => {
                if p.remaining > 1 {
                    p.remaining -= 1;
                    return None;
                }
                self.proposal(desired)
            }
            _ => {
                // Re-seed with the delay for the CURRENT target's direction.
                let delay = delay_toward(self.time_delay, self.time_delay_normal, desired);
                self.pending = Some(PendingTransition::seed(desired, delay));
                None
            }
        }
    }

    /// Confirm a transition to `new_state`, reporting whether `Event_Enable`
    /// permits distributing it. The transition itself is always reported; see
    /// [`TransitionOutcome`].
    fn proposal(&self, new_state: EventState) -> Option<TransitionOutcome> {
        let change = EventStateChange {
            from: self.event_state,
            to: new_state,
        };
        let transition_bit = EventTransition::for_target_state(new_state).bit_mask();
        let distribute = self.event_enable.contains(transition_bit);
        let event_type = change.event_type(Self::ALGORITHM);
        Some(TransitionOutcome {
            change,
            event_type,
            distribute,
        })
    }

    /// Finalize detector-local state only after the object commit kernel succeeds.
    pub(crate) fn confirm_transition(
        &mut self,
        change: &EventStateChange,
        reliability: Reliability,
    ) {
        self.event_state = change.to;
        self.pending = None;
        self.fault_reliability = if change.to == EventState::FAULT {
            Some(reliability)
        } else {
            None
        };
    }

    fn compute_new_state(&self, present_value: u32) -> EventState {
        if self.alarm_values.contains(&present_value) {
            EventState::OFFNORMAL
        } else {
            EventState::NORMAL
        }
    }
}

/// COMMAND_FAILURE event detector for commandable output objects (BO, MSO).
///
/// Transitions to OFFNORMAL when present_value differs
/// from feedback_value. Returns to NORMAL when they match.
///
/// `Time_Delay` is honored via the split [`Self::probe`] / [`Self::tick`]
/// entry points; see [`OutOfRangeDetector`] for the delay contract.
#[derive(Debug, Clone)]
pub struct CommandFailureDetector {
    /// Instance number of the Notification Class object that distributes the events.
    pub notification_class: u32,
    /// Notify_Type: whether the transitions are reported as alarms or as events.
    pub notify_type: NotifyType,
    /// Event_Enable: the transitions whose notifications are distributed.
    pub event_enable: EventTransitionBits,
    /// Seconds an offnormal condition must persist before TO_OFFNORMAL fires, and the NORMAL
    /// delay too when `time_delay_normal` is `None`; 0 fires immediately. Fault transitions are
    /// never delayed.
    pub time_delay: u32,
    /// `Time_Delay_Normal` (property 356): the Clause 13.3.4 pTimeDelayNormal
    /// parameter — the number of seconds a return to normal has to be
    /// sustained before the detector confirms it. `None` is the not-configured case
    /// and uses `time_delay` as the fallback required for absent pTimeDelayNormal.
    pub time_delay_normal: Option<u32>,
    /// Current event state.
    pub event_state: EventState,
    /// Acked_Transitions: a set flag means that transition was acknowledged.
    pub acked_transitions: EventTransitionBits,
    /// Pending delayed transition, or `None` when no delay is in progress.
    pub pending: Option<PendingTransition>,
    /// Reliability value in force at the last entry to FAULT; `None` outside FAULT.
    pub fault_reliability: Option<Reliability>,
}

impl Default for CommandFailureDetector {
    fn default() -> Self {
        Self {
            notification_class: 0,
            notify_type: NotifyType::ALARM,
            event_enable: EventTransitionBits::empty(),
            time_delay: 0,
            time_delay_normal: None,
            event_state: EventState::NORMAL,
            acked_transitions: EventTransitionBits::all(),
            pending: None,
            fault_reliability: None,
        }
    }
}

impl CommandFailureDetector {
    /// BACnet Event Type corresponding to this detector's event algorithm.
    pub const ALGORITHM: EventType = EventType::COMMAND_FAILURE;

    /// Per-write entry point; see [`OutOfRangeDetector::evaluate`].
    pub fn evaluate(
        &mut self,
        present_value: u32,
        feedback_value: u32,
        reliability: Reliability,
    ) -> Option<TransitionOutcome> {
        self.probe(present_value, feedback_value, reliability)
    }

    /// Clause 13.2.2 fault precedence; see [`OutOfRangeDetector::fault_proposal`].
    fn fault_proposal(&self, reliability: Reliability) -> ControlFlow<Option<TransitionOutcome>> {
        match fault_precedence(reliability, self.fault_reliability, self.event_state) {
            FaultPrecedence::EnterFault | FaultPrecedence::ReenterFault => {
                ControlFlow::Break(self.proposal(EventState::FAULT))
            }
            FaultPrecedence::HoldFault => ControlFlow::Break(None),
            FaultPrecedence::RecoverToNormal => {
                ControlFlow::Break(self.proposal(EventState::NORMAL))
            }
            FaultPrecedence::RunAlgorithm => ControlFlow::Continue(()),
        }
    }

    /// Per-write probe: seed or cancel a pending transition, fire on zero delay.
    pub fn probe(
        &mut self,
        present_value: u32,
        feedback_value: u32,
        reliability: Reliability,
    ) -> Option<TransitionOutcome> {
        let outcome = self.propose(present_value, feedback_value, reliability, false);
        if let Some(ref outcome) = outcome {
            self.confirm_transition(&outcome.change, reliability);
        }
        outcome
    }

    /// Stage a per-write transition without confirming any transition-owned state.
    pub(crate) fn propose(
        &mut self,
        present_value: u32,
        feedback_value: u32,
        reliability: Reliability,
        inhibited: bool,
    ) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_proposal(reliability) {
            return result;
        }
        if inhibited {
            return inhibited_target(self.event_state, &mut self.pending)
                .and_then(|target| self.proposal(target));
        }
        let desired = self.compute_new_state(present_value, feedback_value);
        if desired == self.event_state {
            self.pending = None;
            return None;
        }
        let delay = delay_toward(self.time_delay, self.time_delay_normal, desired);
        if delay == 0 {
            return self.proposal(desired);
        }
        // See [`OutOfRangeDetector::probe`]: do not restart an in-flight
        // countdown to the same target on a redundant qualifying write.
        if self.pending.as_ref().is_none_or(|p| p.state != desired) {
            self.pending = Some(PendingTransition::seed(desired, delay));
        }
        None
    }

    /// Periodic tick: advance the countdown and fire on expiry.
    pub fn tick(
        &mut self,
        present_value: u32,
        feedback_value: u32,
        reliability: Reliability,
    ) -> Option<TransitionOutcome> {
        let outcome = self.tick_proposal(present_value, feedback_value, reliability, false);
        if let Some(ref outcome) = outcome {
            self.confirm_transition(&outcome.change, reliability);
        }
        outcome
    }

    /// Advance a pending countdown, leaving a fire-ready transition retryable.
    pub(crate) fn tick_proposal(
        &mut self,
        present_value: u32,
        feedback_value: u32,
        reliability: Reliability,
        inhibited: bool,
    ) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_proposal(reliability) {
            return result;
        }
        if inhibited {
            return inhibited_target(self.event_state, &mut self.pending)
                .and_then(|target| self.proposal(target));
        }
        let desired = self.compute_new_state(present_value, feedback_value);
        if desired == self.event_state {
            self.pending = None;
            return None;
        }
        match &mut self.pending {
            Some(p) if p.state == desired => {
                if p.remaining > 1 {
                    p.remaining -= 1;
                    return None;
                }
                self.proposal(desired)
            }
            _ => {
                // Re-seed with the delay for the CURRENT target's direction.
                let delay = delay_toward(self.time_delay, self.time_delay_normal, desired);
                self.pending = Some(PendingTransition::seed(desired, delay));
                None
            }
        }
    }

    /// Confirm a transition to `new_state`, reporting whether `Event_Enable`
    /// permits distributing it. The transition itself is always reported; see
    /// [`TransitionOutcome`].
    fn proposal(&self, new_state: EventState) -> Option<TransitionOutcome> {
        let change = EventStateChange {
            from: self.event_state,
            to: new_state,
        };
        // The FAULT arm reads `Event_Enable` like the other two. It previously
        // hardcoded `false`, which was unobservable only because no reliability
        // ever reached a detector (#200); Clause 13.2.5 scopes `Event_Enable` to
        // distribution uniformly across all three transition directions, so
        // there is no basis for treating TO_FAULT differently.
        let transition_bit = EventTransition::for_target_state(new_state).bit_mask();
        let distribute = self.event_enable.contains(transition_bit);
        let event_type = change.event_type(Self::ALGORITHM);
        Some(TransitionOutcome {
            change,
            event_type,
            distribute,
        })
    }

    /// Finalize detector-local state only after the object commit kernel succeeds.
    pub(crate) fn confirm_transition(
        &mut self,
        change: &EventStateChange,
        reliability: Reliability,
    ) {
        self.event_state = change.to;
        self.pending = None;
        self.fault_reliability = if change.to == EventState::FAULT {
            Some(reliability)
        } else {
            None
        };
    }

    fn compute_new_state(&self, present_value: u32, feedback_value: u32) -> EventState {
        if present_value != feedback_value {
            EventState::OFFNORMAL
        } else {
            EventState::NORMAL
        }
    }
}

#[cfg(test)]
pub(crate) use history::commit_test_proposal;
pub(crate) use history::impl_builtin_intrinsic_reporting;
pub(crate) use state_reporting::impl_change_of_state_reporting;

#[cfg(test)]
mod fault_tests;
#[cfg(test)]
mod inhibit_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod time_delay_normal_tests;
