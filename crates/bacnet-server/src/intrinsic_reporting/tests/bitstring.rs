use super::*;

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
