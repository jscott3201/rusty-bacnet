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
mod tests {
    use super::*;
    use bacnet_objects::database::ObjectDatabase;
    use bacnet_objects::traits::BACnetObject;
    use bacnet_types::enums::ObjectType;
    use bacnet_types::error::Error;

    // ─────── Mock object for testing ───────

    /// A minimal mock object that stores properties in a HashMap for flexible
    /// testing of all five algorithms without depending on real object types.
    struct MockObject {
        oid: ObjectIdentifier,
        name: String,
        props: HashMap<PropertyIdentifier, PropertyValue>,
    }

    impl MockObject {
        fn new(oid: ObjectIdentifier) -> Self {
            Self {
                name: format!("mock-{}", oid),
                oid,
                props: HashMap::new(),
            }
        }

        fn set(&mut self, prop: PropertyIdentifier, val: PropertyValue) {
            self.props.insert(prop, val);
        }
    }

    impl BACnetObject for MockObject {
        fn object_identifier(&self) -> ObjectIdentifier {
            self.oid
        }

        fn object_name(&self) -> &str {
            &self.name
        }

        fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
            std::borrow::Cow::Owned(self.props.keys().copied().collect())
        }

        fn read_property(
            &self,
            property: PropertyIdentifier,
            _array_index: Option<u32>,
        ) -> Result<PropertyValue, Error> {
            self.props.get(&property).cloned().ok_or(Error::Protocol {
                class: 2, // PROPERTY
                code: 32, // UNKNOWN_PROPERTY
            })
        }

        fn write_property(
            &mut self,
            property: PropertyIdentifier,
            _array_index: Option<u32>,
            value: PropertyValue,
            _priority: Option<u8>,
        ) -> Result<(), Error> {
            self.props.insert(property, value);
            Ok(())
        }
    }

    // ─────── Helpers ───────

    fn make_oid(instance: u32) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::BINARY_INPUT, instance).unwrap()
    }

    fn make_analog_oid(instance: u32) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
    }

    fn setup_change_of_state(
        pv: u32,
        alarm_values: Vec<u32>,
    ) -> (ObjectDatabase, ObjectIdentifier) {
        let oid = make_oid(1);
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::CHANGE_OF_STATE.to_raw()),
        );
        obj.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x07),
        );
        obj.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj.set(
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(pv),
        );
        obj.set(PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(0));
        let alarm_pvs: Vec<PropertyValue> = alarm_values
            .into_iter()
            .map(PropertyValue::Enumerated)
            .collect();
        obj.set(
            PropertyIdentifier::ALARM_VALUES,
            PropertyValue::List(alarm_pvs),
        );

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();
        (db, oid)
    }

    // ═══════════════════════════════════════════════════════════════════════
    // T38: CHANGE_OF_STATE
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn cos_normal_to_offnormal() {
        let (db, oid) = setup_change_of_state(1, vec![1]); // pv=1 is in alarm_values
        let mut engine = IntrinsicReportingEngine::new();
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].object_id, oid);
        assert_eq!(events[0].change.from, EventState::NORMAL);
        assert_eq!(events[0].change.to, EventState::OFFNORMAL);
        assert_eq!(events[0].event_type, EventType::CHANGE_OF_STATE);
    }

    #[test]
    fn cos_stays_normal_when_not_in_alarm() {
        let (db, _oid) = setup_change_of_state(0, vec![1]); // pv=0 not in alarm
        let mut engine = IntrinsicReportingEngine::new();
        let events = engine.evaluate(&db);
        assert!(events.is_empty());
    }

    #[test]
    fn cos_offnormal_to_normal() {
        let (db, oid) = setup_change_of_state(1, vec![1]);
        let mut engine = IntrinsicReportingEngine::new();

        // First: NORMAL → OFFNORMAL
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.to, EventState::OFFNORMAL);

        // Change present_value to non-alarm
        let obj = db.get(&oid).unwrap();
        assert_eq!(
            obj.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(1)
        );

        // Create new DB with pv=0 (not in alarm)
        let (db2, _) = setup_change_of_state(0, vec![1]);
        // Transfer tracker state by re-using the engine.
        // But we need the same OID — our setup always uses BI:1.
        let events = engine.evaluate(&db2);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.from, EventState::OFFNORMAL);
        assert_eq!(events[0].change.to, EventState::NORMAL);
    }

    #[test]
    fn cos_time_delay_suppresses_spurious() {
        let oid = make_oid(2);
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::CHANGE_OF_STATE.to_raw()),
        );
        obj.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x07),
        );
        obj.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj.set(
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(1),
        );
        obj.set(PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(3));
        obj.set(
            PropertyIdentifier::ALARM_VALUES,
            PropertyValue::List(vec![PropertyValue::Enumerated(1)]),
        );

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();

        let mut engine = IntrinsicReportingEngine::new();

        // Tick 1: start pending
        assert!(engine.evaluate(&db).is_empty());
        // Tick 2: still pending
        assert!(engine.evaluate(&db).is_empty());
        // Tick 3: still pending (delay=3 means 3 ticks)
        assert!(engine.evaluate(&db).is_empty());
        // Tick 4: delay elapsed
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.to, EventState::OFFNORMAL);
    }

    #[test]
    fn cos_time_delay_cancelled_on_revert() {
        let oid = make_oid(3);
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::CHANGE_OF_STATE.to_raw()),
        );
        obj.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x07),
        );
        obj.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj.set(
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(1),
        );
        obj.set(PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(3));
        obj.set(
            PropertyIdentifier::ALARM_VALUES,
            PropertyValue::List(vec![PropertyValue::Enumerated(1)]),
        );

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();

        let mut engine = IntrinsicReportingEngine::new();

        // Tick 1: start pending
        assert!(engine.evaluate(&db).is_empty());
        // Tick 2: still pending
        assert!(engine.evaluate(&db).is_empty());

        // Now revert present_value to non-alarm
        let obj_mut = db.get_mut(&oid).unwrap();
        obj_mut
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Enumerated(0),
                None,
            )
            .unwrap();

        // Tick 3: pending cancelled
        assert!(engine.evaluate(&db).is_empty());
        // Tick 4: no transition
        assert!(engine.evaluate(&db).is_empty());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // T39: CHANGE_OF_BITSTRING
    // ═══════════════════════════════════════════════════════════════════════

    fn setup_bitstring(
        pv: &[u8],
        mask: &[u8],
        alarm_values: Vec<Vec<u8>>,
    ) -> (ObjectDatabase, ObjectIdentifier) {
        let oid = ObjectIdentifier::new(ObjectType::BINARY_INPUT, 10).unwrap();
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::CHANGE_OF_BITSTRING.to_raw()),
        );
        obj.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x07),
        );
        obj.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj.set(
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::BitString {
                unused_bits: 0,
                data: pv.to_vec(),
            },
        );
        obj.set(
            PropertyIdentifier::BIT_MASK,
            PropertyValue::BitString {
                unused_bits: 0,
                data: mask.to_vec(),
            },
        );
        obj.set(PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(0));

        let alarm_pvs: Vec<PropertyValue> = alarm_values
            .into_iter()
            .map(|d| PropertyValue::BitString {
                unused_bits: 0,
                data: d,
            })
            .collect();
        obj.set(
            PropertyIdentifier::ALARM_VALUES,
            PropertyValue::List(alarm_pvs),
        );

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();
        (db, oid)
    }

    #[test]
    fn cob_normal_to_offnormal() {
        // PV=0xFF, mask=0x0F → masked=0x0F. Alarm if masked==0x0F.
        let (db, _oid) = setup_bitstring(&[0xFF], &[0x0F], vec![vec![0x0F]]);
        let mut engine = IntrinsicReportingEngine::new();
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.to, EventState::OFFNORMAL);
    }

    #[test]
    fn cob_stays_normal_no_match() {
        // PV=0xFF, mask=0x0F → masked=0x0F. Alarm values=[0x03] — no match.
        let (db, _) = setup_bitstring(&[0xFF], &[0x0F], vec![vec![0x03]]);
        let mut engine = IntrinsicReportingEngine::new();
        assert!(engine.evaluate(&db).is_empty());
    }

    #[test]
    fn cob_offnormal_to_normal() {
        // Start in alarm, then change PV so it no longer matches.
        let (mut db, oid) = setup_bitstring(&[0xFF], &[0x0F], vec![vec![0x0F]]);
        let mut engine = IntrinsicReportingEngine::new();

        // → OFFNORMAL
        let events = engine.evaluate(&db);
        assert_eq!(events[0].change.to, EventState::OFFNORMAL);

        // Change PV so masked value won't match alarm
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::BitString {
                unused_bits: 0,
                data: vec![0xF0],
            },
            None,
        )
        .unwrap();

        // → NORMAL
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.to, EventState::NORMAL);
    }

    #[test]
    fn cob_time_delay() {
        let oid = ObjectIdentifier::new(ObjectType::BINARY_INPUT, 11).unwrap();
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::CHANGE_OF_BITSTRING.to_raw()),
        );
        obj.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x07),
        );
        obj.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj.set(
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::BitString {
                unused_bits: 0,
                data: vec![0xFF],
            },
        );
        obj.set(
            PropertyIdentifier::BIT_MASK,
            PropertyValue::BitString {
                unused_bits: 0,
                data: vec![0xFF],
            },
        );
        obj.set(PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(2));
        obj.set(
            PropertyIdentifier::ALARM_VALUES,
            PropertyValue::List(vec![PropertyValue::BitString {
                unused_bits: 0,
                data: vec![0xFF],
            }]),
        );

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();
        let mut engine = IntrinsicReportingEngine::new();

        // Tick 1: pending
        assert!(engine.evaluate(&db).is_empty());
        // Tick 2: still pending
        assert!(engine.evaluate(&db).is_empty());
        // Tick 3: fires
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // T40: FLOATING_LIMIT
    // ═══════════════════════════════════════════════════════════════════════

    fn setup_floating_limit(
        pv: f32,
        setpoint: f32,
        error_limit: f32,
        deadband: f32,
    ) -> (ObjectDatabase, ObjectIdentifier) {
        let oid = make_analog_oid(20);
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::FLOATING_LIMIT.to_raw()),
        );
        obj.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x07),
        );
        obj.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj.set(PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(pv));
        obj.set(
            PropertyIdentifier::SETPOINT_REFERENCE,
            PropertyValue::Real(setpoint),
        );
        obj.set(
            PropertyIdentifier::ERROR_LIMIT,
            PropertyValue::Real(error_limit),
        );
        obj.set(PropertyIdentifier::DEADBAND, PropertyValue::Real(deadband));
        obj.set(PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(0));
        // Both limits enabled: bit 0 (MSB) = low, bit 1 = high.
        obj.set(
            PropertyIdentifier::LIMIT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![0xC0],
            },
        );

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();
        (db, oid)
    }

    #[test]
    fn fl_normal_stays_normal_within_band() {
        // setpoint=50, error_limit=10 → high=60, low=40. pv=55 is within.
        let (db, _) = setup_floating_limit(55.0, 50.0, 10.0, 2.0);
        let mut engine = IntrinsicReportingEngine::new();
        assert!(engine.evaluate(&db).is_empty());
    }

    #[test]
    fn fl_normal_to_high_limit() {
        // setpoint=50, error_limit=10 → high=60. pv=61 exceeds.
        let (db, _) = setup_floating_limit(61.0, 50.0, 10.0, 2.0);
        let mut engine = IntrinsicReportingEngine::new();
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.to, EventState::HIGH_LIMIT);
    }

    #[test]
    fn fl_normal_to_low_limit() {
        // setpoint=50, error_limit=10 → low=40. pv=39 below.
        let (db, _) = setup_floating_limit(39.0, 50.0, 10.0, 2.0);
        let mut engine = IntrinsicReportingEngine::new();
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.to, EventState::LOW_LIMIT);
    }

    #[test]
    fn fl_high_limit_to_normal_with_deadband() {
        // setpoint=50, error_limit=10, deadband=2 → high=60.
        // Need pv < 60-2=58 to return to NORMAL.
        let (mut db, oid) = setup_floating_limit(61.0, 50.0, 10.0, 2.0);
        let mut engine = IntrinsicReportingEngine::new();

        // → HIGH_LIMIT
        let events = engine.evaluate(&db);
        assert_eq!(events[0].change.to, EventState::HIGH_LIMIT);

        // pv=59 — still in deadband (>= 58)
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(59.0),
            None,
        )
        .unwrap();
        assert!(engine.evaluate(&db).is_empty());

        // pv=57 — below deadband → NORMAL
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(57.0),
            None,
        )
        .unwrap();
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.to, EventState::NORMAL);
    }

    #[test]
    fn fl_low_limit_to_normal_with_deadband() {
        // setpoint=50, error_limit=10, deadband=2 → low=40.
        // Need pv > 40+2=42 to return to NORMAL.
        let (mut db, oid) = setup_floating_limit(39.0, 50.0, 10.0, 2.0);
        let mut engine = IntrinsicReportingEngine::new();

        // → LOW_LIMIT
        let events = engine.evaluate(&db);
        assert_eq!(events[0].change.to, EventState::LOW_LIMIT);

        // pv=41 — still in deadband (<= 42)
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(41.0),
            None,
        )
        .unwrap();
        assert!(engine.evaluate(&db).is_empty());

        // pv=43 — above deadband → NORMAL
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(43.0),
            None,
        )
        .unwrap();
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.to, EventState::NORMAL);
    }

    #[test]
    fn fl_high_to_low_direct() {
        let (mut db, oid) = setup_floating_limit(61.0, 50.0, 10.0, 2.0);
        let mut engine = IntrinsicReportingEngine::new();

        // → HIGH_LIMIT
        engine.evaluate(&db);

        // Jump to below low_limit
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(39.0),
            None,
        )
        .unwrap();
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.from, EventState::HIGH_LIMIT);
        assert_eq!(events[0].change.to, EventState::LOW_LIMIT);
    }

    #[test]
    fn fl_limit_enable_high_only() {
        // Only high limit enabled — low limit violations should be ignored.
        let oid = make_analog_oid(21);
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::FLOATING_LIMIT.to_raw()),
        );
        obj.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x07),
        );
        obj.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj.set(PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(39.0));
        obj.set(
            PropertyIdentifier::SETPOINT_REFERENCE,
            PropertyValue::Real(50.0),
        );
        obj.set(PropertyIdentifier::ERROR_LIMIT, PropertyValue::Real(10.0));
        obj.set(PropertyIdentifier::DEADBAND, PropertyValue::Real(2.0));
        obj.set(PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(0));
        // Only high limit enabled (bit 1 = 0x40).
        obj.set(
            PropertyIdentifier::LIMIT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![0x40],
            },
        );

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();
        let mut engine = IntrinsicReportingEngine::new();

        // pv=39 < low_limit=40 but low not enabled — stays NORMAL
        assert!(engine.evaluate(&db).is_empty());
    }

    #[test]
    fn fl_at_boundary_no_transition() {
        // pv exactly at high_limit (60) — not exceeded, stays NORMAL.
        let (db, _) = setup_floating_limit(60.0, 50.0, 10.0, 2.0);
        let mut engine = IntrinsicReportingEngine::new();
        assert!(engine.evaluate(&db).is_empty());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // T41: COMMAND_FAILURE
    // ═══════════════════════════════════════════════════════════════════════

    fn setup_command_failure(pv: u32, feedback: u32) -> (ObjectDatabase, ObjectIdentifier) {
        let oid = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 30).unwrap();
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::COMMAND_FAILURE.to_raw()),
        );
        obj.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x07),
        );
        obj.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj.set(
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(pv),
        );
        obj.set(
            PropertyIdentifier::FEEDBACK_VALUE,
            PropertyValue::Enumerated(feedback),
        );
        obj.set(PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(0));

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();
        (db, oid)
    }

    #[test]
    fn cf_mismatch_to_offnormal() {
        let (db, _) = setup_command_failure(1, 0); // pv != feedback
        let mut engine = IntrinsicReportingEngine::new();
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.to, EventState::OFFNORMAL);
        assert_eq!(events[0].event_type, EventType::COMMAND_FAILURE);
    }

    #[test]
    fn cf_match_stays_normal() {
        let (db, _) = setup_command_failure(1, 1); // pv == feedback
        let mut engine = IntrinsicReportingEngine::new();
        assert!(engine.evaluate(&db).is_empty());
    }

    #[test]
    fn cf_offnormal_to_normal() {
        let (mut db, oid) = setup_command_failure(1, 0);
        let mut engine = IntrinsicReportingEngine::new();

        // → OFFNORMAL
        let events = engine.evaluate(&db);
        assert_eq!(events[0].change.to, EventState::OFFNORMAL);

        // Fix feedback to match
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::FEEDBACK_VALUE,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();

        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.from, EventState::OFFNORMAL);
        assert_eq!(events[0].change.to, EventState::NORMAL);
    }

    #[test]
    fn cf_time_delay() {
        let oid = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 31).unwrap();
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::COMMAND_FAILURE.to_raw()),
        );
        obj.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x07),
        );
        obj.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj.set(
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(1),
        );
        obj.set(
            PropertyIdentifier::FEEDBACK_VALUE,
            PropertyValue::Enumerated(0),
        );
        obj.set(PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(2));

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();
        let mut engine = IntrinsicReportingEngine::new();

        assert!(engine.evaluate(&db).is_empty()); // tick 1
        assert!(engine.evaluate(&db).is_empty()); // tick 2
        let events = engine.evaluate(&db); // tick 3: fires
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.to, EventState::OFFNORMAL);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // T42: CHANGE_OF_VALUE
    // ═══════════════════════════════════════════════════════════════════════

    fn setup_cov_analog(pv: f32, increment: f32) -> (ObjectDatabase, ObjectIdentifier) {
        let oid = make_analog_oid(40);
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::CHANGE_OF_VALUE.to_raw()),
        );
        obj.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x07),
        );
        obj.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj.set(PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(pv));
        obj.set(
            PropertyIdentifier::COV_INCREMENT,
            PropertyValue::Real(increment),
        );

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();
        (db, oid)
    }

    #[test]
    fn cov_first_evaluation_always_notifies() {
        let (db, _) = setup_cov_analog(50.0, 5.0);
        let mut engine = IntrinsicReportingEngine::new();
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::CHANGE_OF_VALUE);
    }

    #[test]
    fn cov_within_increment_no_notify() {
        let (mut db, oid) = setup_cov_analog(50.0, 5.0);
        let mut engine = IntrinsicReportingEngine::new();

        // First eval: fires (initial)
        engine.evaluate(&db);

        // Change by less than increment
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(53.0),
            None,
        )
        .unwrap();

        // Should not fire (delta=3 < increment=5)
        assert!(engine.evaluate(&db).is_empty());
    }

    #[test]
    fn cov_exceeds_increment_notifies() {
        let (mut db, oid) = setup_cov_analog(50.0, 5.0);
        let mut engine = IntrinsicReportingEngine::new();

        // First eval: fires
        engine.evaluate(&db);

        // Change by exactly increment
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(55.0),
            None,
        )
        .unwrap();

        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn cov_binary_state_change() {
        let oid = make_oid(40);
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::CHANGE_OF_VALUE.to_raw()),
        );
        obj.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x07),
        );
        obj.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj.set(
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(0),
        );

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();
        let mut engine = IntrinsicReportingEngine::new();

        // First eval: fires
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);

        // Same value: no notification
        assert!(engine.evaluate(&db).is_empty());

        // Change binary state
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();

        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn cov_binary_no_change_no_notify() {
        let oid = make_oid(41);
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::CHANGE_OF_VALUE.to_raw()),
        );
        obj.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x07),
        );
        obj.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj.set(
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(1),
        );

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();
        let mut engine = IntrinsicReportingEngine::new();

        // First eval fires
        engine.evaluate(&db);
        // Second eval: same state, no fire
        assert!(engine.evaluate(&db).is_empty());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Edge cases
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn objects_without_event_type_are_skipped() {
        let oid = make_oid(99);
        let mut obj = MockObject::new(oid);
        // No EVENT_TYPE property set
        obj.set(
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(1),
        );

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();
        let mut engine = IntrinsicReportingEngine::new();
        assert!(engine.evaluate(&db).is_empty());
    }

    #[test]
    fn event_enable_zero_skips_object() {
        let oid = make_oid(100);
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::CHANGE_OF_STATE.to_raw()),
        );
        obj.set(PropertyIdentifier::EVENT_ENABLE, PropertyValue::Unsigned(0));
        obj.set(
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(1),
        );
        obj.set(
            PropertyIdentifier::ALARM_VALUES,
            PropertyValue::List(vec![PropertyValue::Enumerated(1)]),
        );

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();
        let mut engine = IntrinsicReportingEngine::new();
        assert!(engine.evaluate(&db).is_empty());
    }

    #[test]
    fn multiple_objects_evaluated() {
        let (mut db, _) = setup_change_of_state(1, vec![1]);

        let oid2 = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 50).unwrap();
        let mut obj2 = MockObject::new(oid2);
        obj2.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::COMMAND_FAILURE.to_raw()),
        );
        obj2.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x07),
        );
        obj2.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj2.set(
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(1),
        );
        obj2.set(
            PropertyIdentifier::FEEDBACK_VALUE,
            PropertyValue::Enumerated(0),
        );
        obj2.set(PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(0));
        db.add(Box::new(obj2)).unwrap();

        let mut engine = IntrinsicReportingEngine::new();
        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn cos_event_enable_to_normal_only() {
        // Only TO_NORMAL enabled (0x04).
        let oid = make_oid(101);
        let mut obj = MockObject::new(oid);
        obj.set(
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::CHANGE_OF_STATE.to_raw()),
        );
        obj.set(
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::Unsigned(0x04), // only TO_NORMAL
        );
        obj.set(
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        );
        obj.set(
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(1),
        );
        obj.set(PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(0));
        obj.set(
            PropertyIdentifier::ALARM_VALUES,
            PropertyValue::List(vec![PropertyValue::Enumerated(1)]),
        );

        let mut db = ObjectDatabase::new();
        db.add(Box::new(obj)).unwrap();
        let mut engine = IntrinsicReportingEngine::new();

        // TO_OFFNORMAL suppressed, but state still updates internally
        assert!(engine.evaluate(&db).is_empty());

        // Now revert to normal — TO_NORMAL is enabled so it should fire
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(0),
            None,
        )
        .unwrap();

        let events = engine.evaluate(&db);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].change.from, EventState::OFFNORMAL);
        assert_eq!(events[0].change.to, EventState::NORMAL);
    }
}
