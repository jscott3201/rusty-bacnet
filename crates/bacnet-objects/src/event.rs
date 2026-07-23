//! Intrinsic reporting — OUT_OF_RANGE event state machine.
//!
//! Per ASHRAE 135-2020 Clause 13.3.2, the OUT_OF_RANGE algorithm monitors
//! an analog present_value against HIGH_LIMIT and LOW_LIMIT with a DEADBAND
//! to prevent oscillation at the boundary.

use bacnet_types::enums::{EventState, EventType};

/// A detected change in event state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventStateChange {
    /// The previous event state.
    pub from: EventState,
    /// The new event state.
    pub to: EventState,
}

impl EventStateChange {
    /// Derive the BACnet EventType from the state transition.
    ///
    /// If either the `from` or `to` state is `HIGH_LIMIT` or `LOW_LIMIT`,
    /// the event type is `OUT_OF_RANGE`. Otherwise it is `CHANGE_OF_STATE`.
    pub fn event_type(&self) -> EventType {
        if self.from == EventState::HIGH_LIMIT
            || self.from == EventState::LOW_LIMIT
            || self.to == EventState::HIGH_LIMIT
            || self.to == EventState::LOW_LIMIT
        {
            EventType::OUT_OF_RANGE
        } else {
            EventType::CHANGE_OF_STATE
        }
    }

    /// Derive the event transition category from the state change.
    ///
    /// - `to == NORMAL` -> `ToNormal`
    /// - `to == FAULT` -> `ToFault`
    /// - Everything else (OFFNORMAL, HIGH_LIMIT, LOW_LIMIT) -> `ToOffnormal`
    pub fn transition(&self) -> EventTransition {
        if self.to == EventState::NORMAL {
            EventTransition::ToNormal
        } else if self.to == EventState::FAULT {
            EventTransition::ToFault
        } else {
            EventTransition::ToOffnormal
        }
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
    /// Event_Enable bit masks.
    const TO_OFFNORMAL: u8 = 0x01;
    const TO_FAULT: u8 = 0x02;
    const TO_NORMAL: u8 = 0x04;

    /// Evaluate the present value against configured limits.
    ///
    /// This is the per-write entry point: it seeds a pending delayed
    /// transition (or fires immediately when `time_delay == 0`) but never
    /// advances the countdown — so repeated writes to the same value do not
    /// shorten the delay. Returns `Some(EventStateChange)` only when a
    /// transition fires **and** the corresponding `event_enable` bit is set.
    pub fn evaluate(&mut self, present_value: f32) -> Option<EventStateChange> {
        self.probe(present_value)
    }

    /// Per-write probe: seed or cancel a pending transition, fire on zero delay.
    ///
    /// When `time_delay == 0` the transition is confirmed immediately and
    /// `event_state` is updated, preserving the legacy instant-transition
    /// behavior. Otherwise a [`PendingTransition`] is seeded (or cleared if
    /// the condition reverted) and `None` is returned; the periodic
    /// [`Self::tick`] advances and eventually confirms it.
    pub fn probe(&mut self, present_value: f32) -> Option<EventStateChange> {
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
    /// Returns `Some(EventStateChange)` when the pending transition's delay
    /// elapses this tick, or `None` if still counting down / no pending
    /// transition / the condition reverted (which cancels the pending).
    pub fn tick(&mut self, present_value: f32) -> Option<EventStateChange> {
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

    /// Confirm a transition to `new_state`: mutate `event_state` and, if the
    /// matching `event_enable` bit is set, return the change for notification.
    fn fire(&mut self, new_state: EventState) -> Option<EventStateChange> {
        let change = EventStateChange {
            from: self.event_state,
            to: new_state,
        };
        self.event_state = new_state;
        let enabled = match new_state {
            s if s == EventState::NORMAL => self.event_enable & Self::TO_NORMAL != 0,
            s if s == EventState::HIGH_LIMIT || s == EventState::LOW_LIMIT => {
                self.event_enable & Self::TO_OFFNORMAL != 0
            }
            _ => self.event_enable & Self::TO_FAULT != 0,
        };
        enabled.then_some(change)
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
    const TO_OFFNORMAL: u8 = 0x01;
    const TO_FAULT: u8 = 0x02;
    const TO_NORMAL: u8 = 0x04;

    /// Per-write entry point; see [`OutOfRangeDetector::evaluate`].
    pub fn evaluate(&mut self, present_value: u32) -> Option<EventStateChange> {
        self.probe(present_value)
    }

    /// Per-write probe: seed or cancel a pending transition, fire on zero delay.
    pub fn probe(&mut self, present_value: u32) -> Option<EventStateChange> {
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
    pub fn tick(&mut self, present_value: u32) -> Option<EventStateChange> {
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

    /// Confirm a transition to `new_state`, gated by `event_enable`.
    fn fire(&mut self, new_state: EventState) -> Option<EventStateChange> {
        let change = EventStateChange {
            from: self.event_state,
            to: new_state,
        };
        self.event_state = new_state;
        let enabled = match new_state {
            s if s == EventState::NORMAL => self.event_enable & Self::TO_NORMAL != 0,
            s if s == EventState::OFFNORMAL => self.event_enable & Self::TO_OFFNORMAL != 0,
            _ => self.event_enable & Self::TO_FAULT != 0,
        };
        enabled.then_some(change)
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
    const TO_OFFNORMAL: u8 = 0x01;
    #[allow(dead_code)]
    const TO_FAULT: u8 = 0x02;
    const TO_NORMAL: u8 = 0x04;

    /// Per-write entry point; see [`OutOfRangeDetector::evaluate`].
    pub fn evaluate(
        &mut self,
        present_value: u32,
        feedback_value: u32,
    ) -> Option<EventStateChange> {
        self.probe(present_value, feedback_value)
    }

    /// Per-write probe: seed or cancel a pending transition, fire on zero delay.
    pub fn probe(&mut self, present_value: u32, feedback_value: u32) -> Option<EventStateChange> {
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
    pub fn tick(&mut self, present_value: u32, feedback_value: u32) -> Option<EventStateChange> {
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

    /// Confirm a transition to `new_state`, gated by `event_enable`.
    fn fire(&mut self, new_state: EventState) -> Option<EventStateChange> {
        let change = EventStateChange {
            from: self.event_state,
            to: new_state,
        };
        self.event_state = new_state;
        let enabled = match new_state {
            s if s == EventState::NORMAL => self.event_enable & Self::TO_NORMAL != 0,
            s if s == EventState::OFFNORMAL => self.event_enable & Self::TO_OFFNORMAL != 0,
            _ => false,
        };
        enabled.then_some(change)
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
/// object whose detector is exposed as `self.event_detector` and whose
/// present value is `self.present_value`.
///
/// This wires both the per-write [`evaluate_intrinsic_reporting`](crate::traits::BACnetObject::evaluate_intrinsic_reporting)
/// probe and the periodic [`tick_intrinsic_reporting`](crate::traits::BACnetObject::tick_intrinsic_reporting)
/// tick to the detector's split `probe` / `tick` entry points, honoring
/// `Time_Delay` without the object types repeating the delegation.
#[macro_export]
macro_rules! impl_intrinsic_reporting {
    ($detector_field:ident, $present_value_field:ident) => {
        fn evaluate_intrinsic_reporting(&mut self) -> Option<$crate::event::EventStateChange> {
            self.$detector_field.probe(self.$present_value_field)
        }

        fn tick_intrinsic_reporting(&mut self) -> Option<$crate::event::EventStateChange> {
            self.$detector_field.tick(self.$present_value_field)
        }
    };
}

#[cfg(test)]
mod tests;
