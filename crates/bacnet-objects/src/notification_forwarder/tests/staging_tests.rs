//! Saves that run off the database lock (#1270): staged writes, and the
//! operation task's coalesced saves.

use super::storage::{MemoryPersistence, WAIT, block_on, persistent};
use super::*;
use crate::durable::{DurableWrites, STAGED_WRITE_LIFETIME, SaveWait, StageStep};
use bacnet_types::enums::PropertyIdentifier as P;
use std::sync::atomic::Ordering;
use std::time::Instant;

fn staged(step: StageStep) -> SaveWait {
    match step {
        StageStep::Staged(wait) => wait,
        other => panic!("expected a staged write, got {other:?}"),
    }
}

fn busy(step: StageStep) -> SaveWait {
    match step {
        StageStep::Busy(wait) => wait,
        other => panic!("expected a busy forwarder, got {other:?}"),
    }
}

fn write(
    nf: &mut NotificationForwarderObject,
    list: &[BACnetEventNotificationSubscription],
) -> Result<(), Error> {
    nf.write_property(
        P::SUBSCRIBED_RECIPIENTS,
        None,
        framed_subscriptions(list),
        None,
    )
}

/// Stage a Subscribed_Recipients write of `list`, wait for its save, and
/// return the wait to release it with.
fn stage_saved(
    nf: &mut NotificationForwarderObject,
    list: &[BACnetEventNotificationSubscription],
) -> SaveWait {
    let wait = staged(nf.stage_write(P::SUBSCRIBED_RECIPIENTS, None, &framed_subscriptions(list)));
    block_on(wait.clone());
    wait
}

#[test]
fn a_staged_write_saves_while_the_forwarder_serves_the_old_list() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let old = [subscription(device(7), 1, 10)];
    write(&mut nf, &old).unwrap();
    let held = storage.hold();
    let new = [subscription(device(8), 2, 20)];
    let wait = staged(nf.stage_write(P::SUBSCRIBED_RECIPIENTS, None, &framed_subscriptions(&new)));

    // The save runs on the writer thread and is held there. The forwarder,
    // and so the database guard that would hold it, is free meanwhile: it
    // answers reads, and serves the old list.
    let saving = held.started.recv_timeout(WAIT).unwrap();
    assert_eq!(saving.subscribed_recipients, new);
    assert!(!wait.is_ready());
    assert_eq!(
        nf.read_property(P::SUBSCRIBED_RECIPIENTS, None).unwrap(),
        framed_subscriptions(&old)
    );
    held.go.send(()).unwrap();
    block_on(wait.clone());

    // The write takes the saved list without saving again.
    write(&mut nf, &new).unwrap();
    nf.release_staged_write(&wait);
    nf.wait_for_saves();
    assert_eq!(nf.subscriptions(), new);
    assert_eq!(storage.saved(), new);
    assert_eq!(storage.saves(), 2);
}

#[test]
fn a_staged_write_whose_save_fails_is_refused_and_the_old_list_stays() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let counters = nf.save_counters();
    let old = [subscription(device(7), 1, 10)];
    write(&mut nf, &old).unwrap();
    storage.fail.store(true, Ordering::SeqCst);
    let new = [subscription(device(8), 2, 20)];
    let wait = stage_saved(&mut nf, &new);
    assert_refused(
        write(&mut nf, &new),
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM,
    );
    nf.release_staged_write(&wait);
    assert_eq!(nf.subscriptions(), old);
    assert_eq!(storage.saved(), old);
    assert_eq!(counters.failed_saves(), 1);
}

#[test]
fn a_second_write_waits_until_the_first_has_landed() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let first = [subscription(device(7), 1, 10)];
    let second = [subscription(device(8), 1, 10)];
    // The first save is held while the second request stages, so the second
    // finds the forwarder busy however slowly the test runs.
    let held = storage.hold();
    let wait = staged(nf.stage_write(
        P::SUBSCRIBED_RECIPIENTS,
        None,
        &framed_subscriptions(&first),
    ));
    held.started.recv_timeout(WAIT).unwrap();
    let queued = busy(nf.stage_write(
        P::RECIPIENT_LIST,
        None,
        &framed_destinations(&[destination(device(3), 1, false)]),
    ));
    drop(held.go);
    block_on(wait.clone());
    // Saved, but not yet taken: the second still waits.
    assert!(!queued.is_ready());
    write(&mut nf, &first).unwrap();
    nf.release_staged_write(&wait);
    block_on(queued);
    let wait = stage_saved(&mut nf, &second);
    write(&mut nf, &second).unwrap();
    nf.release_staged_write(&wait);
    nf.wait_for_saves();
    assert_eq!(nf.subscriptions(), second);
    assert_eq!(storage.saved(), second);
}

#[test]
fn a_staged_write_its_request_never_made_leaves_storage_with_the_served_list() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let served = [subscription(device(7), 1, 10)];
    write(&mut nf, &served).unwrap();
    let never = [subscription(device(9), 1, 10)];
    let wait = stage_saved(&mut nf, &never);
    assert_eq!(storage.saved(), never);
    // The request failed before its write. Releasing it saves the served
    // list at once, with no operation task call, so a restart right after
    // serves what the forwarder served.
    nf.release_staged_write(&wait);
    nf.wait_for_saves();
    assert_eq!(storage.saved(), served);
    assert_eq!(nf.subscriptions(), served);
    drop(nf);
    assert_eq!(persistent(&storage).subscriptions(), served);
}

#[test]
fn a_released_staged_write_whose_save_failed_saves_nothing_more() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let served = [subscription(device(7), 1, 10)];
    write(&mut nf, &served).unwrap();
    storage.fail.store(true, Ordering::SeqCst);
    let wait = stage_saved(&mut nf, &[subscription(device(9), 1, 10)]);
    storage.fail.store(false, Ordering::SeqCst);
    // Storage still holds the served list, so there is nothing to put back.
    nf.release_staged_write(&wait);
    nf.wait_for_saves();
    assert_eq!(storage.saves(), 1);
    assert_eq!(storage.saved(), served);
}

#[test]
fn a_release_from_another_request_leaves_the_staged_write_alone() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let first = [subscription(device(7), 1, 10)];
    let second = [subscription(device(8), 1, 10)];
    let earlier = stage_saved(&mut nf, &first);
    write(&mut nf, &first).unwrap();
    nf.release_staged_write(&earlier);
    // A second request stages; its save is held, so it holds the forwarder.
    let held = storage.hold();
    let wait = staged(nf.stage_write(
        P::SUBSCRIBED_RECIPIENTS,
        None,
        &framed_subscriptions(&second),
    ));
    held.started.recv_timeout(WAIT).unwrap();
    // The first request releasing again, late, does not drop it.
    nf.release_staged_write(&earlier);
    let queued = busy(nf.stage_write(P::RECIPIENT_LIST, None, &framed_destinations(&[])));
    assert!(!queued.is_ready());
    drop(held.go);
    block_on(wait.clone());
    write(&mut nf, &second).unwrap();
    nf.release_staged_write(&wait);
    nf.wait_for_saves();
    // Each write took its own staged save: no save in place, no correction.
    assert_eq!(storage.saves(), 2);
    assert_eq!(storage.saved(), second);
}

#[test]
fn a_slow_save_keeps_its_staged_write_for_a_lifetime_after_it_finishes() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let list = [subscription(device(7), 1, 10)];
    let held = storage.hold();
    let staged_at = Instant::now();
    let wait = staged(nf.stage_write(P::SUBSCRIBED_RECIPIENTS, None, &framed_subscriptions(&list)));
    held.started.recv_timeout(WAIT).unwrap();
    // The save takes longer than a staged write's whole lifetime.
    std::thread::sleep(STAGED_WRITE_LIFETIME + Duration::from_millis(100));
    held.go.send(()).unwrap();
    block_on(wait.clone());
    // A lifetime has passed since the write was staged, but the save has
    // only just finished, so the staged write still holds the forwarder.
    let past_staging = staged_at + STAGED_WRITE_LIFETIME + Duration::from_millis(50);
    let storage_now = nf.storage.as_mut().unwrap();
    assert!(storage_now.busy_at(past_staging).is_some());
    // The write takes the staged save; it is not saved again in place.
    write(&mut nf, &list).unwrap();
    nf.release_staged_write(&wait);
    nf.wait_for_saves();
    assert_eq!(nf.subscriptions(), list);
    assert_eq!(storage.saves(), 1);
}

#[test]
fn a_write_nobody_staged_supersedes_a_staged_one() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let staged_list = [subscription(device(7), 1, 10)];
    let direct = [subscription(device(8), 1, 10)];
    let wait = stage_saved(&mut nf, &staged_list);
    // A write made without staging, by application code holding the guard,
    // saves in place and drops the staged list.
    write(&mut nf, &direct).unwrap();
    assert_eq!(storage.saved(), direct);
    // The staged write's request arrives late: it saves again, and wins.
    write(&mut nf, &staged_list).unwrap();
    nf.release_staged_write(&wait);
    nf.wait_for_saves();
    assert_eq!(nf.subscriptions(), staged_list);
    assert_eq!(storage.saved(), staged_list);
}

#[test]
fn a_forgotten_staged_write_frees_the_forwarder() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let list = [subscription(device(7), 1, 10)];
    let _forgotten = stage_saved(&mut nf, &list);
    let waiting =
        busy(nf.stage_write(P::SUBSCRIBED_RECIPIENTS, None, &framed_subscriptions(&list)));
    // Its request never comes back. Once the staged write's lifetime is
    // over, the operation task drops it and the waiting request goes on.
    std::thread::sleep(STAGED_WRITE_LIFETIME + Duration::from_millis(100));
    nf.advance_monotonic_time_internal(Duration::ZERO);
    block_on(waiting);
    let wait = stage_saved(&mut nf, &list);
    write(&mut nf, &list).unwrap();
    nf.release_staged_write(&wait);
    assert_eq!(nf.subscriptions(), list);
}

#[test]
fn an_operation_task_call_drops_a_forgotten_staged_write_on_the_store_clock() {
    let storage = Arc::new(MemoryPersistence::default());
    let (clock, set) = manual_clock();
    let mut nf = persistent(&storage);
    nf.bind_monotonic_clock_internal(Some(clock));
    let served = [subscription(device(7), 1, 10)];
    write(&mut nf, &served).unwrap();
    let _forgotten = stage_saved(&mut nf, &[subscription(device(9), 1, 10)]);
    // Its request never comes back. The first call that finds the save
    // finished starts the count on the store's clock, and the call a
    // lifetime later drops it and saves the served lists.
    nf.advance_monotonic_time_internal(Duration::ZERO);
    set(STAGED_WRITE_LIFETIME);
    nf.advance_monotonic_time_internal(STAGED_WRITE_LIFETIME);
    nf.wait_for_saves();
    assert_eq!(storage.saved(), served);
    assert_eq!(nf.subscriptions(), served);
}

#[test]
fn staging_skips_writes_the_forwarder_does_not_save_or_will_refuse() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let list = framed_subscriptions(&[subscription(device(7), 1, 10)]);
    let skipped = |step| matches!(step, StageStep::Skip);
    assert!(skipped(nf.stage_write(
        P::PROCESS_IDENTIFIER_FILTER,
        None,
        &PropertyValue::Null
    )));
    assert!(skipped(nf.stage_write(
        P::SUBSCRIBED_RECIPIENTS,
        Some(1),
        &list
    )));
    // A Time Remaining of 0 is refused by the write itself.
    assert!(skipped(nf.stage_write(
        P::SUBSCRIBED_RECIPIENTS,
        None,
        &framed_subscriptions(&[subscription(device(7), 1, 0)]),
    )));
    let mut in_memory = NotificationForwarderObject::new(2, "NF-2").unwrap();
    assert!(skipped(in_memory.stage_write(
        P::SUBSCRIBED_RECIPIENTS,
        None,
        &list
    )));
    nf.wait_for_saves();
    assert_eq!(storage.saves(), 0);
}

#[test]
fn operation_task_saves_coalesce_and_never_hold_the_forwarder() {
    let storage = Arc::new(MemoryPersistence::default());
    let (clock, set) = manual_clock();
    let mut nf = persistent(&storage);
    nf.bind_monotonic_clock_internal(Some(clock));
    let entries: Vec<_> = (1..=6)
        .map(|minutes| subscription(device(minutes), 1, minutes))
        .collect();
    write(&mut nf, &entries).unwrap();
    let held = storage.hold();
    // The entry with one minute lapses: its save starts and is held.
    set(MINUTE);
    assert!(nf.advance_monotonic_time_internal(MINUTE));
    held.started.recv_timeout(WAIT).unwrap();
    // While it is held, four more entries lapse, one a minute. Each call
    // returns at once, and each queued save replaces the one before.
    for minute in 2..=5 {
        let at = MINUTE * minute;
        set(at);
        assert!(nf.advance_monotonic_time_internal(at));
        assert_eq!(nf.subscriptions().len(), 6 - minute as usize);
    }
    drop(held.go);
    nf.wait_for_saves();
    // The write, the held save, and one save of the latest list.
    assert_eq!(storage.saves(), 3);
    assert_eq!(storage.saved(), [subscription(device(6), 1, 1)]);
}

#[test]
fn an_operation_task_save_returns_while_storage_is_held() {
    let storage = Arc::new(MemoryPersistence::default());
    let (clock, set) = manual_clock();
    let mut nf = persistent(&storage);
    nf.bind_monotonic_clock_internal(Some(clock));
    write(
        &mut nf,
        &[subscription(device(1), 1, 1), subscription(device(2), 1, 6)],
    )
    .unwrap();
    let held = storage.hold();
    set(MINUTE);
    // The lapse queues a save; the call returns while storage still holds
    // it, so the operation task never keeps the database guard for a save.
    // Storage stays held until the call returns or the long wait runs out.
    let (returned_tx, returned) = std::sync::mpsc::channel();
    let returned_in_time = std::thread::scope(|scope| {
        scope.spawn(|| {
            nf.advance_monotonic_time_internal(MINUTE);
            returned_tx.send(()).unwrap();
        });
        held.started
            .recv_timeout(WAIT)
            .expect("the lapse save started");
        let in_time = returned.recv_timeout(WAIT).is_ok();
        drop(held.go);
        in_time
    });
    assert!(
        returned_in_time,
        "the operation task call waited for the save"
    );
    nf.wait_for_saves();
    assert_eq!(storage.saved(), [subscription(device(2), 1, 5)]);
}

// A forwarder can go while a write is still staged for a request that never
// came back, as when the server stops mid-request and the database is
// dropped (#1363). Storage then goes back to the lists it served.

#[test]
fn a_forwarder_dropped_with_a_staged_write_puts_storage_back_to_the_served_lists() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let served = [subscription(device(7), 1, 10)];
    write(&mut nf, &served).unwrap();
    let _forgotten = stage_saved(&mut nf, &[subscription(device(9), 1, 10)]);
    assert_eq!(storage.saved(), [subscription(device(9), 1, 10)]);
    // The forwarder goes before any lifetime check could drop the write.
    drop(nf);
    assert_eq!(storage.saved(), served);
    // The first write, the staged one and the put-back.
    assert_eq!(storage.saves(), 3);
    assert_eq!(persistent(&storage).subscriptions(), served);
}

#[test]
fn a_forwarder_dropped_with_a_staged_recipient_list_keeps_none_written() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let list = [destination(device(9), 1, false)];
    let wait = staged(nf.stage_write(P::RECIPIENT_LIST, None, &framed_destinations(&list)));
    block_on(wait);
    assert_eq!(storage.snapshot().recipient_list, Some(list.to_vec()));
    drop(nf);
    // No write set Recipient_List, so the configured one applies again.
    assert_eq!(storage.snapshot().recipient_list, None);
    assert!(!persistent(&storage).recipient_list_saved());
}

#[test]
fn a_forwarder_dropped_after_its_staged_write_was_made_keeps_the_new_lists() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    write(&mut nf, &[subscription(device(7), 1, 10)]).unwrap();
    let new = [subscription(device(8), 1, 10)];
    let wait = stage_saved(&mut nf, &new);
    write(&mut nf, &new).unwrap();
    nf.release_staged_write(&wait);
    drop(nf);
    assert_eq!(storage.saved(), new);
    assert_eq!(storage.saves(), 2);
    assert_eq!(persistent(&storage).subscriptions(), new);
}

#[test]
fn a_forwarder_dropped_with_a_staged_write_whose_save_failed_leaves_storage_alone() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let served = [subscription(device(7), 1, 10)];
    write(&mut nf, &served).unwrap();
    storage.fail.store(true, Ordering::SeqCst);
    let _failed = stage_saved(&mut nf, &[subscription(device(9), 1, 10)]);
    // Storage works again, so a save at the drop would land and count.
    storage.fail.store(false, Ordering::SeqCst);
    drop(nf);
    assert_eq!(storage.saved(), served);
    assert_eq!(storage.saves(), 1);
}

#[test]
fn a_forwarder_dropped_while_its_staged_save_runs_puts_storage_back_after_it() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let served = [subscription(device(7), 1, 10)];
    write(&mut nf, &served).unwrap();
    let held = storage.hold();
    let list = framed_subscriptions(&[subscription(device(9), 1, 10)]);
    let _forgotten = staged(nf.stage_write(P::SUBSCRIBED_RECIPIENTS, None, &list));
    held.started.recv_timeout(WAIT).unwrap();
    // The drop waits for the saves it queues, so it runs on a thread of its
    // own while the staged save is held.
    let dropping = std::thread::spawn(move || drop(nf));
    drop(held.go);
    dropping.join().unwrap();
    // The staged save landed first, then the put-back.
    assert_eq!(storage.saved(), served);
    assert_eq!(storage.saves(), 3);
}

#[test]
fn a_forwarder_dropped_with_a_staged_write_puts_back_the_lists_it_serves_as_it_goes() {
    let storage = Arc::new(MemoryPersistence::default());
    let (clock, set) = manual_clock();
    let mut nf = persistent(&storage);
    nf.bind_monotonic_clock_internal(Some(clock));
    write(
        &mut nf,
        &[subscription(device(1), 1, 1), subscription(device(2), 1, 6)],
    )
    .unwrap();
    // A Recipient_List write stages, and its save of both lists as they
    // stand lands.
    let list = framed_destinations(&[destination(device(9), 1, false)]);
    let wait = staged(nf.stage_write(P::RECIPIENT_LIST, None, &list));
    block_on(wait);
    // While it waits for its request, the one-minute entry lapses and the
    // other loses a minute. Nothing saves that while the write is staged.
    set(MINUTE);
    assert!(nf.advance_monotonic_time_internal(MINUTE));
    drop(nf);
    // The put-back holds the lists the forwarder served when it went, not
    // those it served when the write staged.
    assert_eq!(
        storage.snapshot(),
        ForwarderSnapshot {
            recipient_list: None,
            subscribed_recipients: vec![subscription(device(2), 1, 5)],
        }
    );
}

#[test]
fn settling_forgotten_writes_puts_storage_back_and_frees_the_forwarder() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let served = [subscription(device(7), 1, 10)];
    write(&mut nf, &served).unwrap();
    let _forgotten = stage_saved(&mut nf, &[subscription(device(9), 1, 10)]);
    // What the server's stop() does once it has joined every request.
    let settled = nf.settle_forgotten_writes().expect("the forwarder saves");
    block_on(settled);
    assert_eq!(storage.saved(), served);
    assert_eq!(nf.subscriptions(), served);
    // The forwarder is free: the next write stages at once.
    let list = [subscription(device(8), 1, 10)];
    let wait = stage_saved(&mut nf, &list);
    write(&mut nf, &list).unwrap();
    nf.release_staged_write(&wait);
    nf.wait_for_saves();
    assert_eq!(storage.saved(), list);
    assert!(nf.settle_forgotten_writes().unwrap().is_ready());
    assert!(
        NotificationForwarderObject::new(2, "NF-2")
            .unwrap()
            .settle_forgotten_writes()
            .is_none()
    );
}
