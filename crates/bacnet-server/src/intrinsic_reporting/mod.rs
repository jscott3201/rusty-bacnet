//! Intrinsic reporting engine.
//!
//! Evaluates five intrinsic reporting algorithms periodically:
//! CHANGE_OF_STATE, CHANGE_OF_BITSTRING, FLOATING_LIMIT, COMMAND_FAILURE,
//! and CHANGE_OF_VALUE.
//!
//! The engine maintains per-object tracking state (last event state, time-delay
//! countdown, last notified value) and is designed to be called on the same
//! 10-second periodic interval as fault detection.

use std::collections::HashMap;

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::EventStateChange;
use bacnet_types::enums::{EventState, EventType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

// ───────────────────────────────── helpers ──────────────────────────────────

/// Returns a status-flags byte with the IN_ALARM bit set or clear.
fn status_flags_byte(in_alarm: bool) -> u8 {
    if in_alarm {
        0x08
    } else {
        0x00
    }
}

/// Read a `Real` property, returning `None` on missing / wrong type.
fn read_real(
    obj: &dyn bacnet_objects::traits::BACnetObject,
    prop: PropertyIdentifier,
) -> Option<f32> {
    match obj.read_property(prop, None) {
        Ok(PropertyValue::Real(v)) => Some(v),
        _ => None,
    }
}

/// Read an `Unsigned` property as u32.
fn read_unsigned(
    obj: &dyn bacnet_objects::traits::BACnetObject,
    prop: PropertyIdentifier,
) -> Option<u32> {
    match obj.read_property(prop, None) {
        Ok(PropertyValue::Unsigned(v)) => Some(v as u32),
        _ => None,
    }
}

/// Read EVENT_ENABLE as a 3-bit value.
///
/// Objects store EVENT_ENABLE as `BitString { unused_bits: 5, data: [value << 5] }`.
/// This helper handles both BitString and Unsigned representations for robustness.
fn read_event_enable(obj: &dyn bacnet_objects::traits::BACnetObject) -> u8 {
    match obj.read_property(PropertyIdentifier::EVENT_ENABLE, None) {
        Ok(PropertyValue::BitString { unused_bits, data }) => {
            if data.is_empty() {
                return 0;
            }
            data[0] >> unused_bits
        }
        Ok(PropertyValue::Unsigned(v)) => v as u8,
        _ => 0,
    }
}

/// Read an `Enumerated` property.
fn read_enum(
    obj: &dyn bacnet_objects::traits::BACnetObject,
    prop: PropertyIdentifier,
) -> Option<u32> {
    match obj.read_property(prop, None) {
        Ok(PropertyValue::Enumerated(v)) => Some(v),
        _ => None,
    }
}

/// Read a `BitString` property.
fn read_bitstring(
    obj: &dyn bacnet_objects::traits::BACnetObject,
    prop: PropertyIdentifier,
) -> Option<(u8, Vec<u8>)> {
    match obj.read_property(prop, None) {
        Ok(PropertyValue::BitString { unused_bits, data }) => Some((unused_bits, data)),
        _ => None,
    }
}

/// Read a `List` of `Enumerated` values (e.g. alarm_values for binary objects).
fn read_enum_list(
    obj: &dyn bacnet_objects::traits::BACnetObject,
    prop: PropertyIdentifier,
) -> Vec<u32> {
    match obj.read_property(prop, None) {
        Ok(PropertyValue::List(items)) => items
            .into_iter()
            .filter_map(|pv| match pv {
                PropertyValue::Enumerated(v) => Some(v),
                _ => None,
            })
            .collect(),
        Ok(PropertyValue::Enumerated(v)) => vec![v],
        _ => Vec::new(),
    }
}

/// Read a `List` of `BitString` values (alarm_values for bitstring objects).
fn read_bitstring_list(
    obj: &dyn bacnet_objects::traits::BACnetObject,
    prop: PropertyIdentifier,
) -> Vec<(u8, Vec<u8>)> {
    match obj.read_property(prop, None) {
        Ok(PropertyValue::List(items)) => items
            .into_iter()
            .filter_map(|pv| match pv {
                PropertyValue::BitString { unused_bits, data } => Some((unused_bits, data)),
                _ => None,
            })
            .collect(),
        Ok(PropertyValue::BitString { unused_bits, data }) => vec![(unused_bits, data)],
        _ => Vec::new(),
    }
}

// ──────────────────────────── per-object state ─────────────────────────────

/// Tracking state kept per-object between evaluation cycles.
#[derive(Debug, Clone)]
struct ObjectTracker {
    /// Current event state as tracked by the engine.
    event_state: EventState,
    /// Remaining time-delay ticks before a pending transition is confirmed.
    /// Each tick represents one evaluation cycle (10 s).
    time_delay_remaining: Option<u32>,
    /// The event state that would be confirmed after the time delay.
    pending_state: Option<EventState>,
    /// Last present_value when a COV notification was sent (for CHANGE_OF_VALUE).
    last_cov_real: Option<f32>,
    /// Last binary present_value when a COV notification was sent.
    last_cov_binary: Option<u32>,
}

impl ObjectTracker {
    fn new() -> Self {
        Self {
            event_state: EventState::NORMAL,
            time_delay_remaining: None,
            pending_state: None,
            last_cov_real: None,
            last_cov_binary: None,
        }
    }
}

// ──────────────────────────── result types ──────────────────────────────────

/// Result of an intrinsic reporting evaluation for one object.
#[derive(Debug, Clone)]
pub struct IntrinsicEvent {
    /// Object that generated the event.
    pub object_id: ObjectIdentifier,
    /// The state transition.
    pub change: EventStateChange,
    /// BACnet event type for the notification.
    pub event_type: EventType,
    /// Status-flags byte for the notification parameters.
    pub status_flags: u8,
}

// ───────────────────────────── algorithm IDs ───────────────────────────────

/// Intrinsic algorithm type, derived from an object's configured event_type
/// or object type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlgorithmType {
    ChangeOfState,
    ChangeOfBitstring,
    FloatingLimit,
    CommandFailure,
    ChangeOfValue,
}

// ──────────────────────────────── engine ────────────────────────────────────

/// Intrinsic reporting engine.
///
/// Holds per-object tracking state and evaluates all five algorithms each
/// time [`IntrinsicReportingEngine::evaluate`] is called.
pub struct IntrinsicReportingEngine {
    trackers: HashMap<ObjectIdentifier, ObjectTracker>,
}

impl Default for IntrinsicReportingEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl IntrinsicReportingEngine {
    pub fn new() -> Self {
        Self {
            trackers: HashMap::new(),
        }
    }

    /// Evaluate all objects in the database and return any detected events.
    pub fn evaluate(&mut self, db: &ObjectDatabase) -> Vec<IntrinsicEvent> {
        let mut events = Vec::new();

        let oids: Vec<ObjectIdentifier> = db.iter_objects().map(|(oid, _)| oid).collect();

        for oid in oids {
            if let Some(obj) = db.get(&oid) {
                if let Some(event) = self.evaluate_object(oid, obj) {
                    events.push(event);
                }
            }
        }

        events
    }

    /// Evaluate a single object.
    fn evaluate_object(
        &mut self,
        oid: ObjectIdentifier,
        obj: &dyn bacnet_objects::traits::BACnetObject,
    ) -> Option<IntrinsicEvent> {
        // Determine which algorithm applies based on EVENT_TYPE property.
        let algo = match read_enum(obj, PropertyIdentifier::EVENT_TYPE) {
            Some(v) if v == EventType::CHANGE_OF_STATE.to_raw() => AlgorithmType::ChangeOfState,
            Some(v) if v == EventType::CHANGE_OF_BITSTRING.to_raw() => {
                AlgorithmType::ChangeOfBitstring
            }
            Some(v) if v == EventType::FLOATING_LIMIT.to_raw() => AlgorithmType::FloatingLimit,
            Some(v) if v == EventType::COMMAND_FAILURE.to_raw() => AlgorithmType::CommandFailure,
            Some(v) if v == EventType::CHANGE_OF_VALUE.to_raw() => AlgorithmType::ChangeOfValue,
            _ => return None,
        };

        // Check EVENT_ENABLE — if it's 0 the object has intrinsic reporting disabled.
        let event_enable = read_event_enable(obj) as u32;
        if event_enable == 0 {
            return None;
        }

        let time_delay = read_unsigned(obj, PropertyIdentifier::TIME_DELAY).unwrap_or(0);

        let tracker = self.trackers.entry(oid).or_insert_with(|| {
            let mut t = ObjectTracker::new();
            t.event_state =
                EventState::from_raw(read_enum(obj, PropertyIdentifier::EVENT_STATE).unwrap_or(0));
            t
        });

        match algo {
            AlgorithmType::ChangeOfState => {
                evaluate_change_of_state(oid, obj, tracker, time_delay, event_enable)
            }
            AlgorithmType::ChangeOfBitstring => {
                evaluate_change_of_bitstring(oid, obj, tracker, time_delay, event_enable)
            }
            AlgorithmType::FloatingLimit => {
                evaluate_floating_limit(oid, obj, tracker, time_delay, event_enable)
            }
            AlgorithmType::CommandFailure => {
                evaluate_command_failure(oid, obj, tracker, time_delay, event_enable)
            }
            AlgorithmType::ChangeOfValue => evaluate_change_of_value(oid, obj, tracker),
        }
    }
}

// ──────────────────── time-delay helper ────────────────────────────────────

/// Apply time-delay logic to a pending state transition.
///
/// Returns `Some(IntrinsicEvent)` when the time delay has elapsed and the
/// transition should be confirmed. If the desired state reverts before the
/// delay elapses, the pending transition is cancelled.
fn apply_time_delay(
    tracker: &mut ObjectTracker,
    desired_state: EventState,
    time_delay: u32,
    oid: ObjectIdentifier,
    event_type: EventType,
    event_enable: u32,
) -> Option<IntrinsicEvent> {
    if desired_state == tracker.event_state {
        tracker.pending_state = None;
        tracker.time_delay_remaining = None;
        return None;
    }

    // Check if this transition is enabled.
    let enabled = if desired_state == EventState::NORMAL {
        event_enable & 0x04 != 0 // TO_NORMAL
    } else {
        event_enable & 0x01 != 0 // TO_OFFNORMAL
    };

    if !enabled {
        tracker.event_state = desired_state;
        tracker.pending_state = None;
        tracker.time_delay_remaining = None;
        return None;
    }

    if time_delay == 0 {
        let change = EventStateChange {
            from: tracker.event_state,
            to: desired_state,
        };
        let in_alarm = desired_state != EventState::NORMAL;
        tracker.event_state = desired_state;
        tracker.pending_state = None;
        tracker.time_delay_remaining = None;
        return Some(IntrinsicEvent {
            object_id: oid,
            change,
            event_type,
            status_flags: status_flags_byte(in_alarm),
        });
    }

    match tracker.pending_state {
        Some(ps) if ps == desired_state => {
            let remaining = tracker.time_delay_remaining.unwrap_or(time_delay);
            if remaining <= 1 {
                let change = EventStateChange {
                    from: tracker.event_state,
                    to: desired_state,
                };
                let in_alarm = desired_state != EventState::NORMAL;
                tracker.event_state = desired_state;
                tracker.pending_state = None;
                tracker.time_delay_remaining = None;
                Some(IntrinsicEvent {
                    object_id: oid,
                    change,
                    event_type,
                    status_flags: status_flags_byte(in_alarm),
                })
            } else {
                tracker.time_delay_remaining = Some(remaining - 1);
                None
            }
        }
        _ => {
            tracker.pending_state = Some(desired_state);
            tracker.time_delay_remaining = Some(time_delay);
            None
        }
    }
}

// ───────────────────── T38: CHANGE_OF_STATE ────────────────────────────────

/// CHANGE_OF_STATE for binary objects.
///
/// OFFNORMAL if present_value is in alarm_values, otherwise NORMAL.
fn evaluate_change_of_state(
    oid: ObjectIdentifier,
    obj: &dyn bacnet_objects::traits::BACnetObject,
    tracker: &mut ObjectTracker,
    time_delay: u32,
    event_enable: u32,
) -> Option<IntrinsicEvent> {
    let pv = read_enum(obj, PropertyIdentifier::PRESENT_VALUE)?;
    let alarm_values = read_enum_list(obj, PropertyIdentifier::ALARM_VALUES);

    let desired = if alarm_values.contains(&pv) {
        EventState::OFFNORMAL
    } else {
        EventState::NORMAL
    };

    apply_time_delay(
        tracker,
        desired,
        time_delay,
        oid,
        EventType::CHANGE_OF_STATE,
        event_enable,
    )
}

// ───────────────────── T39: CHANGE_OF_BITSTRING ────────────────────────────

/// Bitwise AND of two byte slices (shortest length wins).
fn bitstring_and(a: &[u8], b: &[u8]) -> Vec<u8> {
    a.iter().zip(b.iter()).map(|(x, y)| x & y).collect()
}

/// CHANGE_OF_BITSTRING.
///
/// OFFNORMAL if (present_value AND bit_mask) matches any alarm_values entry.
fn evaluate_change_of_bitstring(
    oid: ObjectIdentifier,
    obj: &dyn bacnet_objects::traits::BACnetObject,
    tracker: &mut ObjectTracker,
    time_delay: u32,
    event_enable: u32,
) -> Option<IntrinsicEvent> {
    let (_pv_unused, pv_data) = read_bitstring(obj, PropertyIdentifier::PRESENT_VALUE)?;
    let (_bm_unused, bm_data) = read_bitstring(obj, PropertyIdentifier::BIT_MASK)?;
    let alarm_values = read_bitstring_list(obj, PropertyIdentifier::ALARM_VALUES);

    let masked = bitstring_and(&pv_data, &bm_data);

    let in_alarm = alarm_values.iter().any(|(_unused, av_data)| {
        let masked_av = bitstring_and(av_data, &bm_data);
        masked_av == masked
    });

    let desired = if in_alarm {
        EventState::OFFNORMAL
    } else {
        EventState::NORMAL
    };

    apply_time_delay(
        tracker,
        desired,
        time_delay,
        oid,
        EventType::CHANGE_OF_BITSTRING,
        event_enable,
    )
}

// ───────────────────── T40: FLOATING_LIMIT ─────────────────────────────────

/// FLOATING_LIMIT.
///
/// Compares present_value against setpoint +/- error_limit with deadband
/// hysteresis. Respects LIMIT_ENABLE flags.
fn evaluate_floating_limit(
    oid: ObjectIdentifier,
    obj: &dyn bacnet_objects::traits::BACnetObject,
    tracker: &mut ObjectTracker,
    time_delay: u32,
    event_enable: u32,
) -> Option<IntrinsicEvent> {
    let pv = read_real(obj, PropertyIdentifier::PRESENT_VALUE)?;
    let setpoint = read_real(obj, PropertyIdentifier::SETPOINT_REFERENCE)?;
    let error_limit = read_real(obj, PropertyIdentifier::ERROR_LIMIT)?;
    let deadband = read_real(obj, PropertyIdentifier::DEADBAND).unwrap_or(0.0);

    let limit_enable = read_bitstring(obj, PropertyIdentifier::LIMIT_ENABLE)
        .and_then(|(_, data)| data.first().copied())
        .unwrap_or(0);
    let low_enabled = limit_enable & 0x80 != 0;
    let high_enabled = limit_enable & 0x40 != 0;

    let high_limit = setpoint + error_limit;
    let low_limit = setpoint - error_limit;

    let desired = match tracker.event_state {
        s if s == EventState::NORMAL => {
            if high_enabled && pv > high_limit {
                EventState::HIGH_LIMIT
            } else if low_enabled && pv < low_limit {
                EventState::LOW_LIMIT
            } else {
                EventState::NORMAL
            }
        }
        s if s == EventState::HIGH_LIMIT => {
            if low_enabled && pv < low_limit {
                EventState::LOW_LIMIT
            } else if pv < high_limit - deadband {
                EventState::NORMAL
            } else {
                EventState::HIGH_LIMIT
            }
        }
        s if s == EventState::LOW_LIMIT => {
            if high_enabled && pv > high_limit {
                EventState::HIGH_LIMIT
            } else if pv > low_limit + deadband {
                EventState::NORMAL
            } else {
                EventState::LOW_LIMIT
            }
        }
        _ => tracker.event_state,
    };

    apply_time_delay(
        tracker,
        desired,
        time_delay,
        oid,
        EventType::FLOATING_LIMIT,
        event_enable,
    )
}

// ───────────────────── T41: COMMAND_FAILURE ─────────────────────────────────

/// COMMAND_FAILURE.
///
/// OFFNORMAL if feedback_value differs from present_value, otherwise NORMAL.
fn evaluate_command_failure(
    oid: ObjectIdentifier,
    obj: &dyn bacnet_objects::traits::BACnetObject,
    tracker: &mut ObjectTracker,
    time_delay: u32,
    event_enable: u32,
) -> Option<IntrinsicEvent> {
    let pv = read_enum(obj, PropertyIdentifier::PRESENT_VALUE)?;
    let feedback = read_enum(obj, PropertyIdentifier::FEEDBACK_VALUE)?;

    let desired = if pv != feedback {
        EventState::OFFNORMAL
    } else {
        EventState::NORMAL
    };

    apply_time_delay(
        tracker,
        desired,
        time_delay,
        oid,
        EventType::COMMAND_FAILURE,
        event_enable,
    )
}

// ───────────────────── T42: CHANGE_OF_VALUE ────────────────────────────────

/// CHANGE_OF_VALUE (intrinsic COV).
///
/// Fires when present_value changes by more than cov_increment (analog) or
/// changes state (binary). Does not use time_delay.
fn evaluate_change_of_value(
    oid: ObjectIdentifier,
    obj: &dyn bacnet_objects::traits::BACnetObject,
    tracker: &mut ObjectTracker,
) -> Option<IntrinsicEvent> {
    if let Some(pv) = read_real(obj, PropertyIdentifier::PRESENT_VALUE) {
        let increment = read_real(obj, PropertyIdentifier::COV_INCREMENT).unwrap_or(0.0);

        let should_notify = match tracker.last_cov_real {
            Some(last) => {
                let delta = (pv - last).abs();
                increment <= 0.0 || delta >= increment
            }
            None => true,
        };

        if should_notify {
            tracker.last_cov_real = Some(pv);
            return Some(IntrinsicEvent {
                object_id: oid,
                change: EventStateChange {
                    from: EventState::NORMAL,
                    to: EventState::NORMAL,
                },
                event_type: EventType::CHANGE_OF_VALUE,
                status_flags: status_flags_byte(false),
            });
        }

        return None;
    }

    if let Some(pv) = read_enum(obj, PropertyIdentifier::PRESENT_VALUE) {
        let should_notify = match tracker.last_cov_binary {
            Some(last) => pv != last,
            None => true,
        };

        if should_notify {
            tracker.last_cov_binary = Some(pv);
            return Some(IntrinsicEvent {
                object_id: oid,
                change: EventStateChange {
                    from: EventState::NORMAL,
                    to: EventState::NORMAL,
                },
                event_type: EventType::CHANGE_OF_VALUE,
                status_flags: status_flags_byte(false),
            });
        }
    }

    None
}

// ────────────────────────────── tests ──────────────────────────────────────

#[cfg(test)]
mod tests;
