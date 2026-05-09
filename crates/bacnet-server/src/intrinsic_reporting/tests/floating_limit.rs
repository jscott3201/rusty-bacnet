use super::*;

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
