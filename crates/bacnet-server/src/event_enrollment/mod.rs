//! Event Enrollment algorithmic evaluation.
//!
//! Unlike intrinsic reporting (built into object types), Event Enrollment is a
//! separate object that monitors another object's property and evaluates an
//! algorithm against it.
//!
//! Supported algorithms: OUT_OF_RANGE, FLOATING_LIMIT, CHANGE_OF_STATE,
//! CHANGE_OF_BITSTRING, CHANGE_OF_VALUE.

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::EventStateChange;
use bacnet_types::enums::{EventState, EventType, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// A state transition detected during event enrollment evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct EventEnrollmentTransition {
    /// The EventEnrollment object that detected the transition.
    pub enrollment_oid: ObjectIdentifier,
    /// The monitored object whose property triggered the transition.
    pub monitored_oid: ObjectIdentifier,
    /// The detected state change.
    pub change: EventStateChange,
    /// The event type that was evaluated.
    pub event_type: EventType,
}

// ---- Event parameter encoding helpers ----

/// Encode OUT_OF_RANGE parameters: `[high_limit: f32 LE][low_limit: f32 LE][deadband: f32 LE]`
pub fn encode_out_of_range_params(high_limit: f32, low_limit: f32, deadband: f32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(12);
    buf.extend_from_slice(&high_limit.to_le_bytes());
    buf.extend_from_slice(&low_limit.to_le_bytes());
    buf.extend_from_slice(&deadband.to_le_bytes());
    buf
}

/// Encode FLOATING_LIMIT parameters:
/// `[setpoint: f32 LE][high_diff: f32 LE][low_diff: f32 LE][deadband: f32 LE]`
pub fn encode_floating_limit_params(
    setpoint: f32,
    high_diff_limit: f32,
    low_diff_limit: f32,
    deadband: f32,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16);
    buf.extend_from_slice(&setpoint.to_le_bytes());
    buf.extend_from_slice(&high_diff_limit.to_le_bytes());
    buf.extend_from_slice(&low_diff_limit.to_le_bytes());
    buf.extend_from_slice(&deadband.to_le_bytes());
    buf
}

/// Encode CHANGE_OF_STATE parameters: `[count: u32 LE][alarm_values: u32 LE ...]`
pub fn encode_change_of_state_params(alarm_values: &[u32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + alarm_values.len() * 4);
    buf.extend_from_slice(&(alarm_values.len() as u32).to_le_bytes());
    for &v in alarm_values {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

/// Encode CHANGE_OF_VALUE parameters: `[increment: f32 LE]`
pub fn encode_change_of_value_params(increment: f32) -> Vec<u8> {
    increment.to_le_bytes().to_vec()
}

/// Encode CHANGE_OF_BITSTRING parameters:
/// `[mask_len: u32 LE][mask_bytes ...][alarm_bits ...]`
pub fn encode_change_of_bitstring_params(mask: &[u8], alarm_bits: &[u8]) -> Vec<u8> {
    let len = mask.len().min(alarm_bits.len());
    let mut buf = Vec::with_capacity(4 + len * 2);
    buf.extend_from_slice(&(len as u32).to_le_bytes());
    buf.extend_from_slice(&mask[..len]);
    buf.extend_from_slice(&alarm_bits[..len]);
    buf
}

// ---- Algorithm evaluation ----

/// Evaluate the OUT_OF_RANGE algorithm.
///
/// Compares a real present_value against high/low limits with deadband hysteresis.
fn eval_out_of_range(params: &[u8], value: f32, current: EventState) -> EventState {
    if params.len() < 12 {
        return current;
    }
    let high_limit = f32::from_le_bytes([params[0], params[1], params[2], params[3]]);
    let low_limit = f32::from_le_bytes([params[4], params[5], params[6], params[7]]);
    let deadband = f32::from_le_bytes([params[8], params[9], params[10], params[11]]);

    match current {
        s if s == EventState::NORMAL => {
            if value > high_limit {
                EventState::HIGH_LIMIT
            } else if value < low_limit {
                EventState::LOW_LIMIT
            } else {
                EventState::NORMAL
            }
        }
        s if s == EventState::HIGH_LIMIT => {
            if value < low_limit {
                EventState::LOW_LIMIT
            } else if value < high_limit - deadband {
                EventState::NORMAL
            } else {
                EventState::HIGH_LIMIT
            }
        }
        s if s == EventState::LOW_LIMIT => {
            if value > high_limit {
                EventState::HIGH_LIMIT
            } else if value > low_limit + deadband {
                EventState::NORMAL
            } else {
                EventState::LOW_LIMIT
            }
        }
        _ => current,
    }
}

/// Evaluate the FLOATING_LIMIT algorithm.
///
/// Compares a real present_value against a setpoint +/- differential limits
/// with deadband hysteresis.
fn eval_floating_limit(params: &[u8], value: f32, current: EventState) -> EventState {
    if params.len() < 16 {
        return current;
    }
    let setpoint = f32::from_le_bytes([params[0], params[1], params[2], params[3]]);
    let high_diff = f32::from_le_bytes([params[4], params[5], params[6], params[7]]);
    let low_diff = f32::from_le_bytes([params[8], params[9], params[10], params[11]]);
    let deadband = f32::from_le_bytes([params[12], params[13], params[14], params[15]]);

    let high_limit = setpoint + high_diff;
    let low_limit = setpoint - low_diff;

    match current {
        s if s == EventState::NORMAL => {
            if value > high_limit {
                EventState::HIGH_LIMIT
            } else if value < low_limit {
                EventState::LOW_LIMIT
            } else {
                EventState::NORMAL
            }
        }
        s if s == EventState::HIGH_LIMIT => {
            if value < low_limit {
                EventState::LOW_LIMIT
            } else if value < high_limit - deadband {
                EventState::NORMAL
            } else {
                EventState::HIGH_LIMIT
            }
        }
        s if s == EventState::LOW_LIMIT => {
            if value > high_limit {
                EventState::HIGH_LIMIT
            } else if value > low_limit + deadband {
                EventState::NORMAL
            } else {
                EventState::LOW_LIMIT
            }
        }
        _ => current,
    }
}

/// Evaluate the CHANGE_OF_STATE algorithm.
///
/// OFFNORMAL if the value matches any alarm value, otherwise NORMAL.
fn eval_change_of_state(params: &[u8], value: u32, _current: EventState) -> EventState {
    if params.len() < 4 {
        return EventState::NORMAL;
    }
    let count = u32::from_le_bytes([params[0], params[1], params[2], params[3]]) as usize;
    let needed = 4usize.saturating_add(count.saturating_mul(4));
    if params.len() < needed {
        return EventState::NORMAL;
    }
    for i in 0..count {
        let offset = 4 + i * 4;
        let alarm_val = u32::from_le_bytes([
            params[offset],
            params[offset + 1],
            params[offset + 2],
            params[offset + 3],
        ]);
        if value == alarm_val {
            return EventState::OFFNORMAL;
        }
    }
    EventState::NORMAL
}

/// Evaluate the CHANGE_OF_BITSTRING algorithm.
///
/// Applies a mask to the monitored bitstring and compares against the alarm pattern.
fn eval_change_of_bitstring(params: &[u8], value_bits: &[u8], _current: EventState) -> EventState {
    if params.len() < 4 {
        return EventState::NORMAL;
    }
    let mask_len = u32::from_le_bytes([params[0], params[1], params[2], params[3]]) as usize;
    let needed = 4usize.saturating_add(mask_len.saturating_mul(2));
    if params.len() < needed {
        return EventState::NORMAL;
    }

    let mask = &params[4..4 + mask_len];
    let alarm_bits = &params[4 + mask_len..4 + 2 * mask_len];

    for i in 0..mask_len {
        let monitored_byte = value_bits.get(i).copied().unwrap_or(0);
        if (monitored_byte & mask[i]) != (alarm_bits[i] & mask[i]) {
            return EventState::NORMAL;
        }
    }
    EventState::OFFNORMAL
}

/// Evaluate the CHANGE_OF_VALUE algorithm.
///
/// OFFNORMAL if |current_value| >= increment, otherwise NORMAL.
fn eval_change_of_value(params: &[u8], value: f32, _current: EventState) -> EventState {
    if params.len() < 4 {
        return EventState::NORMAL;
    }
    let increment = f32::from_le_bytes([params[0], params[1], params[2], params[3]]);
    if increment <= 0.0 || !increment.is_finite() {
        return EventState::NORMAL;
    }
    if value.abs() >= increment {
        EventState::OFFNORMAL
    } else {
        EventState::NORMAL
    }
}

/// Extract a real (f32) value from a PropertyValue.
fn extract_real(pv: &PropertyValue) -> Option<f32> {
    match pv {
        PropertyValue::Real(v) => Some(*v),
        PropertyValue::Double(v) => Some(*v as f32),
        PropertyValue::Unsigned(v) => Some(*v as f32),
        PropertyValue::Signed(v) => Some(*v as f32),
        _ => None,
    }
}

/// Extract an enumerated (u32) value from a PropertyValue.
fn extract_enumerated(pv: &PropertyValue) -> Option<u32> {
    match pv {
        PropertyValue::Enumerated(v) => Some(*v),
        PropertyValue::Unsigned(v) => Some(*v as u32),
        _ => None,
    }
}

/// Extract bitstring bytes from a PropertyValue.
fn extract_bitstring(pv: &PropertyValue) -> Option<Vec<u8>> {
    match pv {
        PropertyValue::BitString { data, .. } => Some(data.clone()),
        _ => None,
    }
}

/// Read the object_property_reference from an EventEnrollment object.
///
/// Returns (monitored_object_id, monitored_property_id) if valid.
fn read_object_property_ref(
    enrollment: &dyn bacnet_objects::traits::BACnetObject,
) -> Option<(ObjectIdentifier, PropertyIdentifier)> {
    match enrollment.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None) {
        Ok(PropertyValue::List(ref items)) if items.len() >= 2 => {
            let obj_id = match &items[0] {
                PropertyValue::ObjectIdentifier(oid) => *oid,
                _ => return None,
            };
            let prop_id = match &items[1] {
                PropertyValue::Unsigned(v) => PropertyIdentifier::from_raw(*v as u32),
                _ => return None,
            };
            Some((obj_id, prop_id))
        }
        _ => None,
    }
}

/// Evaluate all EventEnrollment objects in the database.
///
/// For each active enrollment, reads the monitored property, evaluates the
/// configured algorithm, and returns any state transitions.
pub fn evaluate_event_enrollments(db: &mut ObjectDatabase) -> Vec<EventEnrollmentTransition> {
    let oids = db.find_by_type(ObjectType::EVENT_ENROLLMENT);

    let mut updates: Vec<(
        ObjectIdentifier,
        ObjectIdentifier,
        u32,
        EventState,
        EventState,
    )> = Vec::new();

    for oid in &oids {
        let Some(enrollment) = db.get(oid) else {
            continue;
        };

        if let Ok(PropertyValue::Boolean(true)) =
            enrollment.read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
        {
            continue;
        }

        let event_type_raw = match enrollment.read_property(PropertyIdentifier::EVENT_TYPE, None) {
            Ok(PropertyValue::Enumerated(v)) => v,
            _ => continue,
        };

        let current_state = match enrollment.read_property(PropertyIdentifier::EVENT_STATE, None) {
            Ok(PropertyValue::Enumerated(v)) => EventState::from_raw(v),
            _ => continue,
        };

        let event_enable = match enrollment.read_property(PropertyIdentifier::EVENT_ENABLE, None) {
            Ok(PropertyValue::BitString { data, .. }) => data.first().map(|b| b >> 5).unwrap_or(0),
            _ => 0,
        };

        let params = match enrollment.read_property(PropertyIdentifier::EVENT_PARAMETERS, None) {
            Ok(PropertyValue::OctetString(bytes)) => bytes,
            _ => Vec::new(),
        };

        let Some((monitored_oid, monitored_prop)) = read_object_property_ref(enrollment) else {
            continue;
        };

        let Some(monitored_obj) = db.get(&monitored_oid) else {
            continue;
        };
        let monitored_value = match monitored_obj.read_property(monitored_prop, None) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let event_type = EventType::from_raw(event_type_raw);
        let new_state = if event_type == EventType::OUT_OF_RANGE {
            let Some(val) = extract_real(&monitored_value) else {
                continue;
            };
            eval_out_of_range(&params, val, current_state)
        } else if event_type == EventType::FLOATING_LIMIT {
            let Some(val) = extract_real(&monitored_value) else {
                continue;
            };
            eval_floating_limit(&params, val, current_state)
        } else if event_type == EventType::CHANGE_OF_STATE {
            let Some(val) = extract_enumerated(&monitored_value) else {
                continue;
            };
            eval_change_of_state(&params, val, current_state)
        } else if event_type == EventType::CHANGE_OF_BITSTRING {
            let Some(bits) = extract_bitstring(&monitored_value) else {
                continue;
            };
            eval_change_of_bitstring(&params, &bits, current_state)
        } else if event_type == EventType::CHANGE_OF_VALUE {
            let Some(val) = extract_real(&monitored_value) else {
                continue;
            };
            eval_change_of_value(&params, val, current_state)
        } else {
            continue;
        };

        if new_state == current_state {
            continue;
        }

        let transition_enabled = match new_state {
            s if s == EventState::NORMAL => event_enable & 0x04 != 0,
            s if s == EventState::HIGH_LIMIT
                || s == EventState::LOW_LIMIT
                || s == EventState::OFFNORMAL =>
            {
                event_enable & 0x01 != 0
            }
            _ => event_enable & 0x02 != 0,
        };

        if transition_enabled {
            updates.push((
                *oid,
                monitored_oid,
                event_type_raw,
                current_state,
                new_state,
            ));
        }
    }

    let mut transitions = Vec::new();
    for (oid, monitored_oid, event_type_raw, from_state, to_state) in updates {
        if let Some(obj) = db.get_mut(&oid) {
            if obj
                .write_property(
                    PropertyIdentifier::EVENT_STATE,
                    None,
                    PropertyValue::Enumerated(to_state.to_raw()),
                    None,
                )
                .is_ok()
            {
                transitions.push(EventEnrollmentTransition {
                    enrollment_oid: oid,
                    monitored_oid,
                    change: EventStateChange {
                        from: from_state,
                        to: to_state,
                    },
                    event_type: EventType::from_raw(event_type_raw),
                });
            }
        }
    }

    transitions
}

#[cfg(test)]
mod tests;
