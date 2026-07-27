//! Intrinsic reporting — OUT_OF_RANGE event state machine.
//!
//! Per ASHRAE 135-2020 Clause 13.3.6 (Table 13-7), the OUT_OF_RANGE algorithm monitors
//! an analog present_value against HIGH_LIMIT and LOW_LIMIT with a DEADBAND
//! to prevent oscillation at the boundary.

use core::ops::ControlFlow;

use bacnet_types::enums::{EventState, EventType, Reliability};

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
    /// Seconds remaining; seeded with `time_delay` and decremented per tick.
    pub remaining: u32,
}

impl PendingTransition {
    /// Begin a pending transition to `state`, seeded with `time_delay` seconds.
    fn seed(state: EventState, time_delay: u32) -> Self {
        Self {
            state,
            remaining: time_delay,
        }
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
/// The rule is a standing condition, not an edge: Clause 13.2.2.1 states the
/// invariant directly — "In the Fault state reliability-evaluation indicates a
/// value other than NO_FAULT_DETECTED." So this is recomputed from `reliability`
/// on every evaluation and nothing about the fault is stored.
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
    /// Reliability is bad and FAULT already holds: the standing condition is
    /// satisfied, so no transition fires — and the algorithm must not run.
    HoldFault,
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
pub(crate) fn fault_precedence(reliability: u32, current: EventState) -> FaultPrecedence {
    let faulted = reliability != Reliability::NO_FAULT_DETECTED.to_raw();
    match (faulted, current == EventState::FAULT) {
        (true, false) => FaultPrecedence::EnterFault,
        (true, true) => FaultPrecedence::HoldFault,
        (false, true) => FaultPrecedence::RecoverToNormal,
        (false, false) => FaultPrecedence::RunAlgorithm,
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
/// `Time_Delay` is honored via the split [`Self::probe`] / [`Self::tick`]
/// entry points: a present-value write calls `probe`, which seeds a pending
/// transition (or fires immediately when `time_delay == 0`); a one-second
/// periodic task calls `tick` to advance the countdown and fire on expiry.
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
    pub event_state: EventState,
    /// Acknowledged-transitions bitfield (3 bits: TO_OFFNORMAL, TO_FAULT, TO_NORMAL).
    /// A set bit means the corresponding transition has been acknowledged.
    pub acked_transitions: u8,
    /// Pending delayed transition, or `None` when no delay is in progress.
    pub pending: Option<PendingTransition>,
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
            event_state: EventState::NORMAL,
            acked_transitions: 0b111, // all acknowledged by default
            pending: None,
        }
    }
}

impl OutOfRangeDetector {
    /// BACnet Event Type corresponding to this detector's event algorithm.
    pub const ALGORITHM: EventType = EventType::OUT_OF_RANGE;

    /// Evaluate the present value against configured limits.
    ///
    /// This is the per-write entry point: it seeds a pending delayed
    /// transition (or fires immediately when `time_delay == 0`) but never
    /// advances the countdown — so repeated writes to the same value do not
    /// shorten the delay. Returns `Some(TransitionOutcome)` whenever a
    /// transition fires; the outcome's `distribute` flag carries the
    /// `event_enable` bit rather than withholding the transition.
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
    /// `event_state`, `pending` and `fire`; the clause interpretation they share
    /// lives once, in [`fault_precedence`].
    fn fault_step(&mut self, reliability: u32) -> ControlFlow<Option<TransitionOutcome>> {
        match fault_precedence(reliability, self.event_state) {
            FaultPrecedence::EnterFault => {
                // Drop any in-flight countdown. The standard does not say what
                // becomes of one (Clauses 13.2.2, 13.2.2.1 and 13.3 are all
                // silent), and this is the project's ruled choice rather than a
                // quotation: on recovery the machine re-enters NORMAL, so a
                // countdown seeded before the fault targets a transition out of
                // a state the machine no longer occupies. The nearest analogous
                // rule agrees — Clause 13.2.2.1.5 restarts the *full* delay when
                // Event_Algorithm_Inhibit clears rather than resuming a partial
                // one — but it is scoped to inhibition, not faults.
                self.pending = None;
                ControlFlow::Break(self.fire(EventState::FAULT))
            }
            FaultPrecedence::HoldFault => ControlFlow::Break(None),
            FaultPrecedence::RecoverToNormal => {
                // Unreachable today, and kept deliberately: `EnterFault` above
                // already cleared `pending`, and `HoldFault` returns before the
                // algorithm can seed a new one, so nothing can be in flight
                // while FAULT holds. Mutation testing confirms no test fails
                // when this line is removed. It stays because it is the
                // invariant this arm relies on rather than dead weight — were
                // `HoldFault` ever to let the algorithm run, its absence would
                // silently resurrect a stale countdown across the fault.
                self.pending = None;
                ControlFlow::Break(self.fire(EventState::NORMAL))
            }
            FaultPrecedence::RunAlgorithm => ControlFlow::Continue(()),
        }
    }

    /// Per-write probe: seed or cancel a pending transition, fire on zero delay.
    ///
    /// When `time_delay == 0` the transition is confirmed immediately and
    /// `event_state` is updated, preserving the legacy instant-transition
    /// behavior. Otherwise a [`PendingTransition`] is seeded (or cleared if
    /// the condition reverted) and `None` is returned; the periodic
    /// [`Self::tick`] advances and eventually confirms it.
    pub fn probe(&mut self, present_value: f32, reliability: u32) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_step(reliability) {
            return result;
        }
        let desired = self.compute_new_state(present_value);
        if desired == self.event_state {
            // Condition reverted to the confirmed state: cancel any pending
            // transition without firing or notifying.
            self.pending = None;
            return None;
        }
        if self.time_delay == 0 {
            return self.fire(desired);
        }
        // Nonzero delay: seed a pending transition only when there is none to
        // the same target. A redundant write of the same qualifying value must
        // NOT restart the countdown (ASHRAE 135-2020 §13.2.4 — Time_Delay is a
        // debounce timer); re-seeding here would let writes faster than the
        // 1s tick pin the transition forever. The periodic `tick` advances it.
        if self.pending.as_ref().map_or(true, |p| p.state != desired) {
            self.pending = Some(PendingTransition::seed(desired, self.time_delay));
        }
        None
    }

    /// Periodic tick: advance the countdown and fire on expiry.
    ///
    /// Returns `Some(TransitionOutcome)` when the pending transition's delay
    /// elapses this tick, or `None` if still counting down / no pending
    /// transition / the condition reverted (which cancels the pending).
    pub fn tick(&mut self, present_value: f32, reliability: u32) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_step(reliability) {
            return result;
        }
        let desired = self.compute_new_state(present_value);
        if desired == self.event_state {
            self.pending = None;
            return None;
        }
        match &mut self.pending {
            Some(p) if p.state == desired => {
                if p.remaining > 0 {
                    p.remaining -= 1;
                }
                if p.remaining == 0 {
                    self.pending = None;
                    return self.fire(desired);
                }
                None
            }
            _ => {
                // Condition changed target mid-delay, or no pending yet: re-seed.
                self.pending = Some(PendingTransition::seed(desired, self.time_delay));
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
    fn fire(&mut self, new_state: EventState) -> Option<TransitionOutcome> {
        let change = EventStateChange {
            from: self.event_state,
            to: new_state,
        };
        self.event_state = new_state;
        let transition_bit = EventTransition::for_target_state(new_state).bit_mask();
        let distribute = self.event_enable & transition_bit != 0;
        let event_type = change.event_type(Self::ALGORITHM);
        Some(TransitionOutcome {
            change,
            event_type,
            distribute,
        })
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
    pub event_state: EventState,
    pub acked_transitions: u8,
    /// Pending delayed transition, or `None` when no delay is in progress.
    pub pending: Option<PendingTransition>,
}

impl Default for ChangeOfStateDetector {
    fn default() -> Self {
        Self {
            alarm_values: Vec::new(),
            notification_class: 0,
            notify_type: 0,
            event_enable: 0,
            time_delay: 0,
            event_state: EventState::NORMAL,
            acked_transitions: 0b111,
            pending: None,
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

    /// Clause 13.2.2 fault precedence; see [`OutOfRangeDetector::fault_step`].
    fn fault_step(&mut self, reliability: u32) -> ControlFlow<Option<TransitionOutcome>> {
        match fault_precedence(reliability, self.event_state) {
            FaultPrecedence::EnterFault => {
                self.pending = None;
                ControlFlow::Break(self.fire(EventState::FAULT))
            }
            FaultPrecedence::HoldFault => ControlFlow::Break(None),
            FaultPrecedence::RecoverToNormal => {
                self.pending = None;
                ControlFlow::Break(self.fire(EventState::NORMAL))
            }
            FaultPrecedence::RunAlgorithm => ControlFlow::Continue(()),
        }
    }

    /// Per-write probe: seed or cancel a pending transition, fire on zero delay.
    pub fn probe(&mut self, present_value: u32, reliability: u32) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_step(reliability) {
            return result;
        }
        let desired = self.compute_new_state(present_value);
        if desired == self.event_state {
            self.pending = None;
            return None;
        }
        if self.time_delay == 0 {
            return self.fire(desired);
        }
        // See [`OutOfRangeDetector::probe`]: do not restart an in-flight
        // countdown to the same target on a redundant qualifying write.
        if self.pending.as_ref().map_or(true, |p| p.state != desired) {
            self.pending = Some(PendingTransition::seed(desired, self.time_delay));
        }
        None
    }

    /// Periodic tick: advance the countdown and fire on expiry.
    pub fn tick(&mut self, present_value: u32, reliability: u32) -> Option<TransitionOutcome> {
        if let ControlFlow::Break(result) = self.fault_step(reliability) {
            return result;
        }
        let desired = self.compute_new_state(present_value);
        if desired == self.event_state {
            self.pending = None;
            return None;
        }
        match &mut self.pending {
            Some(p) if p.state == desired => {
                if p.remaining > 0 {
                    p.remaining -= 1;
                }
                if p.remaining == 0 {
                    self.pending = None;
                    return self.fire(desired);
                }
                None
            }
            _ => {
                self.pending = Some(PendingTransition::seed(desired, self.time_delay));
                None
            }
        }
    }

    /// Confirm a transition to `new_state`, reporting whether `Event_Enable`
    /// permits distributing it. The transition itself is always reported; see
    /// [`TransitionOutcome`].
    fn fire(&mut self, new_state: EventState) -> Option<TransitionOutcome> {
        let change = EventStateChange {
            from: self.event_state,
            to: new_state,
        };
        self.event_state = new_state;
        let transition_bit = EventTransition::for_target_state(new_state).bit_mask();
        let distribute = self.event_enable & transition_bit != 0;
        let event_type = change.event_type(Self::ALGORITHM);
        Some(TransitionOutcome {
            change,
            event_type,
            distribute,
        })
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
    pub event_state: EventState,
    pub acked_transitions: u8,
    /// Pending delayed transition, or `None` when no delay is in progress.
    pub pending: Option<PendingTransition>,
}

impl Default for CommandFailureDetector {
    fn default() -> Self {
        Self {
            notification_class: 0,
            notify_type: 0,
            event_enable: 0,
            time_delay: 0,
            event_state: EventState::NORMAL,
            acked_transitions: 0b111,
            pending: None,
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

    /// Clause 13.2.2 fault precedence; see [`OutOfRangeDetector::fault_step`].
    fn fault_step(&mut self, reliability: u32) -> ControlFlow<Option<TransitionOutcome>> {
        match fault_precedence(reliability, self.event_state) {
            FaultPrecedence::EnterFault => {
                self.pending = None;
                ControlFlow::Break(self.fire(EventState::FAULT))
            }
            FaultPrecedence::HoldFault => ControlFlow::Break(None),
            FaultPrecedence::RecoverToNormal => {
                self.pending = None;
                ControlFlow::Break(self.fire(EventState::NORMAL))
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
        if let ControlFlow::Break(result) = self.fault_step(reliability) {
            return result;
        }
        let desired = self.compute_new_state(present_value, feedback_value);
        if desired == self.event_state {
            self.pending = None;
            return None;
        }
        if self.time_delay == 0 {
            return self.fire(desired);
        }
        // See [`OutOfRangeDetector::probe`]: do not restart an in-flight
        // countdown to the same target on a redundant qualifying write.
        if self.pending.as_ref().map_or(true, |p| p.state != desired) {
            self.pending = Some(PendingTransition::seed(desired, self.time_delay));
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
        if let ControlFlow::Break(result) = self.fault_step(reliability) {
            return result;
        }
        let desired = self.compute_new_state(present_value, feedback_value);
        if desired == self.event_state {
            self.pending = None;
            return None;
        }
        match &mut self.pending {
            Some(p) if p.state == desired => {
                if p.remaining > 0 {
                    p.remaining -= 1;
                }
                if p.remaining == 0 {
                    self.pending = None;
                    return self.fire(desired);
                }
                None
            }
            _ => {
                self.pending = Some(PendingTransition::seed(desired, self.time_delay));
                None
            }
        }
    }

    /// Confirm a transition to `new_state`, reporting whether `Event_Enable`
    /// permits distributing it. The transition itself is always reported; see
    /// [`TransitionOutcome`].
    fn fire(&mut self, new_state: EventState) -> Option<TransitionOutcome> {
        let change = EventStateChange {
            from: self.event_state,
            to: new_state,
        };
        self.event_state = new_state;
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

    fn compute_new_state(&self, present_value: u32, feedback_value: u32) -> EventState {
        if present_value != feedback_value {
            EventState::OFFNORMAL
        } else {
            EventState::NORMAL
        }
    }
}

/// Implement the `BACnetObject` intrinsic-reporting trait methods for an
/// object whose detector is exposed as `self.event_detector`, whose present
/// value is `self.present_value`, and whose reliability is `self.reliability`.
///
/// This wires both the per-write [`evaluate_intrinsic_reporting`](crate::traits::BACnetObject::evaluate_intrinsic_reporting)
/// probe and the periodic [`tick_intrinsic_reporting`](crate::traits::BACnetObject::tick_intrinsic_reporting)
/// tick to the detector's split `probe` / `tick` entry points, honoring
/// `Time_Delay` without the object types repeating the delegation.
///
/// Reliability is passed on every call rather than only when it changes: per
/// ASHRAE 135-2020 Clause 13.2.2 the FAULT determination is a standing
/// condition, so the detector re-derives it each evaluation. That also means a
/// Reliability written by any route — the server's fault detector, a local
/// write, or a network write — reaches event-state-detection without each route
/// needing to notify it.
#[macro_export]
macro_rules! impl_intrinsic_reporting {
    ($detector_field:ident, $present_value_field:ident, $reliability_field:ident) => {
        fn evaluate_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            self.$detector_field
                .probe(self.$present_value_field, self.$reliability_field)
        }

        fn tick_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            self.$detector_field
                .tick(self.$present_value_field, self.$reliability_field)
        }
    };
}

#[cfg(test)]
mod fault_tests;
#[cfg(test)]
mod tests;
