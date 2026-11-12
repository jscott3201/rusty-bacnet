//! A shed request kept across a restart (Clause 12.28, #2479): what is
//! saved, and how a rebuilt object looks at the request again.

use super::*;
use crate::database::ObjectDatabase;
use crate::durable::{DurableWrites, PendingWrite, StageStep};
use crate::load_control::{LoadControlPersistence, LoadControlSnapshot};
use crate::traits::{BACnetObject, Retirement};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

mod levels;
mod retry;
mod setting;

/// Storage in memory, counting its saves and the calls to save, refusing
/// them while `fail`, and, once held, making each call report on one
/// channel and wait for the other.
#[derive(Default)]
struct Store {
    saved: Mutex<Option<LoadControlSnapshot>>,
    saves: AtomicU32,
    attempts: AtomicU32,
    fail: AtomicBool,
    hold: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
}

impl LoadControlPersistence for Store {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<LoadControlSnapshot>, Error> {
        Ok(self.saved.lock().unwrap().clone())
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &LoadControlSnapshot) -> Result<(), Error> {
        if let Some((started, go)) = &*self.hold.lock().unwrap() {
            let _ = started.send(());
            let _ = go.recv();
        }
        self.attempts.fetch_add(1, Ordering::SeqCst);
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
    // Back at 10:45, past 10:30: the request is over. It was being shed when
    // it was saved, so the application, never asked about it since the
    // restart, is told to stop once: loads it still holds shed go.
    clock.set(at(45));
    let mut lc = kept(&store, &clock, &app);
    assert_eq!(present_value(&lc), PENDING);
    lc.start_output();
    assert_idle(&lc, 15);
    assert_eq!(app.take(), [Told::Stop]);
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
    lc.retire(Retirement::Removed);
    assert_eq!(app.take().last(), Some(&Told::Stop));
    lc.wait_for_saves();
    assert_eq!(store.held().state, INACTIVE);
    // A copy, such as a COV snapshot, saves nothing of its own.
    let copy = lc.cov_snapshot().unwrap();
    assert!(copy.read_property(P::PRESENT_VALUE, None).is_ok());
}

#[test]
fn a_pending_request_that_ran_out_while_down_ends_silently() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Waiting, 2, 0);
    let mut lc = kept(&store, &clock, &app);
    write_start(&mut lc, at(5));
    assert_eq!(present_value(&lc), PENDING);
    power_off(lc);
    app.take();
    // Nothing was shed for it, so its end owes the application nothing.
    clock.set(at(45));
    let mut lc = kept(&store, &clock, &app);
    lc.start_output();
    assert_idle(&lc, 0);
    assert!(app.take().is_empty());
}

#[test]
fn a_stop_still_owed_survives_a_second_restart() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    write_start(&mut lc, at(0));
    power_off(lc);
    // Up and down again before any look with a clock: what is saved still
    // says the loads were shed.
    let mut lc = LoadControlObject::with_persistence(1, "LC-1", store.clone()).unwrap();
    lc.set_shedder(app.clone());
    lc.set_shed_levels(vec![(1, "one".into())]).unwrap();
    // A change that saves in this lifetime still saves the shed state.
    lc.set_agreed_duty_window(20);
    power_off(lc);
    assert_eq!(store.held().agreed_duty_window, 20);
    assert_eq!(store.held().state, COMPLIANT);
    app.take();
    clock.set(at(45));
    let mut lc = kept(&store, &clock, &app);
    lc.start_output();
    assert_eq!(app.take(), [Told::Stop]);
    // Once the restored request is offered, its end reaches the application
    // as any other's does, once.
    lc.set_requested_shed_level(BACnetShedLevel::Level(2))
        .unwrap();
    lc.set_shed_duration(30);
    write_start(&mut lc, at(40));
    power_off(lc);
    app.take();
    let mut lc = kept(&store, &clock, &app);
    lc.start_output();
    assert!(matches!(&app.take()[..], [Told::Shed(r)] if r.started));
    // It ends at 11:10, 10:40 plus 30.
    assert!(pass(&mut lc, &clock, 70));
    assert_eq!(app.take(), [Told::Stop]);
}

#[test]
fn enable_and_a_written_description_come_back() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    // A NULL changes nothing, so nothing is saved for it.
    lc.wait_for_saves();
    let saves = store.saves.load(Ordering::SeqCst);
    lc.write_property(P::DESCRIPTION, None, PropertyValue::Null, None)
        .unwrap();
    lc.wait_for_saves();
    assert_eq!(store.saves.load(Ordering::SeqCst), saves);
    assert_eq!(store.held().description, None);
    lc.write_property(
        P::DESCRIPTION,
        None,
        PropertyValue::CharacterString("Chillers".into()),
        None,
    )
    .unwrap();
    // One past 16 KiB is refused before anything is saved.
    let long = PropertyValue::CharacterString("x".repeat(16 * 1024 + 1));
    assert!(matches!(
        lc.write_property(P::DESCRIPTION, None, long, None),
        Err(Error::Remote(RemoteError::Error { code, .. }))
            if code == ErrorCode::NO_SPACE_TO_WRITE_PROPERTY
    ));
    lc.write_property(P::ENABLE, None, PropertyValue::Boolean(false), None)
        .unwrap();
    power_off(lc);
    let lc = kept(&store, &clock, &app);
    assert_eq!(
        lc.read_property(P::DESCRIPTION, None).unwrap(),
        PropertyValue::CharacterString("Chillers".into())
    );
    assert_eq!(
        lc.read_property(P::ENABLE, None).unwrap(),
        PropertyValue::Boolean(false)
    );
}

#[test]
fn a_request_restored_disabled_ends_as_a_written_false_ends_one() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    // Storage of another backend's, holding a shed under way with Enable
    // FALSE, which no save of this object writes.
    *store.saved.lock().unwrap() = Some(LoadControlSnapshot {
        state: COMPLIANT,
        requested_shed_level: BACnetShedLevel::Level(2),
        start_time: at(0),
        shed_duration: 30,
        duty_window: 10,
        enable: false,
        ..LoadControlSnapshot::default()
    });
    let mut lc = kept(&store, &clock, &app);
    lc.start_output();
    // Over with a stop, as a written FALSE leaves a shed, and no new one
    // starts while disabled.
    assert_idle(&lc, 0);
    assert_eq!(app.take(), [Told::Stop]);
    write_start(&mut lc, at(1));
    assert_eq!(present_value(&lc), INACTIVE);
    assert_eq!(start_time(&lc), UNSPECIFIED_DATETIME);
}

#[test]
fn a_replaced_object_leaves_its_storage_to_its_successor() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    write_start(&mut lc, at(0));
    lc.wait_for_saves();
    let oid = lc.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(lc)).unwrap();
    app.take();
    // The successor restores the request; the object it replaces stops its
    // loads but doesn't save its own end over the request.
    let successor = kept(&store, &clock, &app);
    db.add(Box::new(successor)).unwrap();
    assert_eq!(app.take(), [Told::Stop]);
    assert_eq!(store.held().state, COMPLIANT);
    assert_eq!(store.held().start_time, at(0));
    // Removed for good, the successor saves the request's end.
    let removed = db.remove(&oid).unwrap().unwrap();
    drop(removed);
    assert_eq!(store.held().state, INACTIVE);
}

#[test]
fn a_staged_save_is_what_the_object_serves_after_the_writes() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    lc.set_shed_duration(0);
    lc.wait_for_saves();
    // Start_Time now with no Shed_Duration yet: the request ends at once,
    // so the Shed_Duration after it is idle configuration, not a shed.
    let writes = [
        (
            P::START_TIME,
            PropertyValue::List(vec![
                PropertyValue::Date(at(0).0),
                PropertyValue::Time(at(0).1),
            ]),
        ),
        (P::SHED_DURATION, PropertyValue::Unsigned(30)),
    ];
    let pending: Vec<_> = writes
        .iter()
        .map(|(property, value)| PendingWrite {
            property: *property,
            array_index: None,
            value: value.clone(),
        })
        .collect();
    let StageStep::Staged(wait) = lc.stage_writes(&pending) else {
        panic!("the writes stage");
    };
    wait.block();
    let staged = store.held();
    for (property, value) in writes {
        lc.write_property(property, None, value, None).unwrap();
    }
    lc.release_staged_write(&wait);
    assert_eq!(present_value(&lc), INACTIVE);
    assert_eq!(staged.state, INACTIVE);
    assert_eq!(staged.start_time, UNSPECIFIED_DATETIME);
    assert_eq!(staged.shed_duration, 30);
    lc.wait_for_saves();
    assert_eq!(store.held(), staged);
}

/// The writes of one request, as the server hands them to be staged.
fn pending(writes: &[(P, PropertyValue)]) -> Vec<PendingWrite> {
    writes
        .iter()
        .map(|(property, value)| PendingWrite {
            property: *property,
            array_index: None,
            value: value.clone(),
        })
        .collect()
}

/// Stage `writes`, make them, and return what the staged save held.
fn stage_and_write(
    lc: &mut LoadControlObject,
    store: &Store,
    writes: &[(P, PropertyValue)],
) -> LoadControlSnapshot {
    let StageStep::Staged(wait) = lc.stage_writes(&pending(writes)) else {
        panic!("the writes stage");
    };
    wait.block();
    let staged = store.held();
    for (property, value) in writes {
        lc.write_property(*property, None, value.clone(), None)
            .unwrap();
    }
    lc.release_staged_write(&wait);
    staged
}

#[test]
fn a_description_staged_with_a_request_change_leaves_the_request_to_it() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    write_start(&mut lc, at(0));
    // 10:31, past the 30 minutes, before the once-a-second pass looked: a
    // Description changes nothing of the request, and the longer
    // Shed_Duration written with it carries the shed on.
    clock.set(at(31));
    lc.wait_for_saves();
    let staged = stage_and_write(
        &mut lc,
        &store,
        &[
            (
                P::DESCRIPTION,
                PropertyValue::CharacterString("Chillers".into()),
            ),
            (P::SHED_DURATION, PropertyValue::Unsigned(60)),
        ],
    );
    assert_eq!(present_value(&lc), COMPLIANT);
    assert_eq!((staged.state, staged.start_time), (COMPLIANT, at(0)));
    assert_eq!(staged.shed_duration, 60);
    lc.wait_for_saves();
    assert_eq!(store.held(), staged);
}

#[test]
fn a_staged_enable_false_ends_the_request_it_stages() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    write_start(&mut lc, at(0));
    lc.wait_for_saves();
    let staged = stage_and_write(
        &mut lc,
        &store,
        &[(P::ENABLE, PropertyValue::Boolean(false))],
    );
    assert_idle(&lc, 0);
    assert_eq!(app.take().last(), Some(&Told::Stop));
    assert_eq!(
        staged,
        LoadControlSnapshot {
            enable: false,
            ..LoadControlSnapshot::default()
        }
    );
    lc.wait_for_saves();
    assert_eq!(store.held(), staged);
}

#[test]
fn a_stop_owed_waits_for_a_shedder_installed_later() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    write_start(&mut lc, at(0));
    power_off(lc);
    app.take();
    // Back at 10:10 with no shedder yet: the look sheds nothing, and the
    // stop stays owed to the shedder installed after it.
    clock.set(at(10));
    let mut lc = LoadControlObject::with_persistence(1, "LC-1", store.clone()).unwrap();
    lc.bind_clocks(&Clocks::default().with_clock(Some(clock.clone() as Arc<dyn ClockReader>)));
    lc.start_output();
    assert_eq!(present_value(&lc), NON_COMPLIANT);
    lc.set_shedder(app.clone());
    lc.set_requested_shed_level(BACnetShedLevel::Level(0))
        .unwrap();
    assert_eq!(present_value(&lc), INACTIVE);
    assert_eq!(app.take(), [Told::Stop]);
}

#[test]
fn a_written_description_can_be_cleared_from_storage() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    let chillers = PropertyValue::CharacterString("Chillers".into());
    lc.write_property(P::DESCRIPTION, None, chillers.clone(), None)
        .unwrap();
    power_off(lc);
    let mut lc = kept(&store, &clock, &app);
    assert_eq!(store.held().description.as_deref(), Some("Chillers"));
    lc.clear_written_description().unwrap();
    // Storage keeps none now; the text served stays until a write changes it.
    assert_eq!(store.held().description, None);
    assert_eq!(lc.read_property(P::DESCRIPTION, None).unwrap(), chillers);
    // A save that fails keeps the written Description, and nothing to clear
    // is no save at all.
    lc.write_property(P::DESCRIPTION, None, chillers.clone(), None)
        .unwrap();
    lc.wait_for_saves();
    store.fail.store(true, Ordering::SeqCst);
    assert!(lc.clear_written_description().is_err());
    store.fail.store(false, Ordering::SeqCst);
    assert_eq!(store.held().description.as_deref(), Some("Chillers"));
    lc.clear_written_description().unwrap();
    let saves = store.saves.load(Ordering::SeqCst);
    lc.clear_written_description().unwrap();
    assert_eq!(store.saves.load(Ordering::SeqCst), saves);
    power_off(lc);
    let lc = kept(&store, &clock, &app);
    assert_eq!(
        lc.read_property(P::DESCRIPTION, None).unwrap(),
        PropertyValue::CharacterString(String::new())
    );
}
