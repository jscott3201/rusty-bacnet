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
    /// Per Clause 13.2.5:
    /// - `to == NORMAL` → `ToNormal`
    /// - `to == FAULT` → `ToFault`
    /// - Everything else (OFFNORMAL, HIGH_LIMIT, LOW_LIMIT) → `ToOffnormal`
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
}

/// Which limits are enabled (Clause 12.1.14).
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

/// OUT_OF_RANGE event detector for analog objects.
///
/// Implements the event state machine per Clause 13.3.2:
/// - NORMAL → HIGH_LIMIT when `present_value > high_limit` (if high_limit enabled)
/// - NORMAL → LOW_LIMIT when `present_value < low_limit` (if low_limit enabled)
/// - HIGH_LIMIT → NORMAL when `present_value < high_limit - deadband`
/// - LOW_LIMIT → NORMAL when `present_value > low_limit + deadband`
/// - HIGH_LIMIT → LOW_LIMIT when `present_value < low_limit`
/// - LOW_LIMIT → HIGH_LIMIT when `present_value > high_limit`
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
        }
    }
}

impl OutOfRangeDetector {
    /// Event_Enable bit masks per Clause 13.1.4.
    const TO_OFFNORMAL: u8 = 0x01;
    const TO_FAULT: u8 = 0x02;
    const TO_NORMAL: u8 = 0x04;

    /// Evaluate the present value against configured limits.
    ///
    /// Returns `Some(EventStateChange)` if the event state changed **and**
    /// the corresponding `event_enable` bit is set (Clause 13.1.4).
    /// Internal state always updates regardless of event_enable.
    ///
    /// Note: This implementation uses instant transitions (ignores time_delay).
    pub fn evaluate(&mut self, present_value: f32) -> Option<EventStateChange> {
        let new_state = self.compute_new_state(present_value);
        if new_state != self.event_state {
            let change = EventStateChange {
                from: self.event_state,
                to: new_state,
            };
            self.event_state = new_state;

            // Check event_enable bitmask per Clause 13.1.4
            let enabled = match new_state {
                s if s == EventState::NORMAL => self.event_enable & Self::TO_NORMAL != 0,
                s if s == EventState::HIGH_LIMIT || s == EventState::LOW_LIMIT => {
                    self.event_enable & Self::TO_OFFNORMAL != 0
                }
                _ => self.event_enable & Self::TO_FAULT != 0,
            };

            if enabled {
                Some(change)
            } else {
                None
            }
        } else {
            None
        }
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
// CHANGE_OF_STATE event detector (Clause 13.3.1)
// ---------------------------------------------------------------------------

/// CHANGE_OF_STATE event detector for binary and multi-state objects.
///
/// Per Clause 13.3.1: transitions to OFFNORMAL when the monitored value
/// matches any value in the `alarm_values` list. Returns to NORMAL when
/// the value no longer matches any alarm value.
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
        }
    }
}

impl ChangeOfStateDetector {
    const TO_OFFNORMAL: u8 = 0x01;
    const TO_FAULT: u8 = 0x02;
    const TO_NORMAL: u8 = 0x04;

    /// Evaluate the present value against alarm_values.
    ///
    /// Returns `Some(EventStateChange)` if the event state changed and the
    /// corresponding `event_enable` bit is set.
    pub fn evaluate(&mut self, present_value: u32) -> Option<EventStateChange> {
        let is_alarm = self.alarm_values.contains(&present_value);
        let new_state = if is_alarm {
            EventState::OFFNORMAL
        } else {
            EventState::NORMAL
        };

        if new_state != self.event_state {
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

            if enabled {
                Some(change)
            } else {
                None
            }
        } else {
            None
        }
    }
}

/// COMMAND_FAILURE event detector for commandable output objects (BO, MSO).
///
/// Per Clause 13.3.3: transitions to OFFNORMAL when present_value differs
/// from feedback_value. Returns to NORMAL when they match.
#[derive(Debug, Clone)]
pub struct CommandFailureDetector {
    pub notification_class: u32,
    pub notify_type: u32,
    pub event_enable: u8,
    pub time_delay: u32,
    pub event_state: EventState,
    pub acked_transitions: u8,
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
        }
    }
}

impl CommandFailureDetector {
    const TO_OFFNORMAL: u8 = 0x01;
    #[allow(dead_code)]
    const TO_FAULT: u8 = 0x02;
    const TO_NORMAL: u8 = 0x04;

    /// Evaluate present_value vs feedback_value.
    ///
    /// Returns `Some(EventStateChange)` if the event state changed.
    pub fn evaluate(
        &mut self,
        present_value: u32,
        feedback_value: u32,
    ) -> Option<EventStateChange> {
        let new_state = if present_value != feedback_value {
            EventState::OFFNORMAL
        } else {
            EventState::NORMAL
        };

        if new_state != self.event_state {
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

            if enabled {
                Some(change)
            } else {
                None
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_detector() -> OutOfRangeDetector {
        OutOfRangeDetector {
            high_limit: 80.0,
            low_limit: 20.0,
            deadband: 2.0,
            limit_enable: LimitEnable::BOTH,
            notification_class: 1,
            notify_type: 0,
            event_enable: 0x07, // all transitions
            time_delay: 0,
            event_state: EventState::NORMAL,
            acked_transitions: 0b111,
        }
    }

    #[test]
    fn normal_stays_normal_within_limits() {
        let mut det = make_detector();
        assert!(det.evaluate(50.0).is_none());
        assert_eq!(det.event_state, EventState::NORMAL);
    }

    #[test]
    fn normal_to_high_limit() {
        let mut det = make_detector();
        let change = det.evaluate(81.0).unwrap();
        assert_eq!(change.from, EventState::NORMAL);
        assert_eq!(change.to, EventState::HIGH_LIMIT);
        assert_eq!(det.event_state, EventState::HIGH_LIMIT);
    }

    #[test]
    fn normal_to_low_limit() {
        let mut det = make_detector();
        let change = det.evaluate(19.0).unwrap();
        assert_eq!(change.from, EventState::NORMAL);
        assert_eq!(change.to, EventState::LOW_LIMIT);
        assert_eq!(det.event_state, EventState::LOW_LIMIT);
    }

    #[test]
    fn at_boundary_no_transition() {
        let mut det = make_detector();
        // At exactly high_limit — not exceeded, stays NORMAL
        assert!(det.evaluate(80.0).is_none());
        // At exactly low_limit — not below, stays NORMAL
        assert!(det.evaluate(20.0).is_none());
    }

    #[test]
    fn high_limit_to_normal_with_deadband() {
        let mut det = make_detector();
        det.evaluate(81.0); // → HIGH_LIMIT

        // Still above (high_limit - deadband) = 78.0 — stay HIGH_LIMIT
        assert!(det.evaluate(79.0).is_none());

        // Drop below deadband threshold
        let change = det.evaluate(77.0).unwrap();
        assert_eq!(change.from, EventState::HIGH_LIMIT);
        assert_eq!(change.to, EventState::NORMAL);
    }

    #[test]
    fn low_limit_to_normal_with_deadband() {
        let mut det = make_detector();
        det.evaluate(19.0); // → LOW_LIMIT

        // Still below (low_limit + deadband) = 22.0 — stay LOW_LIMIT
        assert!(det.evaluate(21.0).is_none());

        // Rise above deadband threshold
        let change = det.evaluate(23.0).unwrap();
        assert_eq!(change.from, EventState::LOW_LIMIT);
        assert_eq!(change.to, EventState::NORMAL);
    }

    #[test]
    fn high_limit_to_low_limit_direct() {
        let mut det = make_detector();
        det.evaluate(81.0); // → HIGH_LIMIT

        // Drop directly below low_limit
        let change = det.evaluate(19.0).unwrap();
        assert_eq!(change.from, EventState::HIGH_LIMIT);
        assert_eq!(change.to, EventState::LOW_LIMIT);
    }

    #[test]
    fn low_limit_to_high_limit_direct() {
        let mut det = make_detector();
        det.evaluate(19.0); // → LOW_LIMIT

        // Jump directly above high_limit
        let change = det.evaluate(81.0).unwrap();
        assert_eq!(change.from, EventState::LOW_LIMIT);
        assert_eq!(change.to, EventState::HIGH_LIMIT);
    }

    #[test]
    fn high_limit_disabled_no_transition() {
        let mut det = make_detector();
        det.limit_enable.high_limit_enable = false;

        // Above high_limit but disabled — stays NORMAL
        assert!(det.evaluate(100.0).is_none());
    }

    #[test]
    fn low_limit_disabled_no_transition() {
        let mut det = make_detector();
        det.limit_enable.low_limit_enable = false;

        // Below low_limit but disabled — stays NORMAL
        assert!(det.evaluate(0.0).is_none());
    }

    #[test]
    fn both_limits_disabled() {
        let mut det = make_detector();
        det.limit_enable = LimitEnable::NONE;
        assert!(det.evaluate(100.0).is_none());
        assert!(det.evaluate(0.0).is_none());
    }

    #[test]
    fn limit_enable_bits_round_trip() {
        let le = LimitEnable::BOTH;
        let bits = le.to_bits();
        let decoded = LimitEnable::from_bits(bits);
        assert_eq!(decoded, le);

        let le = LimitEnable {
            low_limit_enable: true,
            high_limit_enable: false,
        };
        let bits = le.to_bits();
        let decoded = LimitEnable::from_bits(bits);
        assert_eq!(decoded, le);
    }

    #[test]
    fn deadband_at_exact_boundary() {
        let mut det = make_detector();
        det.evaluate(81.0); // → HIGH_LIMIT

        // At exactly (high_limit - deadband) = 78.0 — still HIGH_LIMIT (need to be below)
        assert!(det.evaluate(78.0).is_none());

        // Just below
        let change = det.evaluate(77.99).unwrap();
        assert_eq!(change.to, EventState::NORMAL);
    }

    #[test]
    fn event_state_change_derives_event_type() {
        use bacnet_types::enums::EventType;

        let change = EventStateChange {
            from: EventState::NORMAL,
            to: EventState::HIGH_LIMIT,
        };
        assert_eq!(change.event_type(), EventType::OUT_OF_RANGE);
    }

    #[test]
    fn event_state_change_to_normal_from_high() {
        use bacnet_types::enums::EventType;

        let change = EventStateChange {
            from: EventState::HIGH_LIMIT,
            to: EventState::NORMAL,
        };
        assert_eq!(change.event_type(), EventType::OUT_OF_RANGE);
    }

    #[test]
    fn event_enable_zero_suppresses_all_notifications() {
        let mut det = make_detector();
        det.event_enable = 0x00; // all disabled

        // Should transition internally but return None
        assert!(det.evaluate(81.0).is_none());
        assert_eq!(det.event_state, EventState::HIGH_LIMIT); // state still updated

        assert!(det.evaluate(50.0).is_none());
        assert_eq!(det.event_state, EventState::NORMAL); // state still updated

        assert!(det.evaluate(19.0).is_none());
        assert_eq!(det.event_state, EventState::LOW_LIMIT); // state still updated
    }

    #[test]
    fn event_enable_to_normal_only() {
        let mut det = make_detector();
        det.event_enable = 0x04; // only TO_NORMAL

        // NORMAL → HIGH_LIMIT: TO_OFFNORMAL not enabled, suppressed
        assert!(det.evaluate(81.0).is_none());
        assert_eq!(det.event_state, EventState::HIGH_LIMIT);

        // HIGH_LIMIT → NORMAL: TO_NORMAL enabled, fires
        let change = det.evaluate(50.0).unwrap();
        assert_eq!(change.from, EventState::HIGH_LIMIT);
        assert_eq!(change.to, EventState::NORMAL);

        // NORMAL → LOW_LIMIT: TO_OFFNORMAL not enabled, suppressed
        assert!(det.evaluate(19.0).is_none());
        assert_eq!(det.event_state, EventState::LOW_LIMIT);

        // LOW_LIMIT → NORMAL: TO_NORMAL enabled, fires
        let change = det.evaluate(50.0).unwrap();
        assert_eq!(change.from, EventState::LOW_LIMIT);
        assert_eq!(change.to, EventState::NORMAL);
    }

    #[test]
    fn event_enable_to_offnormal_only() {
        let mut det = make_detector();
        det.event_enable = 0x01; // only TO_OFFNORMAL

        // NORMAL → HIGH_LIMIT: TO_OFFNORMAL enabled, fires
        let change = det.evaluate(81.0).unwrap();
        assert_eq!(change.to, EventState::HIGH_LIMIT);

        // HIGH_LIMIT → NORMAL: TO_NORMAL not enabled, suppressed
        assert!(det.evaluate(50.0).is_none());
        assert_eq!(det.event_state, EventState::NORMAL);
    }

    #[test]
    fn event_state_change_generic() {
        use bacnet_types::enums::EventType;

        let change = EventStateChange {
            from: EventState::NORMAL,
            to: EventState::NORMAL,
        };
        assert_eq!(change.event_type(), EventType::CHANGE_OF_STATE);
    }

    // --- ChangeOfStateDetector tests ---

    #[test]
    fn cos_normal_when_no_alarm_values() {
        let mut det = ChangeOfStateDetector {
            event_enable: 0x07,
            ..Default::default()
        };
        assert!(det.evaluate(0).is_none()); // empty alarm_values → always NORMAL
    }

    #[test]
    fn cos_normal_to_offnormal() {
        let mut det = ChangeOfStateDetector {
            alarm_values: vec![1], // ACTIVE (1) is alarm
            event_enable: 0x07,
            ..Default::default()
        };
        let change = det.evaluate(1).unwrap();
        assert_eq!(change.from, EventState::NORMAL);
        assert_eq!(change.to, EventState::OFFNORMAL);
    }

    #[test]
    fn cos_offnormal_to_normal() {
        let mut det = ChangeOfStateDetector {
            alarm_values: vec![1],
            event_enable: 0x07,
            ..Default::default()
        };
        det.evaluate(1); // → OFFNORMAL
        let change = det.evaluate(0).unwrap(); // back to NORMAL
        assert_eq!(change.from, EventState::OFFNORMAL);
        assert_eq!(change.to, EventState::NORMAL);
    }

    #[test]
    fn cos_stays_offnormal_while_in_alarm() {
        let mut det = ChangeOfStateDetector {
            alarm_values: vec![1],
            event_enable: 0x07,
            ..Default::default()
        };
        det.evaluate(1); // → OFFNORMAL
        assert!(det.evaluate(1).is_none()); // still alarm value, no change
    }

    #[test]
    fn cos_multistate_alarm_values() {
        let mut det = ChangeOfStateDetector {
            alarm_values: vec![3, 5, 7], // multiple alarm states
            event_enable: 0x07,
            ..Default::default()
        };
        assert!(det.evaluate(1).is_none()); // not an alarm state
        let change = det.evaluate(5).unwrap();
        assert_eq!(change.to, EventState::OFFNORMAL);
        assert!(det.evaluate(3).is_none()); // still offnormal (different alarm value)
        let change = det.evaluate(2).unwrap();
        assert_eq!(change.to, EventState::NORMAL);
    }

    // --- CommandFailureDetector tests ---

    #[test]
    fn cmdfail_matching_stays_normal() {
        let mut det = CommandFailureDetector {
            event_enable: 0x07,
            ..Default::default()
        };
        assert!(det.evaluate(1, 1).is_none()); // present == feedback
    }

    #[test]
    fn cmdfail_mismatch_goes_offnormal() {
        let mut det = CommandFailureDetector {
            event_enable: 0x07,
            ..Default::default()
        };
        let change = det.evaluate(1, 0).unwrap(); // present != feedback
        assert_eq!(change.to, EventState::OFFNORMAL);
    }

    #[test]
    fn cmdfail_match_restores_normal() {
        let mut det = CommandFailureDetector {
            event_enable: 0x07,
            ..Default::default()
        };
        det.evaluate(1, 0); // → OFFNORMAL
        let change = det.evaluate(1, 1).unwrap(); // match → NORMAL
        assert_eq!(change.to, EventState::NORMAL);
    }
}
