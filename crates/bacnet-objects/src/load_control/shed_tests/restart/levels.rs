//! Shed_Levels elements a peer wrote, kept across a restart (#2502): only
//! the written ones are saved, they win over the application's levels at
//! start-up, the running server's route and the clear drop them, and a
//! restored request is put to the shedder with them.

use super::*;
use crate::load_control::{LoadControlSetting, MAX_WRITTEN_SHED_LEVELS};
use crate::traits::{LocalInput, LocalInputValue};

fn levels(lc: &LoadControlObject) -> PropertyValue {
    lc.read_property(P::SHED_LEVELS, None).unwrap()
}

fn unsigned(values: &[u64]) -> PropertyValue {
    PropertyValue::List(values.iter().map(|&v| PropertyValue::Unsigned(v)).collect())
}

fn saves(store: &Store) -> u32 {
    store.saves.load(Ordering::SeqCst)
}

/// LC-1 from `store` as [`kept`] builds it, but given `levels` at start-up.
fn kept_with(store: &Arc<Store>, clock: &Arc<TestClock>, levels: &[u64]) -> LoadControlObject {
    let mut lc = LoadControlObject::with_persistence(1, "LC-1", store.clone()).unwrap();
    let configured = levels.iter().map(|&level| (level, format!("{level}")));
    lc.set_shed_levels(configured.collect()).unwrap();
    lc.bind_clocks(&Clocks::default().with_clock(Some(clock.clone() as Arc<dyn ClockReader>)));
    lc
}

fn error_code(error: Error) -> ErrorCode {
    match error {
        Error::Remote(RemoteError::Error { code, .. }) => code,
        other => panic!("not a BACnet error: {other:?}"),
    }
}

#[test]
fn a_whole_array_written_comes_back_over_the_start_up_levels() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    // The levels the application gave are configuration: nothing saved.
    lc.wait_for_saves();
    assert!(store.held().shed_levels.is_empty());
    lc.write_property(P::SHED_LEVELS, None, unsigned(&[3, 7]), None)
        .unwrap();
    assert_eq!(store.held().shed_levels, [(1, 3), (2, 7)]);
    power_off(lc);
    let lc = kept(&store, &clock, &app);
    assert_eq!(levels(&lc), unsigned(&[3, 7]));
    // The descriptions are the application's still.
    assert_eq!(
        lc.read_property(P::SHED_LEVEL_DESCRIPTIONS, Some(2))
            .unwrap(),
        PropertyValue::CharacterString("two".into())
    );
}

#[test]
fn one_element_written_comes_back_and_the_others_follow_the_application() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    lc.write_property(P::SHED_LEVELS, Some(2), PropertyValue::Unsigned(9), None)
        .unwrap();
    power_off(lc);
    // Only the element written is saved.
    assert_eq!(store.held().shed_levels, [(2, 9)]);
    let lc = kept(&store, &clock, &app);
    assert_eq!(levels(&lc), unsigned(&[1, 9]));
    power_off(lc);
    // The application gives new levels at the next start: the written
    // element wins over its own, the rest are the application's, and one
    // past the shorter array goes from storage.
    let lc = kept_with(&store, &clock, &[4, 5, 6]);
    assert_eq!(levels(&lc), unsigned(&[4, 9, 6]));
    power_off(lc);
    let lc = kept_with(&store, &clock, &[8]);
    assert_eq!(levels(&lc), unsigned(&[8]));
    power_off(lc);
    assert!(store.held().shed_levels.is_empty());
    let lc = kept_with(&store, &clock, &[4, 5]);
    assert_eq!(levels(&lc), unsigned(&[4, 5]));
}

#[test]
fn a_restored_request_is_put_to_the_shedder_with_the_written_levels() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    // Level 2 picks the element valued 2: the second of the application's
    // [1, 2], the first of the written [2, 5].
    lc.write_property(P::SHED_LEVELS, None, unsigned(&[2, 5]), None)
        .unwrap();
    write_start(&mut lc, at(0));
    assert_eq!(present_value(&lc), COMPLIANT);
    power_off(lc);
    app.take();
    clock.set(at(10));
    let mut lc = kept(&store, &clock, &app);
    lc.start_output();
    assert_eq!(present_value(&lc), COMPLIANT);
    assert!(
        matches!(&app.take()[..], [Told::Shed(r)] if r.started && r.shed_level == Some(0)),
        "the request is decided against the written levels"
    );
}

#[test]
fn the_running_servers_route_drops_the_written_levels() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    lc.write_property(P::SHED_LEVELS, Some(1), PropertyValue::Unsigned(4), None)
        .unwrap();
    // A refused setting changes nothing, the written level included.
    let refused = LoadControlSetting::ShedLevels(vec![(0, "x".into())]);
    assert!(
        lc.apply_local_input(LocalInputValue::LoadControl(refused))
            .is_err()
    );
    lc.wait_for_saves();
    assert_eq!(store.held().shed_levels, [(1, 4)]);
    let setting = LoadControlSetting::ShedLevels(vec![(6, "six".into()), (7, "seven".into())]);
    lc.apply_local_input(LocalInputValue::LoadControl(setting))
        .unwrap();
    assert_eq!(levels(&lc), unsigned(&[6, 7]));
    lc.wait_for_saves();
    assert!(store.held().shed_levels.is_empty());
    power_off(lc);
    let lc = kept(&store, &clock, &app);
    assert_eq!(levels(&lc), unsigned(&[1, 2]));
}

#[test]
fn written_levels_can_be_cleared_from_storage() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    lc.write_property(P::SHED_LEVELS, None, unsigned(&[3, 7]), None)
        .unwrap();
    power_off(lc);
    let mut lc = LoadControlObject::with_persistence(1, "LC-1", store.clone()).unwrap();
    // A save that fails keeps them.
    store.fail.store(true, Ordering::SeqCst);
    assert!(lc.clear_written_shed_levels().is_err());
    store.fail.store(false, Ordering::SeqCst);
    assert_eq!(store.held().shed_levels, [(1, 3), (2, 7)]);
    lc.clear_written_shed_levels().unwrap();
    assert!(store.held().shed_levels.is_empty());
    // Nothing left to clear saves nothing; the levels given next apply.
    let before = saves(&store);
    lc.clear_written_shed_levels().unwrap();
    assert_eq!(saves(&store), before);
    lc.set_shed_levels(vec![(1, "one".into()), (2, "two".into())])
        .unwrap();
    assert_eq!(levels(&lc), unsigned(&[1, 2]));
    // The request is left alone.
    power_off(lc);
    assert_eq!(store.held().requested_shed_level, BACnetShedLevel::Level(2));
    let lc = kept(&store, &clock, &app);
    assert_eq!(levels(&lc), unsigned(&[1, 2]));
}

#[test]
fn a_levels_write_that_cannot_be_saved_or_is_refused_changes_nothing() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    lc.wait_for_saves();
    store.fail.store(true, Ordering::SeqCst);
    let error = lc
        .write_property(P::SHED_LEVELS, Some(2), PropertyValue::Unsigned(9), None)
        .unwrap_err();
    assert_eq!(error_code(error), ErrorCode::OPERATIONAL_PROBLEM);
    assert_eq!(levels(&lc), unsigned(&[1, 2]));
    store.fail.store(false, Ordering::SeqCst);
    // The object's own refusals come first, with nothing saved.
    let before = saves(&store);
    for (index, value, code) in [
        (
            Some(0),
            PropertyValue::Unsigned(3),
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
        (
            Some(3),
            PropertyValue::Unsigned(3),
            ErrorCode::INVALID_ARRAY_INDEX,
        ),
        (
            Some(1),
            PropertyValue::Unsigned(0),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (None, unsigned(&[3]), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            Some(1),
            PropertyValue::Real(3.0),
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        let error = lc
            .write_property(P::SHED_LEVELS, index, value, None)
            .unwrap_err();
        assert_eq!(error_code(error), code, "{index:?}");
    }
    lc.wait_for_saves();
    assert_eq!(saves(&store), before);
    assert!(store.held().shed_levels.is_empty());
    assert_eq!(levels(&lc), unsigned(&[1, 2]));
}

#[test]
fn a_staged_levels_write_saves_before_it_is_served_and_a_refused_one_stages_nothing() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    lc.wait_for_saves();
    let before = saves(&store);
    let write = |array_index, value| PendingWrite {
        property: P::SHED_LEVELS,
        array_index,
        value,
    };
    // Two elements in one request: one save holds both.
    let writes = [
        write(Some(1), PropertyValue::Unsigned(4)),
        write(Some(2), PropertyValue::Unsigned(8)),
    ];
    let StageStep::Staged(wait) = lc.stage_writes(&writes) else {
        panic!("a Shed_Levels write stages");
    };
    wait.block();
    assert_eq!(store.held().shed_levels, [(1, 4), (2, 8)]);
    assert_eq!(levels(&lc), unsigned(&[1, 2]));
    for pending in &writes {
        lc.write_property(
            P::SHED_LEVELS,
            pending.array_index,
            pending.value.clone(),
            None,
        )
        .unwrap();
    }
    lc.release_staged_write(&wait);
    lc.wait_for_saves();
    assert_eq!(levels(&lc), unsigned(&[4, 8]));
    assert_eq!(saves(&store), before + 1, "the writes took the staged save");
    // A write the object refuses stages nothing, so nothing is saved.
    for refused in [
        write(Some(3), PropertyValue::Unsigned(4)),
        write(Some(1), PropertyValue::Unsigned(0)),
        write(None, unsigned(&[1, 2, 3])),
    ] {
        assert!(matches!(lc.stage_writes(&[refused]), StageStep::Skip));
    }
    lc.wait_for_saves();
    assert_eq!(saves(&store), before + 1);
    assert_eq!(store.held().shed_levels, [(1, 4), (2, 8)]);
}

#[test]
fn more_written_levels_than_storage_keeps_are_refused() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    let size = MAX_WRITTEN_SHED_LEVELS as u64 + 1;
    lc.set_shed_levels((1..=size).map(|level| (level, String::new())).collect())
        .unwrap();
    let all = PropertyValue::List((1..=size).map(PropertyValue::Unsigned).collect());
    let error = lc
        .write_property(P::SHED_LEVELS, None, all.clone(), None)
        .unwrap_err();
    assert_eq!(error_code(error), ErrorCode::NO_SPACE_TO_WRITE_PROPERTY);
    // One element fits; an object without storage takes the whole array.
    lc.write_property(P::SHED_LEVELS, Some(1), PropertyValue::Unsigned(5), None)
        .unwrap();
    lc.wait_for_saves();
    assert_eq!(store.held().shed_levels, [(1, 5)]);
    let mut plain = LoadControlObject::new(2, "LC-2").unwrap();
    plain
        .set_shed_levels((1..=size).map(|level| (level, String::new())).collect())
        .unwrap();
    plain
        .write_property(P::SHED_LEVELS, None, all, None)
        .unwrap();
}

#[test]
fn saved_levels_no_write_leaves_fail_the_build() {
    let store = Arc::new(Store::default());
    for shed_levels in [vec![(0, 1)], vec![(1, 0)], vec![(2, 1), (1, 1)]] {
        *store.saved.lock().unwrap() = Some(LoadControlSnapshot {
            shed_levels: shed_levels.clone(),
            ..LoadControlSnapshot::default()
        });
        assert!(
            LoadControlObject::with_persistence(1, "LC-1", store.clone()).is_err(),
            "{shed_levels:?}"
        );
    }
}
