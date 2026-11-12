//! A request ends with its object when the database removes or replaces it
//! (#2479).

use super::*;
use crate::database::ObjectDatabase;
use crate::traits::BACnetObject;

/// A database on the test clock holding LC-1, which `app` sheds for: a
/// request from 10:00 under way and compliant when `request` is true, and
/// none otherwise.
fn database(
    app: &Arc<TestShedder>,
    request: bool,
) -> (ObjectDatabase, ObjectIdentifier, Arc<TestClock>) {
    let (mut lc, clock) = object();
    lc.set_shedder(app.clone());
    if request {
        write_start(&mut lc, at(0));
        assert_eq!(present_value(&lc), COMPLIANT);
    }
    app.take();
    let oid = lc.object_identifier();
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(clock.clone() as Arc<dyn ClockReader>));
    db.add(Box::new(lc)).unwrap();
    (db, oid, clock)
}

/// Present_Value and Start_Time as `object` serves them.
fn shed_state_of(object: &dyn BACnetObject) -> (PropertyValue, PropertyValue) {
    (
        object.read_property(P::PRESENT_VALUE, None).unwrap(),
        object.read_property(P::START_TIME, None).unwrap(),
    )
}

fn idle() -> (PropertyValue, PropertyValue) {
    (
        PropertyValue::Enumerated(INACTIVE.to_raw()),
        PropertyValue::List(vec![
            PropertyValue::Date(UNSPECIFIED_DATETIME.0),
            PropertyValue::Time(UNSPECIFIED_DATETIME.1),
        ]),
    )
}

#[test]
fn removing_the_object_during_a_request_stops_the_loads() {
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let (mut db, oid, _) = database(&app, true);
    let removed = db.remove(&oid).unwrap().unwrap();
    assert_eq!(app.take(), [Told::Stop]);
    // The object handed back holds no request, so nothing stops it again.
    assert_eq!(shed_state_of(removed.as_ref()), idle());
    drop(removed);
    assert!(app.take().is_empty());
}

#[test]
fn replacing_the_object_during_a_request_stops_the_loads() {
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let (mut db, oid, _) = database(&app, true);
    let successor = LoadControlObject::new(1, "LC-1").unwrap();
    db.add(Box::new(successor)).unwrap();
    assert_eq!(app.take(), [Told::Stop]);
    assert_eq!(shed_state_of(db.get(&oid).unwrap()), idle());
    // The new object's first request is its own: the old shedder hears
    // nothing more.
    db.remove(&oid).unwrap();
    assert!(app.take().is_empty());
}

#[test]
fn an_idle_object_leaves_without_telling_the_application() {
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    // Never asked about a request.
    let (mut db, oid, _) = database(&app, false);
    db.add(Box::new(LoadControlObject::new(1, "LC-1").unwrap()))
        .unwrap();
    db.remove(&oid).unwrap();
    assert!(app.take().is_empty());
    // A request that ran its course was stopped once, at its end.
    let (mut db, oid, clock) = database(&app, true);
    clock.set(at(30));
    db.for_each_object_mut(|_, object| {
        object
            .timed()
            .unwrap()
            .advance_monotonic_time(Duration::ZERO);
    });
    assert_eq!(app.take(), [Told::Stop]);
    let removed = db.remove(&oid).unwrap().unwrap();
    assert_eq!(shed_state_of(removed.as_ref()), idle());
    assert!(app.take().is_empty());
}

#[test]
fn a_pending_request_the_application_never_saw_ends_silently() {
    // Without a Device clock the request waits, never offered.
    let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    lc.set_shedder(app.clone());
    lc.set_requested_shed_level(BACnetShedLevel::Level(2))
        .unwrap();
    lc.set_shed_duration(30);
    write_start(&mut lc, at(0));
    assert_eq!(present_value(&lc), PENDING);
    let oid = lc.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(lc)).unwrap();
    let removed = db.remove(&oid).unwrap().unwrap();
    assert!(app.take().is_empty());
    assert_eq!(shed_state_of(removed.as_ref()), idle());
}
