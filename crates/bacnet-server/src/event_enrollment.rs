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
mod tests {
    use super::*;
    use bacnet_objects::analog::AnalogInputObject;
    use bacnet_objects::binary::BinaryInputObject;
    use bacnet_objects::event_enrollment::EventEnrollmentObject;
    use bacnet_objects::traits::BACnetObject;
    use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;

    /// Helper: create an EventEnrollment monitoring an AnalogInput with OUT_OF_RANGE.
    fn setup_out_of_range(
        present_value: f32,
        high_limit: f32,
        low_limit: f32,
        deadband: f32,
    ) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
        let mut db = ObjectDatabase::new();

        // Monitored analog input
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.set_present_value(present_value);
        let ai_oid = ai.object_identifier();
        db.add(Box::new(ai)).unwrap();

        // Event enrollment
        let mut ee =
            EventEnrollmentObject::new(1, "EE-OOR", EventType::OUT_OF_RANGE.to_raw()).unwrap();
        ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            ai_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )));
        ee.set_event_parameters(encode_out_of_range_params(high_limit, low_limit, deadband));
        ee.set_event_enable(0x07); // all transitions
        let ee_oid = ee.object_identifier();
        db.add(Box::new(ee)).unwrap();

        (db, ee_oid, ai_oid)
    }

    /// Helper: create an EventEnrollment monitoring an AnalogInput with FLOATING_LIMIT.
    fn setup_floating_limit(
        present_value: f32,
        setpoint: f32,
        high_diff: f32,
        low_diff: f32,
        deadband: f32,
    ) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
        let mut db = ObjectDatabase::new();

        let mut ai = AnalogInputObject::new(2, "AI-2", 62).unwrap();
        ai.set_present_value(present_value);
        let ai_oid = ai.object_identifier();
        db.add(Box::new(ai)).unwrap();

        let mut ee =
            EventEnrollmentObject::new(2, "EE-FL", EventType::FLOATING_LIMIT.to_raw()).unwrap();
        ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            ai_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )));
        ee.set_event_parameters(encode_floating_limit_params(
            setpoint, high_diff, low_diff, deadband,
        ));
        ee.set_event_enable(0x07);
        let ee_oid = ee.object_identifier();
        db.add(Box::new(ee)).unwrap();

        (db, ee_oid, ai_oid)
    }

    /// Helper: create an EventEnrollment monitoring a BinaryInput with CHANGE_OF_STATE.
    fn setup_change_of_state(
        present_value: u32,
        alarm_values: &[u32],
    ) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
        let mut db = ObjectDatabase::new();

        let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
        bi.set_present_value(present_value);
        let bi_oid = bi.object_identifier();
        db.add(Box::new(bi)).unwrap();

        let mut ee =
            EventEnrollmentObject::new(3, "EE-COS", EventType::CHANGE_OF_STATE.to_raw()).unwrap();
        ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            bi_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )));
        ee.set_event_parameters(encode_change_of_state_params(alarm_values));
        ee.set_event_enable(0x07);
        let ee_oid = ee.object_identifier();
        db.add(Box::new(ee)).unwrap();

        (db, ee_oid, bi_oid)
    }

    // ---- OUT_OF_RANGE tests ----

    #[test]
    fn out_of_range_normal_stays_normal() {
        let (mut db, _ee_oid, _ai_oid) = setup_out_of_range(50.0, 80.0, 20.0, 2.0);
        let transitions = evaluate_event_enrollments(&mut db);
        assert!(transitions.is_empty());
    }

    #[test]
    fn out_of_range_normal_to_high_limit() {
        let (mut db, ee_oid, ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
        let transitions = evaluate_event_enrollments(&mut db);
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].enrollment_oid, ee_oid);
        assert_eq!(transitions[0].monitored_oid, ai_oid);
        assert_eq!(transitions[0].change.from, EventState::NORMAL);
        assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
        assert_eq!(transitions[0].event_type, EventType::OUT_OF_RANGE);

        // Verify event_state was persisted
        let obj = db.get(&ee_oid).unwrap();
        assert_eq!(
            obj.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
        );
    }

    #[test]
    fn out_of_range_normal_to_low_limit() {
        let (mut db, ee_oid, _ai_oid) = setup_out_of_range(15.0, 80.0, 20.0, 2.0);
        let transitions = evaluate_event_enrollments(&mut db);
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].change.from, EventState::NORMAL);
        assert_eq!(transitions[0].change.to, EventState::LOW_LIMIT);

        // Verify persisted state
        let obj = db.get(&ee_oid).unwrap();
        assert_eq!(
            obj.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::LOW_LIMIT.to_raw())
        );
    }

    #[test]
    fn out_of_range_high_to_normal_with_deadband() {
        let (mut db, ee_oid, ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
        // First: go to HIGH_LIMIT
        evaluate_event_enrollments(&mut db);

        // Update monitored value — still within deadband (80 - 2 = 78)
        let ai = db.get_mut(&ai_oid).unwrap();
        ai.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(79.0),
            None,
        )
        .unwrap();

        let transitions = evaluate_event_enrollments(&mut db);
        assert!(transitions.is_empty(), "within deadband — no transition");

        // Drop below deadband
        let ai = db.get_mut(&ai_oid).unwrap();
        ai.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(77.0),
            None,
        )
        .unwrap();

        let transitions = evaluate_event_enrollments(&mut db);
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].change.from, EventState::HIGH_LIMIT);
        assert_eq!(transitions[0].change.to, EventState::NORMAL);

        let obj = db.get(&ee_oid).unwrap();
        assert_eq!(
            obj.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::NORMAL.to_raw())
        );
    }

    #[test]
    fn out_of_range_no_change_when_already_faulted() {
        let (mut db, _ee_oid, _ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
        let t1 = evaluate_event_enrollments(&mut db);
        assert_eq!(t1.len(), 1);

        // Second evaluation: same state, no new transition
        let t2 = evaluate_event_enrollments(&mut db);
        assert!(t2.is_empty());
    }

    #[test]
    fn out_of_range_event_enable_suppresses_notification() {
        let mut db = ObjectDatabase::new();

        let mut ai = AnalogInputObject::new(10, "AI-10", 62).unwrap();
        ai.set_present_value(85.0);
        let ai_oid = ai.object_identifier();
        db.add(Box::new(ai)).unwrap();

        let mut ee =
            EventEnrollmentObject::new(10, "EE-sup", EventType::OUT_OF_RANGE.to_raw()).unwrap();
        ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            ai_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )));
        ee.set_event_parameters(encode_out_of_range_params(80.0, 20.0, 2.0));
        ee.set_event_enable(0x04); // only TO_NORMAL enabled
        let ee_oid = ee.object_identifier();
        db.add(Box::new(ee)).unwrap();

        // TO_OFFNORMAL not enabled — should not appear in transitions
        let transitions = evaluate_event_enrollments(&mut db);
        assert!(transitions.is_empty());

        // But event_state should NOT have been updated (notification suppressed)
        let obj = db.get(&ee_oid).unwrap();
        assert_eq!(
            obj.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::NORMAL.to_raw())
        );
    }

    #[test]
    fn out_of_range_skips_out_of_service() {
        let (mut db, ee_oid, _ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);

        // Set enrollment to out-of-service
        let obj = db.get_mut(&ee_oid).unwrap();
        obj.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();

        let transitions = evaluate_event_enrollments(&mut db);
        assert!(transitions.is_empty());
    }

    // ---- FLOATING_LIMIT tests ----

    #[test]
    fn floating_limit_normal_stays_normal() {
        // setpoint=50, high_diff=10, low_diff=10 → limits at 60/40
        let (mut db, _ee_oid, _ai_oid) = setup_floating_limit(50.0, 50.0, 10.0, 10.0, 2.0);
        let transitions = evaluate_event_enrollments(&mut db);
        assert!(transitions.is_empty());
    }

    #[test]
    fn floating_limit_to_high() {
        // setpoint=50, high_diff=10 → high_limit=60; value=65 exceeds
        let (mut db, ee_oid, ai_oid) = setup_floating_limit(65.0, 50.0, 10.0, 10.0, 2.0);
        let transitions = evaluate_event_enrollments(&mut db);
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].enrollment_oid, ee_oid);
        assert_eq!(transitions[0].monitored_oid, ai_oid);
        assert_eq!(transitions[0].change.from, EventState::NORMAL);
        assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
        assert_eq!(transitions[0].event_type, EventType::FLOATING_LIMIT);
    }

    #[test]
    fn floating_limit_to_low() {
        // setpoint=50, low_diff=10 → low_limit=40; value=35 below
        let (mut db, _ee_oid, _ai_oid) = setup_floating_limit(35.0, 50.0, 10.0, 10.0, 2.0);
        let transitions = evaluate_event_enrollments(&mut db);
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].change.to, EventState::LOW_LIMIT);
    }

    #[test]
    fn floating_limit_deadband_hysteresis() {
        // setpoint=50, high_diff=10, deadband=2 → high_limit=60, return threshold=58
        let (mut db, _ee_oid, ai_oid) = setup_floating_limit(65.0, 50.0, 10.0, 10.0, 2.0);
        evaluate_event_enrollments(&mut db);

        // Still above return threshold (58)
        let ai = db.get_mut(&ai_oid).unwrap();
        ai.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(59.0),
            None,
        )
        .unwrap();
        let transitions = evaluate_event_enrollments(&mut db);
        assert!(transitions.is_empty());

        // Below return threshold
        let ai = db.get_mut(&ai_oid).unwrap();
        ai.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(57.0),
            None,
        )
        .unwrap();
        let transitions = evaluate_event_enrollments(&mut db);
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].change.to, EventState::NORMAL);
    }

    // ---- CHANGE_OF_STATE tests ----

    #[test]
    fn change_of_state_normal_when_not_in_alarm_set() {
        // Binary INACTIVE (0), alarm on ACTIVE (1)
        let (mut db, _ee_oid, _bi_oid) = setup_change_of_state(0, &[1]);
        let transitions = evaluate_event_enrollments(&mut db);
        assert!(transitions.is_empty());
    }

    #[test]
    fn change_of_state_to_offnormal() {
        // Binary ACTIVE (1), alarm on ACTIVE (1)
        let (mut db, ee_oid, bi_oid) = setup_change_of_state(1, &[1]);
        let transitions = evaluate_event_enrollments(&mut db);
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].enrollment_oid, ee_oid);
        assert_eq!(transitions[0].monitored_oid, bi_oid);
        assert_eq!(transitions[0].change.from, EventState::NORMAL);
        assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
        assert_eq!(transitions[0].event_type, EventType::CHANGE_OF_STATE);
    }

    #[test]
    fn change_of_state_back_to_normal() {
        let (mut db, _ee_oid, bi_oid) = setup_change_of_state(1, &[1]);
        evaluate_event_enrollments(&mut db);

        // Set monitored value to non-alarm
        let bi = db.get_mut(&bi_oid).unwrap();
        bi.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        bi.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(0),
            None,
        )
        .unwrap();

        let transitions = evaluate_event_enrollments(&mut db);
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].change.from, EventState::OFFNORMAL);
        assert_eq!(transitions[0].change.to, EventState::NORMAL);
    }

    #[test]
    fn change_of_state_multiple_alarm_values() {
        // Alarm on values 1, 3, 5
        let (mut db, _ee_oid, _bi_oid) = setup_change_of_state(3, &[1, 3, 5]);
        let transitions = evaluate_event_enrollments(&mut db);
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
    }

    // ---- CHANGE_OF_BITSTRING tests ----

    #[test]
    fn change_of_bitstring_normal() {
        let mut db = ObjectDatabase::new();

        // Create an object with a bitstring property (using a multistate or similar)
        // For testing, we'll use an EventEnrollment monitoring another enrollment's EVENT_ENABLE
        let mut target =
            EventEnrollmentObject::new(50, "Target", EventType::NONE.to_raw()).unwrap();
        // EVENT_ENABLE is a 3-bit bitstring
        target.set_event_enable(0x05); // bits: TO_OFFNORMAL | TO_NORMAL
        let target_oid = target.object_identifier();
        db.add(Box::new(target)).unwrap();

        let mut ee =
            EventEnrollmentObject::new(51, "EE-COBS", EventType::CHANGE_OF_BITSTRING.to_raw())
                .unwrap();
        ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            target_oid,
            PropertyIdentifier::EVENT_ENABLE.to_raw(),
        )));
        // mask=0xFF, alarm_pattern=0xE0 (all 3 high bits set)
        ee.set_event_parameters(encode_change_of_bitstring_params(&[0xFF], &[0xE0]));
        ee.set_event_enable(0x07);
        db.add(Box::new(ee)).unwrap();

        let transitions = evaluate_event_enrollments(&mut db);
        // 0x05 << 5 = 0xA0, mask 0xFF → 0xA0, alarm 0xE0 → no match → NORMAL
        assert!(transitions.is_empty());
    }

    #[test]
    fn change_of_bitstring_offnormal() {
        let mut db = ObjectDatabase::new();

        let mut target =
            EventEnrollmentObject::new(60, "Target2", EventType::NONE.to_raw()).unwrap();
        target.set_event_enable(0x07); // all 3 bits set
        let target_oid = target.object_identifier();
        db.add(Box::new(target)).unwrap();

        let mut ee =
            EventEnrollmentObject::new(61, "EE-COBS2", EventType::CHANGE_OF_BITSTRING.to_raw())
                .unwrap();
        ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            target_oid,
            PropertyIdentifier::EVENT_ENABLE.to_raw(),
        )));
        // mask=0xE0, alarm_pattern=0xE0 (all 3 high bits)
        ee.set_event_parameters(encode_change_of_bitstring_params(&[0xE0], &[0xE0]));
        ee.set_event_enable(0x07);
        db.add(Box::new(ee)).unwrap();

        let transitions = evaluate_event_enrollments(&mut db);
        // 0x07 << 5 = 0xE0, mask 0xE0 → 0xE0, alarm 0xE0 → match → OFFNORMAL
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
    }

    // ---- CHANGE_OF_VALUE tests ----

    #[test]
    fn change_of_value_within_increment() {
        let mut db = ObjectDatabase::new();

        let mut ai = AnalogInputObject::new(70, "AI-COV", 62).unwrap();
        ai.set_present_value(3.0);
        let ai_oid = ai.object_identifier();
        db.add(Box::new(ai)).unwrap();

        let mut ee =
            EventEnrollmentObject::new(70, "EE-COV", EventType::CHANGE_OF_VALUE.to_raw()).unwrap();
        ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            ai_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )));
        ee.set_event_parameters(encode_change_of_value_params(5.0));
        ee.set_event_enable(0x07);
        db.add(Box::new(ee)).unwrap();

        // |3.0| < 5.0 → NORMAL
        let transitions = evaluate_event_enrollments(&mut db);
        assert!(transitions.is_empty());
    }

    #[test]
    fn change_of_value_exceeds_increment() {
        let mut db = ObjectDatabase::new();

        let mut ai = AnalogInputObject::new(71, "AI-COV2", 62).unwrap();
        ai.set_present_value(10.0);
        let ai_oid = ai.object_identifier();
        db.add(Box::new(ai)).unwrap();

        let mut ee =
            EventEnrollmentObject::new(71, "EE-COV2", EventType::CHANGE_OF_VALUE.to_raw()).unwrap();
        ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            ai_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )));
        ee.set_event_parameters(encode_change_of_value_params(5.0));
        ee.set_event_enable(0x07);
        db.add(Box::new(ee)).unwrap();

        // |10.0| >= 5.0 → OFFNORMAL
        let transitions = evaluate_event_enrollments(&mut db);
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
    }

    // ---- Integration: multiple enrollments ----

    #[test]
    fn evaluates_multiple_enrollments() {
        let mut db = ObjectDatabase::new();

        // Two analog inputs
        let mut ai1 = AnalogInputObject::new(80, "AI-80", 62).unwrap();
        ai1.set_present_value(90.0); // will trigger HIGH_LIMIT
        let ai1_oid = ai1.object_identifier();
        db.add(Box::new(ai1)).unwrap();

        let mut ai2 = AnalogInputObject::new(81, "AI-81", 62).unwrap();
        ai2.set_present_value(50.0); // normal
        let ai2_oid = ai2.object_identifier();
        db.add(Box::new(ai2)).unwrap();

        // Two enrollments
        let mut ee1 =
            EventEnrollmentObject::new(80, "EE-80", EventType::OUT_OF_RANGE.to_raw()).unwrap();
        ee1.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            ai1_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )));
        ee1.set_event_parameters(encode_out_of_range_params(80.0, 20.0, 2.0));
        ee1.set_event_enable(0x07);
        db.add(Box::new(ee1)).unwrap();

        let mut ee2 =
            EventEnrollmentObject::new(81, "EE-81", EventType::OUT_OF_RANGE.to_raw()).unwrap();
        ee2.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            ai2_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )));
        ee2.set_event_parameters(encode_out_of_range_params(80.0, 20.0, 2.0));
        ee2.set_event_enable(0x07);
        db.add(Box::new(ee2)).unwrap();

        let transitions = evaluate_event_enrollments(&mut db);
        // Only AI-80 triggers (90 > 80)
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].monitored_oid, ai1_oid);
    }

    #[test]
    fn missing_monitored_object_is_skipped() {
        let mut db = ObjectDatabase::new();

        let fake_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 999).unwrap();
        let mut ee =
            EventEnrollmentObject::new(90, "EE-miss", EventType::OUT_OF_RANGE.to_raw()).unwrap();
        ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            fake_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )));
        ee.set_event_parameters(encode_out_of_range_params(80.0, 20.0, 2.0));
        ee.set_event_enable(0x07);
        db.add(Box::new(ee)).unwrap();

        // Should not panic or return transitions
        let transitions = evaluate_event_enrollments(&mut db);
        assert!(transitions.is_empty());
    }

    #[test]
    fn no_reference_is_skipped() {
        let mut db = ObjectDatabase::new();

        let ee =
            EventEnrollmentObject::new(91, "EE-noref", EventType::OUT_OF_RANGE.to_raw()).unwrap();
        db.add(Box::new(ee)).unwrap();

        let transitions = evaluate_event_enrollments(&mut db);
        assert!(transitions.is_empty());
    }

    #[test]
    fn empty_parameters_is_skipped() {
        let mut db = ObjectDatabase::new();

        let mut ai = AnalogInputObject::new(92, "AI-92", 62).unwrap();
        ai.set_present_value(100.0);
        let ai_oid = ai.object_identifier();
        db.add(Box::new(ai)).unwrap();

        let mut ee =
            EventEnrollmentObject::new(92, "EE-noparam", EventType::OUT_OF_RANGE.to_raw()).unwrap();
        ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            ai_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )));
        // No parameters set — should remain at current state
        ee.set_event_enable(0x07);
        db.add(Box::new(ee)).unwrap();

        let transitions = evaluate_event_enrollments(&mut db);
        assert!(transitions.is_empty());
    }
}
