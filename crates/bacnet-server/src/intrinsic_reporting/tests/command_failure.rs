use super::*;

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
