//! What a Load Control's staged steps hold (#2526): nothing but their
//! writes. The stage works the request out on one working copy and saves
//! it once; each write is made on the object's own state as it is taken.

use super::*;
use crate::load_control::LoadControlSetting;
use crate::traits::{LocalInput, LocalInputValue};

fn write(property: P, index: Option<u32>, value: PropertyValue) -> PendingWrite {
    PendingWrite {
        property,
        array_index: index,
        value,
    }
}

fn read(lc: &LoadControlObject, property: P) -> PropertyValue {
    lc.read_property(property, None).unwrap()
}

fn unsigned(values: &[u64]) -> PropertyValue {
    PropertyValue::List(values.iter().map(|&v| PropertyValue::Unsigned(v)).collect())
}

fn error_code(error: Error) -> ErrorCode {
    match error {
        Error::Remote(RemoteError::Error { code, .. }) => code,
        other => panic!("not a BACnet error: {other:?}"),
    }
}

/// Every claim, the later ones included, serves its request's prefix and
/// takes its step, and storage ends where the stage saved it, once.
#[test]
fn each_claim_makes_its_write_on_the_object_served() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    lc.wait_for_saves();
    let saves = store.saves.load(Ordering::SeqCst);
    let writes = [
        write(P::DUTY_WINDOW, None, PropertyValue::Unsigned(5)),
        write(P::SHED_LEVELS, Some(1), PropertyValue::Unsigned(7)),
        write(P::DUTY_WINDOW, None, PropertyValue::Unsigned(6)),
        write(P::SHED_LEVELS, Some(2), PropertyValue::Unsigned(8)),
    ];
    let served = [(5, [1, 2]), (5, [7, 2]), (6, [7, 2]), (6, [7, 8])];
    let StageStep::Staged(wait) = lc.stage_writes(&writes) else {
        panic!("the writes stage");
    };
    wait.block();
    let staged = store.held();
    assert_eq!(
        (staged.duty_window, staged.shed_levels.as_slice()),
        (6, [(1, 7), (2, 8)].as_slice())
    );
    for (at, (write, (duty, levels))) in writes.iter().zip(served).enumerate() {
        let PendingWrite {
            property,
            array_index,
            value,
        } = write.clone();
        lc.write_property(property, array_index, value, None)
            .unwrap();
        assert_eq!(
            read(&lc, P::DUTY_WINDOW),
            PropertyValue::Unsigned(duty),
            "write {at}"
        );
        assert_eq!(read(&lc, P::SHED_LEVELS), unsigned(&levels), "write {at}");
        assert_eq!(lc.has_staged_write(), at + 1 < writes.len(), "write {at}");
    }
    lc.release_staged_write(&wait);
    lc.wait_for_saves();
    assert_eq!(store.held(), staged);
    assert_eq!(store.saves.load(Ordering::SeqCst), saves + 1);
}

/// An object dropped part way through a request puts storage back to what
/// it served then: the writes it made, not the request before nor the one
/// the stage saved (#1363).
#[test]
fn a_drop_part_way_puts_storage_back_to_the_writes_made() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    lc.wait_for_saves();
    let writes = [
        write(P::DUTY_WINDOW, None, PropertyValue::Unsigned(5)),
        write(P::DUTY_WINDOW, None, PropertyValue::Unsigned(6)),
    ];
    let StageStep::Staged(wait) = lc.stage_writes(&writes) else {
        panic!("the writes stage");
    };
    wait.block();
    assert_eq!(store.held().duty_window, 6);
    lc.write_property(P::DUTY_WINDOW, None, PropertyValue::Unsigned(5), None)
        .unwrap();
    drop(lc);
    assert_eq!(store.held().duty_window, 5);
}

/// A Start_Time staged after Enable FALSE in the same request is checked
/// against the disabled object the earlier write leaves: taken but not
/// kept, so the stage saves no request.
#[test]
fn a_start_time_after_a_staged_enable_false_starts_nothing() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    lc.wait_for_saves();
    let start = PropertyValue::List(vec![
        PropertyValue::Date(at(5).0),
        PropertyValue::Time(at(5).1),
    ]);
    let staged = stage_and_write(
        &mut lc,
        &store,
        &[
            (P::ENABLE, PropertyValue::Boolean(false)),
            (P::START_TIME, start),
        ],
    );
    assert_eq!(staged.state, INACTIVE);
    assert_eq!(staged.start_time, UNSPECIFIED_DATETIME);
    assert!(!staged.enable);
    lc.wait_for_saves();
    assert_eq!(store.held(), staged);
    assert_eq!(present_value(&lc), INACTIVE);
}

/// A staged Shed_Levels element the application's smaller array leaves no
/// place for takes no step: it is refused as one nobody staged, the staged
/// write stays for its request to release, and storage goes back to what
/// the object serves.
#[test]
fn a_staged_element_the_application_shrank_away_takes_no_step() {
    let (store, clock) = (Arc::new(Store::default()), test_clock());
    let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
    let mut lc = kept(&store, &clock, &app);
    lc.wait_for_saves();
    let value = PropertyValue::Unsigned(9);
    let StageStep::Staged(wait) = lc.stage_writes(&[write(P::SHED_LEVELS, Some(2), value.clone())])
    else {
        panic!("a Shed_Levels write stages");
    };
    wait.block();
    let setting = LoadControlSetting::ShedLevels(vec![(5, "five".into())]);
    lc.apply_local_input(LocalInputValue::LoadControl(setting))
        .unwrap();
    let error = lc
        .write_property(P::SHED_LEVELS, Some(2), value, None)
        .unwrap_err();
    assert_eq!(error_code(error), ErrorCode::INVALID_ARRAY_INDEX);
    assert!(lc.has_staged_write(), "the refused write took no step");
    lc.release_staged_write(&wait);
    lc.wait_for_saves();
    assert!(store.held().shed_levels.is_empty(), "storage was put back");
    assert_eq!(read(&lc, P::SHED_LEVELS), unsigned(&[5]));
}

/// The same refusal with the object dropped before its request lets the
/// stage go, as when the server is stopped part way: an element past the
/// shorter array, or a whole array of the old size, took no step, so the
/// drop puts storage back to the levels the object serves and leaves none
/// of the refused ones there.
#[test]
fn a_shed_levels_write_the_shorter_array_refuses_leaves_storage_on_a_drop() {
    let old = unsigned(&[7, 8, 9]);
    for (index, value, code) in [
        (
            Some(3),
            PropertyValue::Unsigned(9),
            ErrorCode::INVALID_ARRAY_INDEX,
        ),
        (None, old, ErrorCode::VALUE_OUT_OF_RANGE),
    ] {
        let (store, clock) = (Arc::new(Store::default()), test_clock());
        let app = TestShedder::new(ShedProgress::Compliant, 2, 2);
        let mut lc = kept(&store, &clock, &app);
        let three = (1..=3).map(|level| (level, String::new())).collect();
        lc.set_shed_levels(three).unwrap();
        lc.wait_for_saves();
        let staged = write(P::SHED_LEVELS, index, value.clone());
        let StageStep::Staged(wait) = lc.stage_writes(&[staged]) else {
            panic!("a Shed_Levels write stages");
        };
        wait.block();
        assert!(!store.held().shed_levels.is_empty(), "{index:?}");
        let two = (1..=2).map(|level| (level, String::new())).collect();
        lc.set_shed_levels(two).unwrap();
        let error = lc
            .write_property(P::SHED_LEVELS, index, value, None)
            .unwrap_err();
        assert_eq!(error_code(error), code, "{index:?}");
        drop(wait);
        drop(lc);
        assert!(
            store.held().shed_levels.is_empty(),
            "{index:?}: storage was put back"
        );
    }
}
