use super::*;

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
