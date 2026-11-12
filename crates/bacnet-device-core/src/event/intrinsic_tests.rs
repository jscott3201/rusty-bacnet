//! The sweep's contract with its host: `before_commit` sees each object as
//! it was before its proposal commits.

use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_time::Instant;
use bacnet_types::enums::{EngineeringUnits, EventState, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;

use super::*;

/// An Analog Input reading 90 against a high limit of 80, with
/// `time_delay`, reporting through Notification Class 0.
fn high_limit_input(instance: u32, time_delay: u64) -> AnalogInputObject {
    let name = ["AI-1", "AI-2", "AI-3"][usize::try_from(instance).unwrap() - 1];
    let mut object =
        AnalogInputObject::new(instance, name, EngineeringUnits::DEGREES_CELSIUS).unwrap();
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
            PropertyValue::Unsigned(time_delay),
            None,
        )
        .unwrap();
    object.set_present_value(90.0);
    object
}

fn input(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

/// [`high_limit_input`]s with these Time_Delays, instances from 1.
fn database(delays: &[u64]) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(NotificationClass::new(0, "NC-0").unwrap()))
        .unwrap();
    for (instance, delay) in (1..).zip(delays) {
        db.add(Box::new(high_limit_input(instance, *delay)))
            .unwrap();
    }
    db
}

/// One Analog Input with a one-second Time_Delay.
fn high_limit_database() -> (ObjectDatabase, ObjectIdentifier) {
    (database(&[1]), input(1))
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
    // The evaluation seeds the delay, due a second later; nothing commits
    // yet.
    assert!(after_write(&mut db, &oid, Instant::from_millis(500)).is_none());
    assert_eq!(
        next_due(&db, Instant::from_millis(500)),
        Some(Instant::from_millis(1_500))
    );

    let mut seen = Vec::new();
    assert!(
        sweep(&mut db, Instant::from_secs(1), |_, object| seen
            .push((object, EventState::FAULT)))
        .is_empty()
    );
    assert!(seen.is_empty(), "nothing proposed before the delay ends");
    let committed = sweep(&mut db, Instant::from_millis(1_500), |db, object| {
        seen.push((object, event_state(db, object)));
    });
    assert_eq!(next_due(&db, Instant::from_millis(1_500)), None);

    assert_eq!(seen, vec![(oid, EventState::NORMAL)]);
    let [(swept, Ok(transition))] = committed.as_slice() else {
        panic!("the sweep committed nothing");
    };
    assert_eq!(*swept, oid);
    assert_eq!(transition.change.to, EventState::HIGH_LIMIT);
    assert_eq!(event_state(&db, oid), EventState::HIGH_LIMIT);
}

/// `next_due` is the earliest due instant after the one it is given: two
/// writes' delays end in turn, and a Time_Delay of 0 the sweep noticed,
/// pending until the next sweep with its due instant already reached, hides
/// neither of them.
#[test]
fn next_due_is_the_earliest_after_the_given_instant() {
    let mut db = database(&[1, 1, 0]);
    assert!(after_write(&mut db, &input(1), Instant::from_millis(200)).is_none());
    assert!(after_write(&mut db, &input(2), Instant::from_millis(500)).is_none());
    // The sweep at 1 s notices the third input's condition and seeds it,
    // due at once; it fires at the next sweep.
    assert!(sweep(&mut db, Instant::from_secs(1), |_, _| {}).is_empty());
    assert_eq!(
        next_due(&db, Instant::from_millis(999)),
        Some(Instant::from_secs(1))
    );
    assert_eq!(
        next_due(&db, Instant::from_secs(1)),
        Some(Instant::from_millis(1_200))
    );
    assert_eq!(
        next_due(&db, Instant::from_millis(1_200)),
        Some(Instant::from_millis(1_500))
    );
    assert_eq!(next_due(&db, Instant::from_millis(1_500)), None);

    let fired = sweep(&mut db, Instant::from_millis(1_200), |_, _| {});
    let mut objects: Vec<_> = fired.iter().map(|(object, _)| *object).collect();
    objects.sort_by_key(|object| object.instance_number());
    assert_eq!(objects, [input(1), input(3)]);
    assert_eq!(
        next_due(&db, Instant::from_millis(1_200)),
        Some(Instant::from_millis(1_500))
    );
}
