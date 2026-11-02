//! `StagedSaves` over the build's own save writer: the inline one without
//! std, where each save runs before the call that queued it returns, and the
//! writer thread with std (#2048). Each test waits for the writer before it
//! looks at storage, so it holds for both.

use super::*;
use bacnet_types::enums::ErrorClass;
use bacnet_types::enums::ErrorCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

const P: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;

/// Storage that logs each saved state and refuses saves while `fail` is set.
#[derive(Default)]
struct Store {
    saved: Mutex<Vec<u32>>,
    fail: AtomicBool,
}

impl Store {
    fn saved(&self) -> Vec<u32> {
        self.saved.lock().unwrap().clone()
    }
}

fn staged_saves(store: &Arc<Store>) -> StagedSaves<u32, u32> {
    let store = Arc::clone(store);
    StagedSaves::new(SaveWriter::new(
        "staged-test".into(),
        move |state: &u32| {
            if store.fail.load(Ordering::SeqCst) {
                return Err(Error::Encoding("storage refused".into()));
            }
            store.saved.lock().unwrap().push(*state);
            Ok(())
        },
        |_, _| {},
    ))
}

fn write(value: PropertyValue) -> PendingWrite {
    PendingWrite {
        property: P,
        array_index: None,
        value,
    }
}

fn unsigned(value: u32) -> PendingWrite {
    write(PropertyValue::Unsigned(value.into()))
}

/// The steps of `writes`: an Unsigned leaves its value, zero saves nothing,
/// and anything else is refused as the wrong datatype.
fn steps_of(writes: &[PendingWrite]) -> Vec<Step<u32>> {
    steps(writes, |_, write| match write.value {
        PropertyValue::Unsigned(0) => Ok(None),
        PropertyValue::Unsigned(value) => Ok(Some(value as u32)),
        _ => Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        )),
    })
}

/// Stage `writes` from write count 0 over the served state `served`.
fn stage(saves: &mut StagedSaves<u32, u32>, writes: &[PendingWrite], served: u32) -> SaveWait {
    match saves.stage(steps_of(writes), 0, served, |state, next| *state = *next) {
        StageStep::Staged(wait) => wait,
        other => panic!("expected a staged save, got {other:?}"),
    }
}

fn claim(saves: &mut StagedSaves<u32, u32>, value: u32, base: u64) -> Option<Result<u32, Error>> {
    saves.claim(P, None, &PropertyValue::Unsigned(value.into()), base)
}

#[test]
fn steps_stop_at_a_refused_write_and_pass_a_refused_null() {
    let writes = [
        unsigned(5),
        unsigned(0),
        write(PropertyValue::Null),
        unsigned(6),
        write(PropertyValue::Boolean(true)),
        unsigned(7),
    ];
    let steps = steps_of(&writes);
    assert_eq!(
        steps.iter().map(|step| step.next).collect::<Vec<_>>(),
        [5, 6]
    );
}

#[test]
fn a_staged_write_saves_its_state_and_its_write_takes_it() {
    let store = Arc::new(Store::default());
    let mut saves = staged_saves(&store);
    let wait = stage(&mut saves, &[unsigned(5), unsigned(6)], 1);
    saves.wait_idle();
    assert!(wait.is_ready());
    // One save holds the state the last write leaves.
    assert_eq!(store.saved(), [6]);
    if cfg!(not(feature = "std")) {
        // Without a clock nothing outlives the lifetime but on the
        // operation task's count, so the staged write still holds.
        assert!(saves.busy().is_some_and(|busy| !busy.is_ready()));
    }

    // Only the next write, made at the next count, takes a step.
    assert!(claim(&mut saves, 6, 0).is_none());
    assert!(claim(&mut saves, 5, 1).is_none());
    assert_eq!(claim(&mut saves, 5, 0).unwrap().unwrap(), 5);
    assert!(saves.is_staged());
    assert!(claim(&mut saves, 6, 0).is_none());
    assert_eq!(claim(&mut saves, 6, 1).unwrap().unwrap(), 6);
    assert!(!saves.is_staged());
    assert!(saves.busy().is_none());
    // Taken, so nothing is put back.
    assert!(!saves.take_correction());
    drop(saves);
    assert_eq!(store.saved(), [6]);
}

#[test]
fn a_failed_save_refuses_the_write_and_puts_nothing_back() {
    let store = Arc::new(Store::default());
    let mut saves = staged_saves(&store);
    store.fail.store(true, Ordering::SeqCst);
    let _wait = stage(&mut saves, &[unsigned(5)], 1);
    assert!(matches!(
        claim(&mut saves, 5, 0),
        Some(Err(Error::Encoding(_)))
    ));
    assert!(!saves.is_staged());
    assert!(!saves.take_correction());
    // A save nobody staged reports its refusal too.
    assert!(saves.save_now(7).is_err());
    store.fail.store(false, Ordering::SeqCst);
    drop(saves);
    assert_eq!(store.saved(), Vec::<u32>::new());
}

#[test]
fn a_staged_write_its_request_never_makes_is_put_back() {
    let store = Arc::new(Store::default());
    let mut saves = staged_saves(&store);
    let wait = stage(&mut saves, &[unsigned(5)], 1);
    // Another request's wait leaves it alone.
    let other = saves.idle();
    saves.release(&other);
    assert!(saves.is_staged());
    saves.release(&wait);
    assert!(!saves.is_staged());
    // The served state goes back to storage after the staged one.
    saves.correct(|| 1);
    saves.wait_idle();
    assert_eq!(store.saved(), [5, 1]);
    assert!(!saves.take_correction());
}

#[test]
fn the_operation_task_drops_a_finished_staged_write_after_its_lifetime() {
    let store = Arc::new(Store::default());
    let mut saves = staged_saves(&store);
    let start = Duration::from_secs(100);
    // Nothing staged: nothing to expire.
    saves.expire(start);
    let _wait = stage(&mut saves, &[unsigned(5)], 1);
    saves.wait_idle();

    // The first call that finds the save finished starts the count.
    saves.expire(start);
    saves.expire(start + STAGED_WRITE_LIFETIME - Duration::from_nanos(1));
    assert!(saves.is_staged());
    // A clock that moved back starts the count again.
    let rebound = Duration::from_secs(50);
    saves.expire(rebound);
    saves.expire(rebound + STAGED_WRITE_LIFETIME - Duration::from_nanos(1));
    assert!(saves.is_staged());
    saves.expire(rebound + STAGED_WRITE_LIFETIME);
    assert!(!saves.is_staged());

    // The object then saves the state it serves.
    saves.correct(|| 1);
    saves.wait_idle();
    assert_eq!(store.saved(), [5, 1]);
}

#[test]
fn an_object_dropped_with_a_staged_write_puts_storage_back() {
    let store = Arc::new(Store::default());
    let mut saves = staged_saves(&store);
    let _wait = stage(&mut saves, &[unsigned(5)], 1);
    // A later storage call keeps the served state current.
    saves.correct(|| 2);
    drop(saves);
    assert_eq!(store.saved(), [5, 2]);

    // Unless the staged save failed: storage then still holds what it had.
    let store = Arc::new(Store::default());
    let mut saves = staged_saves(&store);
    store.fail.store(true, Ordering::SeqCst);
    let _wait = stage(&mut saves, &[unsigned(5)], 1);
    saves.wait_idle();
    store.fail.store(false, Ordering::SeqCst);
    drop(saves);
    assert_eq!(store.saved(), Vec::<u32>::new());
}

#[test]
fn settling_a_forgotten_write_saves_the_served_state_and_frees_the_object() {
    let store = Arc::new(Store::default());
    let mut saves = staged_saves(&store);
    let _wait = stage(&mut saves, &[unsigned(5)], 1);
    let settled = saves.drop_forgotten(|| 1);
    saves.wait_idle();
    assert!(settled.is_ready());
    assert!(!saves.is_staged());
    assert_eq!(store.saved(), [5, 1]);
}

#[test]
fn unwaited_saves_run_in_order_and_the_latest_lands_last() {
    let store = Arc::new(Store::default());
    let mut saves = staged_saves(&store);
    for state in [2, 3, 4] {
        saves.submit_coalescing(state);
    }
    saves.wait_idle();
    let saved = store.saved();
    assert_eq!(saved.last(), Some(&4));
    if cfg!(not(feature = "std")) {
        // The inline writer has nothing queued to replace: each save runs
        // as it is queued.
        assert_eq!(saved, [2, 3, 4]);
    }
    // A staged write ahead of a due correction: the correction lands first.
    let wait = stage(&mut saves, &[unsigned(5)], 1);
    saves.release(&wait);
    let staged = stage(&mut saves, &[unsigned(6)], 1);
    saves.wait_idle();
    assert!(staged.is_ready());
    assert_eq!(&store.saved()[saved.len()..], [5, 1, 6]);
}
