//! The sweep's contract with its host: `before_commit` sees each object as
//! it was before its proposal commits.

use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EngineeringUnits, EventState, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;

use super::*;

/// An Analog Input reading 90 against a high limit of 80, with a one-second
/// Time_Delay, reporting through Notification Class 0.
fn high_limit_database() -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(NotificationClass::new(0, "NC-0").unwrap()))
        .unwrap();
    let mut object = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    for (property, value) in [
        (PropertyIdentifier::HIGH_LIMIT, 80.0f32),
        (PropertyIdentifier::LOW_LIMIT, 0.0),
        (PropertyIdentifier::DEADBAND, 1.0),
    ] {
        object
            .write_property(property, None, PropertyValue::Real(value), None)
            .unwrap();
    }
    for (property, unused_bits, bits) in [
        (PropertyIdentifier::LIMIT_ENABLE, 6, 0xC0),
        (PropertyIdentifier::EVENT_ENABLE, 5, 0xE0),
    ] {
        object
            .write_property(
                property,
                None,
                PropertyValue::BitString {
                    unused_bits,
                    data: vec![bits],
                },
                None,
            )
            .unwrap();
    }
    object
        .write_property(
            PropertyIdentifier::TIME_DELAY,
            None,
            PropertyValue::Unsigned(1),
            None,
        )
        .unwrap();
    object.set_present_value(90.0);
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    db.add(Box::new(object)).unwrap();
    (db, oid)
}

fn event_state(db: &ObjectDatabase, oid: ObjectIdentifier) -> EventState {
    match db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap()
    {
        PropertyValue::Enumerated(state) => EventState::from_raw(state),
        other => panic!("Event_State read {other:?}"),
    }
}

#[test]
fn before_commit_sees_the_state_from_before_the_commit() {
    let (mut db, oid) = high_limit_database();
    // The evaluation seeds the delay; nothing commits yet.
    assert!(after_write(&mut db, &oid).is_none());

    let mut seen = Vec::new();
    let mut committed = Vec::new();
    for _ in 0..3 {
        committed = sweep(&mut db, |db, object| {
            seen.push((object, event_state(db, object)));
        });
        if !committed.is_empty() {
            break;
        }
    }

    assert_eq!(seen, vec![(oid, EventState::NORMAL)]);
    let [(swept, Ok(transition))] = committed.as_slice() else {
        panic!("the sweep committed nothing");
    };
    assert_eq!(*swept, oid);
    assert_eq!(transition.change.to, EventState::HIGH_LIMIT);
    assert_eq!(event_state(&db, oid), EventState::HIGH_LIMIT);
}
