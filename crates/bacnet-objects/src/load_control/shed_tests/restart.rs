//! A shed request kept across a restart (Clause 12.28, #2479): what is
//! saved, and how a rebuilt object looks at the request again.

use super::*;
use crate::durable::{DurableWrites, PendingWrite, StageStep};
use crate::load_control::{LoadControlPersistence, LoadControlSnapshot};
use crate::traits::BACnetObject;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Storage in memory, counting its saves, and refusing them while `fail`.
#[derive(Default)]
struct Store {
    saved: Mutex<Option<LoadControlSnapshot>>,
    saves: AtomicU32,
    fail: AtomicBool,
}

impl LoadControlPersistence for Store {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<LoadControlSnapshot>, Error> {
        Ok(self.saved.lock().unwrap().clone())
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &LoadControlSnapshot) -> Result<(), Error> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::Encoding("the disk is full".into()));
        }
        self.saves.fetch_add(1, Ordering::SeqCst);
        *self.saved.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

impl Store {
    fn held(&self) -> LoadControlSnapshot {
        self.saved.lock().unwrap().clone().expect("a save")
    }
}

/// LC-1 built from `store` on `clock`, with `app` installed. A first build
/// is configured as `object` configures one.
fn kept(store: &Arc<Store>, clock: &Arc<TestClock>, app: &Arc<TestShedder>) -> LoadControlObject {
    let first = store.saved.lock().unwrap().is_none();
    let mut lc = LoadControlObject::with_persistence(1, "LC-1", store.clone()).unwrap();
    lc.set_shedder(app.clone());
    lc.set_shed_levels(vec![(1, "one".into()), (2, "two".into())])
        .unwrap();
    if first {
        lc.set_requested_shed_level(BACnetShedLevel::Level(2))
            .unwrap();
        lc.set_shed_duration(30);
        lc.set_duty_window(10);
    }
    // Last, as a database binds it: a look before then can't read the
    // clock, so a restored request stays pending until the first look after.
    lc.bind_clocks(&Clocks::default().with_clock(Some(clock.clone() as Arc<dyn ClockReader>)));
    lc
}

fn test_clock() -> Arc<TestClock> {
    let clock = Arc::new(TestClock(Mutex::new(ClockFrame {
        local_date: DAY,
        local_time: at(0).1,
        utc_offset: 0,
        daylight_savings_status: false,
    })));
    clock.set(at(0));
    clock
}

/// Save what is queued and drop the object: the device goes down.
fn power_off(lc: LoadControlObject) {
    lc.wait_for_saves();
    drop(lc);
}

fn assert_request(lc: &LoadControlObject, start: (Date, Time)) {
    assert_eq!(start_time(lc), start);
    for (p, value) in [
        (P::REQUESTED_SHED_LEVEL, shed(2)),
        (P::SHED_DURATION, PropertyValue::Unsigned(30)),
        (P::DUTY_WINDOW, PropertyValue::Unsigned(10)),
    ] {
        assert_eq!(level(lc, p), value, "{p:?}");
    }
}

#[test]
fn a_pending_request_comes_back_and_waits_again() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Waiting, 2, 0);
    let mut lc = kept(&store, &clock, &app);
    write_start(&mut lc, at(5));
    assert_eq!(present_value(&lc), PENDING);
    power_off(lc);
    assert_eq!(store.held().state, PENDING);
    app.take();
    // Up again at 10:02: the request is back, pending, until a look decides
    // it as a Start_Time write would.
    clock.set(at(2));
    let mut lc = kept(&store, &clock, &app);
    assert_eq!(present_value(&lc), PENDING);
    assert_request(&lc, at(5));
    assert_eq!(level(&lc, P::ACTUAL_SHED_LEVEL), shed(0));
    lc.start_output();
    assert_eq!(present_value(&lc), PENDING);
    assert!(matches!(&app.take()[..], [Told::Shed(r)] if !r.started && r.start_time == at(5)));
    assert_eq!(level(&lc, P::EXPECTED_SHED_LEVEL), shed(2));
    // Start_Time comes and the loads comply.
    app.answer(ShedProgress::Compliant, 2, 2);
    assert!(pass(&mut lc, &clock, 5));
    assert_eq!(present_value(&lc), COMPLIANT);
}

#[test]
fn a_request_being_shed_comes_back_and_sheds_to_its_end() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    write_start(&mut lc, at(0));
    assert_eq!(present_value(&lc), COMPLIANT);
    power_off(lc);
    // The state the look reached was saved too.
    assert_eq!(store.held().state, COMPLIANT);
    app.take();
    clock.set(at(10));
    let mut lc = kept(&store, &clock, &app);
    assert_eq!(present_value(&lc), PENDING);
    lc.start_output();
    assert_eq!(present_value(&lc), COMPLIANT);
    assert!(matches!(&app.take()[..], [Told::Shed(r)] if r.started && r.shed_level == Some(1)));
    assert_eq!(level(&lc, P::ACTUAL_SHED_LEVEL), shed(2));
    // It ends at 10:30 as it would have, and storage follows.
    assert!(pass(&mut lc, &clock, 30));
    assert_idle(&lc, 0);
    assert_eq!(app.take().last(), Some(&Told::Stop));
    lc.wait_for_saves();
    assert_eq!(store.held(), LoadControlSnapshot::default());
}

#[test]
fn a_request_whose_time_ran_out_while_down_ends_at_once() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    lc.set_agreed_duty_window(15);
    lc.set_duty_window(10);
    write_start(&mut lc, at(0));
    power_off(lc);
    app.take();
    // Back at 10:45, past 10:30: the request is over and the application,
    // never asked about it since the restart, hears nothing.
    clock.set(at(45));
    let mut lc = kept(&store, &clock, &app);
    assert_eq!(present_value(&lc), PENDING);
    lc.start_output();
    assert_idle(&lc, 15);
    assert!(app.take().is_empty());
    lc.wait_for_saves();
    let idle = store.held();
    assert_eq!(
        (idle.state, idle.duty_window, idle.agreed_duty_window),
        (INACTIVE, 15, 15)
    );
}

#[test]
fn an_idle_object_keeps_its_configured_request_properties() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    lc.set_agreed_duty_window(15);
    lc.write_property(
        P::REQUESTED_SHED_LEVEL,
        None,
        shed_level_value(&BACnetShedLevel::Percent(80)),
        None,
    )
    .unwrap();
    power_off(lc);
    let mut lc = kept(&store, &clock, &app);
    lc.start_output();
    assert_eq!(present_value(&lc), INACTIVE);
    assert_eq!(
        level(&lc, P::REQUESTED_SHED_LEVEL),
        shed_level_value(&BACnetShedLevel::Percent(80))
    );
    assert_eq!(level(&lc, P::DUTY_WINDOW), PropertyValue::Unsigned(15));
    assert!(app.take().is_empty());
}

#[test]
fn a_write_that_cannot_be_saved_is_refused_and_changes_nothing() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    lc.wait_for_saves();
    store.fail.store(true, Ordering::SeqCst);
    let error = lc
        .write_property(
            P::START_TIME,
            None,
            PropertyValue::List(vec![
                PropertyValue::Date(at(0).0),
                PropertyValue::Time(at(0).1),
            ]),
            None,
        )
        .unwrap_err();
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
        if class == ErrorClass::DEVICE && code == ErrorCode::OPERATIONAL_PROBLEM)
    );
    assert_eq!(present_value(&lc), INACTIVE);
    assert_eq!(start_time(&lc), UNSPECIFIED_DATETIME);
    assert!(app.take().iter().all(|told| !matches!(told, Told::Shed(_))));
    // A value the object refuses is refused as it always is, before any save.
    assert!(matches!(
        lc.write_property(P::SHED_DURATION, None, PropertyValue::Real(1.0), None),
        Err(Error::Remote(RemoteError::Error { code, .. })) if code == ErrorCode::INVALID_DATA_TYPE
    ));
}

#[test]
fn a_staged_write_saves_before_it_is_served() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Waiting, 2, 0);
    let mut lc = kept(&store, &clock, &app);
    lc.wait_for_saves();
    let saves = store.saves.load(Ordering::SeqCst);
    let start = PropertyValue::List(vec![
        PropertyValue::Date(at(5).0),
        PropertyValue::Time(at(5).1),
    ]);
    let write = |value: &PropertyValue| PendingWrite {
        property: P::START_TIME,
        array_index: None,
        value: value.clone(),
    };
    let StageStep::Staged(wait) = lc.stage_writes(&[write(&start)]) else {
        panic!("a Start_Time write stages");
    };
    wait.block();
    // Storage holds the request before the object serves it.
    assert_eq!(store.held().state, PENDING);
    assert_eq!(store.held().start_time, at(5));
    assert_eq!(present_value(&lc), INACTIVE);
    lc.write_property(P::START_TIME, None, start.clone(), None)
        .unwrap();
    lc.release_staged_write(&wait);
    assert_eq!(present_value(&lc), PENDING);
    lc.wait_for_saves();
    // The look left what was staged, so nothing more was saved.
    assert_eq!(store.saves.load(Ordering::SeqCst), saves + 1);
    // A staged write its request never makes is dropped, and storage goes
    // back to the request served.
    let later = PropertyValue::List(vec![
        PropertyValue::Date(at(9).0),
        PropertyValue::Time(at(9).1),
    ]);
    let StageStep::Staged(wait) = lc.stage_writes(&[write(&later)]) else {
        panic!("a Start_Time write stages");
    };
    wait.block();
    assert_eq!(store.held().start_time, at(9));
    lc.release_staged_write(&wait);
    lc.wait_for_saves();
    assert_eq!(store.held().start_time, at(5));
    assert!(!lc.has_staged_write());
}

#[test]
fn a_saved_request_a_write_would_refuse_fails_the_build() {
    let store = Arc::new(Store::default());
    for refused in [
        LoadControlSnapshot {
            requested_shed_level: BACnetShedLevel::Percent(150),
            ..LoadControlSnapshot::default()
        },
        LoadControlSnapshot {
            start_time: (
                DAY,
                Time {
                    hour: 0xFF,
                    ..at(0).1
                },
            ),
            ..LoadControlSnapshot::default()
        },
        LoadControlSnapshot {
            state: ShedState::from_raw(7),
            ..LoadControlSnapshot::default()
        },
    ] {
        *store.saved.lock().unwrap() = Some(refused.clone());
        assert!(
            LoadControlObject::with_persistence(1, "LC-1", store.clone()).is_err(),
            "{refused:?}"
        );
    }
}

#[test]
fn removing_the_object_saves_its_ended_request() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    write_start(&mut lc, at(0));
    lc.retire();
    assert_eq!(app.take().last(), Some(&Told::Stop));
    lc.wait_for_saves();
    assert_eq!(store.held().state, INACTIVE);
    // A copy, such as a COV snapshot, saves nothing of its own.
    let copy = lc.cov_snapshot().unwrap();
    assert!(copy.read_property(P::PRESENT_VALUE, None).is_ok());
}
