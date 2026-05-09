use super::*;

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
