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
use bacnet_types::constructed::BACnetEventParameter;
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
    /// Whether `Event_Enable` permits distributing a notification for this
    /// transition. The transition is reported and `Event_State` persisted
    /// either way; a cleared bit suppresses only the outbound notification
    /// (ASHRAE 135-2020 Clause 12.12).
    pub distribute: bool,
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

// ---- Structured evaluation (consumes BACnetEventParameter fields) ----

/// Structured OUT_OF_RANGE evaluation with explicit limits and deadband.
fn eval_out_of_range_struct(
    low_limit: f32,
    high_limit: f32,
    deadband: f32,
    value: f32,
    current: EventState,
) -> EventState {
    eval_out_of_range(
        &encode_out_of_range_params(high_limit, low_limit, deadband),
        value,
        current,
    )
}

/// Structured FLOATING_LIMIT evaluation with an explicit setpoint.
fn eval_floating_limit_struct(
    setpoint: f32,
    high_diff_limit: f32,
    low_diff_limit: f32,
    deadband: f32,
    value: f32,
    current: EventState,
) -> EventState {
    eval_floating_limit(
        &encode_floating_limit_params(setpoint, high_diff_limit, low_diff_limit, deadband),
        value,
        current,
    )
}

/// Structured CHANGE_OF_STATE evaluation against a list of alarm values.
///
/// OFFNORMAL if the monitored enumerated value matches any listed
/// [`BACnetPropertyStates`] payload, otherwise NORMAL.
fn eval_change_of_state_struct(
    alarm_values: &[bacnet_types::constructed::BACnetPropertyStates],
    value: u32,
    _current: EventState,
) -> EventState {
    use bacnet_types::constructed::BACnetPropertyStates as S;
    for state in alarm_values {
        let matched = match state {
            S::BooleanValue(v) => value == u32::from(*v),
            S::BinaryValue(v) => value == *v,
            S::EventType(v) => value == *v,
            S::Polarity(v) => value == *v,
            S::ProgramChange(v) => value == *v,
            S::ProgramState(v) => value == *v,
            S::ReasonForHalt(v) => value == *v,
            S::Reliability(v) => value == *v,
            S::State(v) => value == *v,
            S::SystemStatus(v) => value == *v,
            S::Units(v) => value == *v,
            S::UnsignedValue(v) => value == *v,
            S::LifeSafetyMode(v) => value == *v,
            S::LifeSafetyState(v) => value == *v,
            S::DoorAlarmState(v) => value == *v,
            S::Action(v) => value == *v,
            S::DoorSecuredStatus(v) => value == *v,
            S::DoorStatus(v) => value == *v,
            S::DoorValue(v) => value == *v,
            S::LiftCarDirection(v) => value == *v,
            S::LiftCarDoorCommand(v) => value == *v,
            S::TimerState(v) => value == *v,
            S::TimerTransition(v) => value == *v,
            S::Other { .. } => false,
        };
        if matched {
            return EventState::OFFNORMAL;
        }
    }
    EventState::NORMAL
}

/// Structured CHANGE_OF_BITSTRING evaluation against a bitmask and alarm values.
fn eval_change_of_bitstring_struct(
    bitmask: &(u8, Vec<u8>),
    list_of_values: &[(u8, Vec<u8>)],
    value_bits: &[u8],
    _current: EventState,
) -> EventState {
    // OFFNORMAL if the masked monitored bits match any alarm pattern.
    let mask = &bitmask.1;
    for alarm in list_of_values {
        let alarm_bits = &alarm.1;
        let len = mask.len().min(alarm_bits.len()).min(value_bits.len());
        let mut matched = true;
        for i in 0..len {
            if (value_bits[i] & mask[i]) != (alarm_bits[i] & mask[i]) {
                matched = false;
                break;
            }
        }
        if matched && len > 0 {
            return EventState::OFFNORMAL;
        }
    }
    EventState::NORMAL
}

/// Structured CHANGE_OF_VALUE evaluation against a `cov-criteria`.
///
/// For a `bitmask` criterion the monitored value is a bitstring and the
/// algorithm reports OFFNORMAL when any masked bit is set; for a
/// `referenced-property-increment` criterion the monitored value is a real
/// and the algorithm reports OFFNORMAL when `|value| >= increment`. Returns
/// `None` when the monitored value is the wrong type for the criterion, so
/// the caller can skip the enrollment rather than spuriously transitioning
/// to `NORMAL`.
fn eval_change_of_value_struct(
    criteria: &bacnet_types::constructed::ChangeOfValueCriteria,
    monitored_value: &PropertyValue,
    _current: EventState,
) -> Option<EventState> {
    use bacnet_types::constructed::ChangeOfValueCriteria as C;
    match criteria {
        C::Bitmask { data, .. } => {
            let bits = extract_bitstring(monitored_value)?;
            let mut state = EventState::NORMAL;
            for i in 0..data.len().min(bits.len()) {
                if (bits[i] & data[i]) != 0 {
                    state = EventState::OFFNORMAL;
                    break;
                }
            }
            Some(state)
        }
        C::ReferencedPropertyIncrement(increment) => extract_real(monitored_value)
            .map(|v| eval_change_of_value(&increment.to_le_bytes(), v, EventState::NORMAL)),
    }
}

/// Read the setpoint referenced by a FLOATING_LIMIT enrollment.
///
/// Returns the referenced property's real value, or `None` if the reference is
/// remote or unreadable.
fn read_setpoint(
    db: &ObjectDatabase,
    reference: &bacnet_types::constructed::BACnetDeviceObjectPropertyReference,
) -> Option<f32> {
    // Remote-device setpoint references are not resolvable from a local DB.
    if reference.device_identifier.is_some() {
        return None;
    }
    let obj = db.get(&reference.object_identifier)?;
    let prop = PropertyIdentifier::from_raw(reference.property_identifier);
    extract_real(
        &obj.read_property(prop, reference.property_array_index)
            .ok()?,
    )
}

/// Legacy little-endian fallback for `Opaque` event parameters.
///
/// Used when an enrollment's `Event_Parameters` could not be decoded into a
/// structured alternative (e.g. raw octets written by an older client that
/// used the private little-endian byte layouts). The algorithm is inferred
/// from the enrollment's `Event_Type`, and the original byte-oriented
/// evaluators consume the opaque payload. Returns `current` (no transition)
/// when the `Event_Type` does not name a known evaluator or the monitored
/// value is the wrong type.
fn eval_legacy_le(
    data: &[u8],
    monitored_value: &PropertyValue,
    current: EventState,
    event_type: EventType,
) -> EventState {
    if event_type == EventType::OUT_OF_RANGE {
        extract_real(monitored_value)
            .map(|v| eval_out_of_range(data, v, current))
            .unwrap_or(current)
    } else if event_type == EventType::FLOATING_LIMIT {
        extract_real(monitored_value)
            .map(|v| eval_floating_limit(data, v, current))
            .unwrap_or(current)
    } else if event_type == EventType::CHANGE_OF_STATE {
        extract_enumerated(monitored_value)
            .map(|v| eval_change_of_state(data, v, current))
            .unwrap_or(current)
    } else if event_type == EventType::CHANGE_OF_BITSTRING {
        extract_bitstring(monitored_value)
            .map(|bits| eval_change_of_bitstring(data, &bits, current))
            .unwrap_or(current)
    } else if event_type == EventType::CHANGE_OF_VALUE {
        extract_real(monitored_value)
            .map(|v| eval_change_of_value(data, v, current))
            .unwrap_or(current)
    } else {
        current
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

    // (enrollment, monitored, event_type, from, to, distribute)
    let mut updates: Vec<(
        ObjectIdentifier,
        ObjectIdentifier,
        u32,
        EventState,
        EventState,
        bool,
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

        // Clause 13.2.2.1: "If the Event_Detection_Enable property is FALSE,
        // then this state machine is not evaluated. In this case, no
        // transitions shall occur". The accompanying reset is applied by the
        // object when the property is written (Clause 12.12 states the disabled
        // condition as an invariant), so skipping here cannot strand a stale
        // non-NORMAL state the way the pre-#136 Event_Enable gate did.
        //
        // An object that does not model the property at all reads as an error
        // here and is treated as enabled. The property is required (R) on both
        // Event Enrollment (Table 12-14) and Alert Enrollment (Table 12-61) and
        // optional on most other types, so absence is common and must not
        // silently disable detection.
        //
        // Removing this guard does NOT change observable behavior, and no test
        // fails if you do — `EventEnrollmentObject::set_event_state_internal`
        // independently refuses a non-NORMAL state while detection is off, and
        // the push below is gated on that call succeeding. Verified by mutation;
        // stated here so nobody deletes it believing it is covered. It stays for
        // two reasons the object-level guard cannot serve: it implements the
        // clause's first sentence literally (the algorithm genuinely does not
        // run, and the monitored object is not read), and without it every pass
        // over every disabled enrollment would do the full evaluation and then
        // silently swallow an `Err` — once per interval, forever.
        if let Ok(PropertyValue::Boolean(false)) =
            enrollment.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
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
            Ok(v) => match BACnetEventParameter::decode(&v) {
                Ok(ep) => ep,
                // Malformed structured value: nothing to evaluate.
                Err(_) => continue,
            },
            // Missing/unreadable Event_Parameters: nothing to evaluate.
            Err(_) => continue,
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
        let new_state = match &params {
            BACnetEventParameter::OutOfRange {
                high_limit,
                low_limit,
                deadband,
                ..
            } => {
                let Some(val) = extract_real(&monitored_value) else {
                    continue;
                };
                eval_out_of_range_struct(*low_limit, *high_limit, *deadband, val, current_state)
            }
            BACnetEventParameter::FloatingLimit {
                setpoint_reference,
                low_diff_limit,
                high_diff_limit,
                deadband,
                ..
            } => {
                let Some(setpoint) = read_setpoint(db, setpoint_reference) else {
                    continue;
                };
                let Some(val) = extract_real(&monitored_value) else {
                    continue;
                };
                eval_floating_limit_struct(
                    setpoint,
                    *high_diff_limit,
                    *low_diff_limit,
                    *deadband,
                    val,
                    current_state,
                )
            }
            BACnetEventParameter::ChangeOfState { list_of_values, .. } => {
                let Some(val) = extract_enumerated(&monitored_value) else {
                    continue;
                };
                eval_change_of_state_struct(list_of_values, val, current_state)
            }
            BACnetEventParameter::ChangeOfBitstring {
                bitmask,
                list_of_values,
                ..
            } => {
                let Some(bits) = extract_bitstring(&monitored_value) else {
                    continue;
                };
                eval_change_of_bitstring_struct(bitmask, list_of_values, &bits, current_state)
            }
            BACnetEventParameter::ChangeOfValue { criteria, .. } => {
                let Some(state) =
                    eval_change_of_value_struct(criteria, &monitored_value, current_state)
                else {
                    continue;
                };
                state
            }
            // Opaque/Extended/unmodeled algorithms: fall back to the legacy
            // little-endian byte layouts, dispatching on the enrollment's
            // Event_Type, preserving compatibility with values written by
            // older clients that used the raw-octets encoding.
            BACnetEventParameter::Opaque { data, .. } => {
                eval_legacy_le(data, &monitored_value, current_state, event_type)
            }
            // Extended [9] and any other modeled-but-unmodeled-for-evaluation
            // alternatives produce no transition here.
            _ => continue,
        };

        // Clause 13.2.2.1.4 requires the transition actions to run "even if the
        // transition does not change the event state", so this skip is not
        // conformant. Removing it alone would be worse: no evaluator here can
        // yet distinguish a genuine same-state indication from "nothing
        // changed", so an unguarded pass would re-fire every poll. Tracked as
        // #166, which depends on the change baseline from #137.
        if new_state == current_state {
            continue;
        }

        // `Event_Enable` governs distribution only (Clause 12.12). The
        // transition is recorded either way; the flag rides along so the
        // notification pipeline can suppress the send (#127).
        let distribute = match new_state {
            s if s == EventState::NORMAL => event_enable & 0x04 != 0,
            s if s == EventState::HIGH_LIMIT
                || s == EventState::LOW_LIMIT
                || s == EventState::OFFNORMAL =>
            {
                event_enable & 0x01 != 0
            }
            _ => event_enable & 0x02 != 0,
        };

        updates.push((
            *oid,
            monitored_oid,
            event_type_raw,
            current_state,
            new_state,
            distribute,
        ));
    }

    let mut transitions = Vec::new();
    for (oid, monitored_oid, event_type_raw, from_state, to_state, distribute) in updates {
        if let Some(obj) = db.get_mut(&oid) {
            // Persist the transition through the internal lifecycle path, not
            // the network `write_property(EVENT_STATE, …)` route. `Event_State`
            // is algorithmically derived (ASHRAE 135-2020 Clause 12.12) and
            // read-only over the network, so the evaluator reaches the field
            // via `set_event_state_internal` (issue #130).
            if obj.set_event_state_internal(to_state).is_ok() {
                transitions.push(EventEnrollmentTransition {
                    enrollment_oid: oid,
                    monitored_oid,
                    change: EventStateChange {
                        from: from_state,
                        to: to_state,
                    },
                    event_type: EventType::from_raw(event_type_raw),
                    distribute,
                });
            }
        }
    }

    transitions
}

#[cfg(test)]
mod tests;
