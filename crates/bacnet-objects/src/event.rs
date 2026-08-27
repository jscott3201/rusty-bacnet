//! Intrinsic reporting — OUT_OF_RANGE event state machine.
//!
//! Per ASHRAE 135-2020 Clause 13.3.6 (Table 13-7), the OUT_OF_RANGE algorithm monitors
//! an analog present_value against HIGH_LIMIT and LOW_LIMIT with a DEADBAND
//! to prevent oscillation at the boundary.

use core::ops::ControlFlow;

use bacnet_types::enums::{EventState, EventType, Reliability};
use bacnet_types::primitives::BACnetTimeStamp;

pub(crate) mod history;

/// A detected change in event state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventStateChange {
    /// The previous event state.
    pub from: EventState,
    /// The new event state.
    pub to: EventState,
}

/// A transition that occurred, and whether it may be distributed.
///
/// ASHRAE 135-2020 Clause 13.2.2.1.4 mandates four actions on every transition:
/// store the new `Event_State`, store the time in `Event_Time_Stamps`, store the
/// message text in `Event_Message_Texts` *if present*, and indicate the
/// transition to the alarm-acknowledgment and notification-distribution
/// processes. None of the four is `Event_Enable`-scoped; the property disables
/// external distribution downstream (Clause 13.2.5), so a cleared bit must not
/// suppress the first three actions, nor the alarm-acknowledgment half of the
/// fourth.
///
/// Separating the two answers keeps that distinction in the type: `None` from a
/// detector means no transition occurred, while `distribute == false` means one
/// occurred and must be recorded but not sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionOutcome {
    /// The transition itself. Always recorded, whatever `distribute` says.
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
    /// Otherwise Clauses 13.8.1.1 and 13.9.1.1 require the Event Type
    /// associated with the event-initiating object's configured event
    /// algorithm, supplied here as `algorithm`.
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

impl EventTransition {
    /// Classify a destination event state under ASHRAE 135-2020 Clause 13.2.
    ///
    /// "All states that are not normal and not fault are offnormal states,"
    /// while Clause 13.2.2.1.2 confirms that "the OffNormal state includes all
    /// event states other than NORMAL and FAULT". Therefore the residual case
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

    /// Bit mask for this transition in the `BACnetDestination.transitions` field.
    ///
    /// bit 0 = TO_OFFNORMAL, bit 1 = TO_FAULT, bit 2 = TO_NORMAL.
    pub fn bit_mask(self) -> u8 {
        match self {
            EventTransition::ToOffnormal => 0x01,
            EventTransition::ToFault => 0x02,
            EventTransition::ToNormal => 0x04,
        }
    }

    /// Positional index into the NotificationClass `PRIORITY` and
    /// `ACK_REQUIRED` arrays, both ordered `[TO_OFFNORMAL, TO_FAULT,
    /// TO_NORMAL]` per ASHRAE 135-2020 Clause 12.31.5 / 12.31.6.
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

/// Which limits are enabled.
///
/// Encoded as a BACnet BIT STRING: bit 0 = low_limit_enable, bit 1 = high_limit_enable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitEnable {
    pub low_limit_enable: bool,
    pub high_limit_enable: bool,
}

impl LimitEnable {
    pub const NONE: Self = Self {
        low_limit_enable: false,
        high_limit_enable: false,
    };

    pub const BOTH: Self = Self {
        low_limit_enable: true,
        high_limit_enable: true,
    };

    /// Encode as a BACnet bitstring byte (2 bits used, 6 unused).
    pub fn to_bits(self) -> u8 {
        let mut bits = 0u8;
        if self.low_limit_enable {
            bits |= 0x80; // bit 0 (MSB first)
        }
        if self.high_limit_enable {
            bits |= 0x40; // bit 1
        }
        bits
    }

    /// Decode from a BACnet bitstring byte.
    pub fn from_bits(byte: u8) -> Self {
        Self {
            low_limit_enable: byte & 0x80 != 0,
            high_limit_enable: byte & 0x40 != 0,
        }
    }
}

/// Pending (delayed) intrinsic-reporting transition state, shared by every
/// detector that honors [`OutOfRangeDetector::time_delay`] and its peers.
///
/// Per ASHRAE 135-2020 Clause 13.2.4, a transition to a new `EventState` is
/// delayed by `Time_Delay` seconds. While the delay counts down the
/// observable `event_state` stays at the *old* (confirmed) state; if the
/// triggering condition clears before the delay elapses the pending
/// transition is cancelled and no notification is sent.
///
/// The countdown advances once per elapsed wall-clock second via
/// [`PendingTransition::tick`], never per detector call — so a fast poll
/// loop writing the same out-of-range value cannot shorten the delay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTransition {
    /// The state the detector wants to transition to once the delay elapses.
    pub state: EventState,
    /// Seconds remaining; seeded with the direction-appropriate delay (see
    /// [`delay_toward`]) and decremented per tick.
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
/// delays. pTimeDelay is "the time, in seconds, that the offnormal conditions
/// must exist before an offnormal event state is indicated": it governs
/// every indication into an OFFNORMAL state, including offnormal→offnormal
/// re-indication (CHANGE_OF_STATE (c) at 13.3.2, OUT_OF_RANGE (d)/(g) at
/// 13.3.6, COMMAND_FAILURE (a) at 13.3.4). pTimeDelayNormal is "the time, in
/// seconds, that the Normal conditions must exist before a NORMAL event
/// state is indicated" and gates only the sustained-condition return to
/// NORMAL (CHANGE_OF_STATE (b), COMMAND_FAILURE (b), OUT_OF_RANGE (e)/(h)).
///
/// The fallback for the absent case is normative text: "If no value is
/// available for this parameter, then it takes on the value of the
/// pTimeDelay parameter" — so `None` behaves exactly as `time_delay`, never
/// as an error or a zero.
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

/// What Clause 13.2.2's fault-precedence rule dictates for a single evaluation.
///
/// ASHRAE 135-2020 Clause 13.2.2: "The event algorithm determines the normal or
/// offnormal states and the Reliability property determines whether or not the
/// event state will indicate a fault. Fault detection takes precedence over the
/// detection of normal and offnormal states. As such, when Reliability has a
/// value other than NO_FAULT_DETECTED, the event-state-detection process will
/// determine the object's event state to be FAULT."
///
/// **Whether FAULT holds is a standing condition; whether a transition fires is
/// an edge.** Clause 13.2.2.1 states the first directly — "In the Fault state
/// reliability-evaluation indicates a value other than NO_FAULT_DETECTED" — so
/// the FAULT determination is re-derived from `reliability` on every evaluation
/// and is never latched. But the same clause's ToFault transition fires on "a
/// **different** Reliability value", which is an edge and cannot be derived from
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
    /// Clause 13.2.2.1's ToFault transitions are unconditional and carry no
    /// delay term — "If reliability-evaluation indicates a value other than
    /// NO_FAULT_DETECTED, then perform the corresponding transition actions and
    /// enter the Fault state." `Time_Delay` belongs to the event algorithm
    /// (Clause 13.3.1 defines pTimeDelay as the time "that the offnormal
    /// conditions must exist before an offnormal event state is indicated"), and
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
    /// Clause 13.2.2.1's Fault ToFault transition: "If reliability-evaluation
    /// indicates a different Reliability value and the new Reliability value is
    /// not NO_FAULT_DETECTED ... then perform the corresponding transition
    /// actions and re-enter the Fault state."
    ///
    /// Also selected when FAULT holds with no recorded value — a state this
    /// crate never produces but a downstream implementor can construct, since
    /// both fields are public. See the comment on that match arm.
    ReenterFault,
    /// Reliability recovered while in FAULT.
    ///
    /// Clause 13.2.2.1's Fault ToNormal transition: "If reliability-evaluation
    /// indicates a value of NO_FAULT_DETECTED, then perform the corresponding
    /// transition actions and enter the Normal state." **NORMAL specifically —
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
    reliability: u32,
    fault_reliability: Option<u32>,
    current: EventState,
) -> FaultPrecedence {
    let faulted = reliability != Reliability::NO_FAULT_DETECTED.to_raw();
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
/// [`delay_toward`]); a one-second periodic task calls `tick` to advance the
/// countdown and fire on expiry.
#[derive(Debug, Clone)]
pub struct OutOfRangeDetector {
    pub high_limit: f32,
    pub low_limit: f32,
    pub deadband: f32,
    pub limit_enable: LimitEnable,
    pub notification_class: u32,
    pub notify_type: u32,
    pub event_enable: u8,
    pub time_delay: u32,
    /// `Time_Delay_Normal` (property 356): the Clause 13.3.6 pTimeDelayNormal
    /// parameter — seconds that Normal conditions must persist before a
    /// NORMAL event state is indicated. `None` is the not-configured case
    /// and takes on `time_delay`: "If no value is available for this
    /// parameter, then it takes on the value of the pTimeDelay parameter."
    pub time_delay_normal: Option<u32>,
    pub event_state: EventState,
    /// Acknowledged-transitions bitfield (3 bits: TO_OFFNORMAL, TO_FAULT, TO_NORMAL).
    /// A set bit means the corresponding transition has been acknowledged.
    pub acked_transitions: u8,
    /// Pending delayed transition, or `None` when no delay is in progress.
    pub pending: Option<PendingTransition>,
    /// Reliability value in force at the last entry to FAULT; `None` outside FAULT.
    pub fault_reliability: Option<u32>,
}

impl Default for OutOfRangeDetector {
    fn default() -> Self {
        Self {
            high_limit: 100.0,
            low_limit: 0.0,
            deadband: 1.0,
            limit_enable: LimitEnable::NONE,
            notification_class: 0,
            notify_type: 0, // ALARM
            event_enable: 0,
            time_delay: 0,
            time_delay_normal: None,
            event_state: EventState::NORMAL,
            acked_transitions: 0b111, // all acknowledged by default
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
    pub fn evaluate(&mut self, present_value: f32, reliability: u32) -> Option<TransitionOutcome> {
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
    fn fault_proposal(&self, reliability: u32) -> ControlFlow<Option<TransitionOutcome>> {
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
    /// When the direction-appropriate delay ([`delay_toward`]) is zero the
    /// transition is confirmed immediately and `event_state` is updated,
    /// preserving the legacy instant-transition behavior. Otherwise a
    /// [`PendingTransition`] is seeded (or cleared if the condition
    /// reverted) and `None` is returned; the periodic [`Self::tick`]
    /// advances and eventually confirms it.
    pub fn probe(&mut self, present_value: f32, reliability: u32) -> Option<TransitionOutcome> {
        let outcome = self.propose(present_value, reliability);
        if let Some(ref outcome) = outcome {
            self.confirm_transition(&outcome.change, reliability);
        }
        outcome
    }

    /// Stage a per-write transition without confirming any transition-owned state.
    pub(crate) fn propose(
        &mut self,
        present_value: f32,
        reliability: u32,
    ) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_proposal(reliability) {
            return result;
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
        // NOT restart the countdown (ASHRAE 135-2020 §13.2.4 — Time_Delay is a
        // debounce timer); re-seeding here would let writes faster than the
        // 1s tick pin the transition forever. The periodic `tick` advances it.
        if self.pending.as_ref().map_or(true, |p| p.state != desired) {
            self.pending = Some(PendingTransition::seed(desired, delay));
        }
        None
    }

    /// Periodic tick: advance the countdown and fire on expiry.
    ///
    /// Returns `Some(TransitionOutcome)` when the pending transition's delay
    /// elapses this tick, or `None` if still counting down / no pending
    /// transition / the condition reverted (which cancels the pending).
    pub fn tick(&mut self, present_value: f32, reliability: u32) -> Option<TransitionOutcome> {
        let outcome = self.tick_proposal(present_value, reliability);
        if let Some(ref outcome) = outcome {
            self.confirm_transition(&outcome.change, reliability);
        }
        outcome
    }

    /// Advance a pending countdown, leaving a fire-ready transition retryable.
    pub(crate) fn tick_proposal(
        &mut self,
        present_value: f32,
        reliability: u32,
    ) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_proposal(reliability) {
            return result;
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
        let distribute = self.event_enable & transition_bit != 0;
        let event_type = change.event_type(Self::ALGORITHM);
        Some(TransitionOutcome {
            change,
            event_type,
            distribute,
        })
    }

    /// Finalize detector-local state only after the object commit kernel succeeds.
    pub(crate) fn confirm_transition(&mut self, change: &EventStateChange, reliability: u32) {
        self.event_state = change.to;
        self.pending = None;
        self.fault_reliability = if change.to == EventState::FAULT {
            Some(reliability)
        } else {
            None
        };
    }

    fn compute_new_state(&self, pv: f32) -> EventState {
        let high_enabled = self.limit_enable.high_limit_enable;
        let low_enabled = self.limit_enable.low_limit_enable;

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
    pub notification_class: u32,
    pub notify_type: u32,
    pub event_enable: u8,
    pub time_delay: u32,
    /// `Time_Delay_Normal` (property 356): the Clause 13.3.2 pTimeDelayNormal
    /// parameter — seconds that Normal conditions must persist before a
    /// NORMAL event state is indicated. `None` is the not-configured case
    /// and takes on `time_delay`: "If no value is available for this
    /// parameter, then it takes on the value of the pTimeDelay parameter."
    pub time_delay_normal: Option<u32>,
    pub event_state: EventState,
    pub acked_transitions: u8,
    /// Pending delayed transition, or `None` when no delay is in progress.
    pub pending: Option<PendingTransition>,
    /// Reliability value in force at the last entry to FAULT; `None` outside FAULT.
    pub fault_reliability: Option<u32>,
}

impl Default for ChangeOfStateDetector {
    fn default() -> Self {
        Self {
            alarm_values: Vec::new(),
            notification_class: 0,
            notify_type: 0,
            event_enable: 0,
            time_delay: 0,
            time_delay_normal: None,
            event_state: EventState::NORMAL,
            acked_transitions: 0b111,
            pending: None,
            fault_reliability: None,
        }
    }
}

impl ChangeOfStateDetector {
    /// BACnet Event Type corresponding to this detector's event algorithm.
    pub const ALGORITHM: EventType = EventType::CHANGE_OF_STATE;

    /// Per-write entry point; see [`OutOfRangeDetector::evaluate`].
    pub fn evaluate(&mut self, present_value: u32, reliability: u32) -> Option<TransitionOutcome> {
        self.probe(present_value, reliability)
    }

    /// Clause 13.2.2 fault precedence; see [`OutOfRangeDetector::fault_proposal`].
    fn fault_proposal(&self, reliability: u32) -> ControlFlow<Option<TransitionOutcome>> {
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
    pub fn probe(&mut self, present_value: u32, reliability: u32) -> Option<TransitionOutcome> {
        let outcome = self.propose(present_value, reliability);
        if let Some(ref outcome) = outcome {
            self.confirm_transition(&outcome.change, reliability);
        }
        outcome
    }

    /// Stage a per-write transition without confirming any transition-owned state.
    pub(crate) fn propose(
        &mut self,
        present_value: u32,
        reliability: u32,
    ) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_proposal(reliability) {
            return result;
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
        if self.pending.as_ref().map_or(true, |p| p.state != desired) {
            self.pending = Some(PendingTransition::seed(desired, delay));
        }
        None
    }

    /// Periodic tick: advance the countdown and fire on expiry.
    pub fn tick(&mut self, present_value: u32, reliability: u32) -> Option<TransitionOutcome> {
        let outcome = self.tick_proposal(present_value, reliability);
        if let Some(ref outcome) = outcome {
            self.confirm_transition(&outcome.change, reliability);
        }
        outcome
    }

    /// Advance a pending countdown, leaving a fire-ready transition retryable.
    pub(crate) fn tick_proposal(
        &mut self,
        present_value: u32,
        reliability: u32,
    ) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_proposal(reliability) {
            return result;
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
        let distribute = self.event_enable & transition_bit != 0;
        let event_type = change.event_type(Self::ALGORITHM);
        Some(TransitionOutcome {
            change,
            event_type,
            distribute,
        })
    }

    /// Finalize detector-local state only after the object commit kernel succeeds.
    pub(crate) fn confirm_transition(&mut self, change: &EventStateChange, reliability: u32) {
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
    pub notification_class: u32,
    pub notify_type: u32,
    pub event_enable: u8,
    pub time_delay: u32,
    /// `Time_Delay_Normal` (property 356): the Clause 13.3.4 pTimeDelayNormal
    /// parameter — seconds that Normal conditions must persist before a
    /// NORMAL event state is indicated. `None` is the not-configured case
    /// and takes on `time_delay`: "If no value is available for this
    /// parameter, then it takes on the value of the pTimeDelay parameter."
    pub time_delay_normal: Option<u32>,
    pub event_state: EventState,
    pub acked_transitions: u8,
    /// Pending delayed transition, or `None` when no delay is in progress.
    pub pending: Option<PendingTransition>,
    /// Reliability value in force at the last entry to FAULT; `None` outside FAULT.
    pub fault_reliability: Option<u32>,
}

impl Default for CommandFailureDetector {
    fn default() -> Self {
        Self {
            notification_class: 0,
            notify_type: 0,
            event_enable: 0,
            time_delay: 0,
            time_delay_normal: None,
            event_state: EventState::NORMAL,
            acked_transitions: 0b111,
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
        reliability: u32,
    ) -> Option<TransitionOutcome> {
        self.probe(present_value, feedback_value, reliability)
    }

    /// Clause 13.2.2 fault precedence; see [`OutOfRangeDetector::fault_proposal`].
    fn fault_proposal(&self, reliability: u32) -> ControlFlow<Option<TransitionOutcome>> {
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
        reliability: u32,
    ) -> Option<TransitionOutcome> {
        let outcome = self.propose(present_value, feedback_value, reliability);
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
        reliability: u32,
    ) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_proposal(reliability) {
            return result;
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
        if self.pending.as_ref().map_or(true, |p| p.state != desired) {
            self.pending = Some(PendingTransition::seed(desired, delay));
        }
        None
    }

    /// Periodic tick: advance the countdown and fire on expiry.
    pub fn tick(
        &mut self,
        present_value: u32,
        feedback_value: u32,
        reliability: u32,
    ) -> Option<TransitionOutcome> {
        let outcome = self.tick_proposal(present_value, feedback_value, reliability);
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
        reliability: u32,
    ) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_proposal(reliability) {
            return result;
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
        let distribute = self.event_enable & transition_bit != 0;
        let event_type = change.event_type(Self::ALGORITHM);
        Some(TransitionOutcome {
            change,
            event_type,
            distribute,
        })
    }

    /// Finalize detector-local state only after the object commit kernel succeeds.
    pub(crate) fn confirm_transition(&mut self, change: &EventStateChange, reliability: u32) {
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

/// Implement legacy immediate intrinsic-reporting detector delegation.
///
/// This exported macro preserves the downstream behavior in which detector
/// `probe` and `tick` calls immediately update detector-local state.
#[macro_export]
macro_rules! impl_intrinsic_reporting {
    (
        $detector_field:ident,
        $present_value_field:ident,
        $feedback_value_field:ident,
        $reliability_field:ident,
        $event_detection_enable_field:ident
    ) => {
        fn evaluate_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            if !self.$event_detection_enable_field {
                return None;
            }
            self.$detector_field.probe(
                self.$present_value_field,
                self.$feedback_value_field,
                self.$reliability_field,
            )
        }

        fn tick_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            if !self.$event_detection_enable_field {
                return None;
            }
            self.$detector_field.tick(
                self.$present_value_field,
                self.$feedback_value_field,
                self.$reliability_field,
            )
        }
    };
    // Gated two-input detector delegation for intrinsic-reporting object types without a
    // feedback value.
    (
        $detector_field:ident,
        $present_value_field:ident,
        $reliability_field:ident,
        $event_detection_enable_field:ident
    ) => {
        fn evaluate_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            if !self.$event_detection_enable_field {
                return None;
            }
            self.$detector_field
                .probe(self.$present_value_field, self.$reliability_field)
        }

        fn tick_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            if !self.$event_detection_enable_field {
                return None;
            }
            self.$detector_field
                .tick(self.$present_value_field, self.$reliability_field)
        }
    }; // There is deliberately no ungated arm. Exporting one would let downstream
       // implementors wire event-state detection permanently on despite Clause 13.2.2.1.
}

#[cfg(test)]
mod fault_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod time_delay_normal_tests;
