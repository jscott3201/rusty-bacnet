//! Recipient_List saves that run off the database lock (#1315): the
//! Notification Class stages a list write the way a Notification Forwarder
//! does (#1270).

use super::super::*;
use super::make_dest_device;
use super::persistence_tests::{assert_refused, framed, write};
use super::storage::{MemoryPersistence, WAIT, block_on, persistent};
use crate::durable::{DurableWrites, STAGED_WRITE_LIFETIME, SaveWait, StageStep};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

fn staged(step: StageStep) -> SaveWait {
    match step {
        StageStep::Staged(wait) => wait,
        other => panic!("expected a staged write, got {other:?}"),
    }
}

fn busy(step: StageStep) -> SaveWait {
    match step {
        StageStep::Busy(wait) => wait,
        other => panic!("expected a busy class, got {other:?}"),
    }
}

/// Stage a Recipient_List write of `list`, wait for its save, and return
/// the wait to release it with.
fn stage_saved(nc: &mut NotificationClass, list: &[BACnetDestination]) -> SaveWait {
    let wait = staged(nc.stage_write(P::RECIPIENT_LIST, None, &framed(list)));
    block_on(&wait);
    wait
}

#[test]
fn a_staged_write_saves_while_the_class_serves_the_old_list() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let old = [make_dest_device(7)];
    write(&mut nc, &old).unwrap();
    let held = storage.hold();
    let new = [make_dest_device(8), make_dest_device(9)];
    let wait = staged(nc.stage_write(P::RECIPIENT_LIST, None, &framed(&new)));

    // The save runs on the writer thread and is held there. The class, and
    // so the database guard that would hold it, is free meanwhile: it
    // answers reads, and serves the old list.
    let saving = held.started.recv_timeout(WAIT).unwrap();
    assert_eq!(saving.recipient_list, Some(new.to_vec()));
    assert!(!wait.is_ready());
    assert_eq!(
        nc.read_property(P::RECIPIENT_LIST, None).unwrap(),
        framed(&old)
    );
    held.go.send(()).unwrap();
    block_on(&wait);

    // The write takes the saved list without saving again.
    write(&mut nc, &new).unwrap();
    nc.release_staged_write(&wait);
    nc.wait_for_saves();
    assert_eq!(nc.recipient_list(), new);
    assert_eq!(storage.saved(), Some(new.to_vec()));
    assert_eq!(storage.saves(), 2);
}

#[test]
fn a_staged_write_whose_save_fails_is_refused_and_the_old_list_stays() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let old = [make_dest_device(7)];
    write(&mut nc, &old).unwrap();
    storage.fail.store(true, Ordering::SeqCst);
    let new = [make_dest_device(8)];
    let wait = stage_saved(&mut nc, &new);
    assert_refused(
        write(&mut nc, &new),
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM,
    );
    nc.release_staged_write(&wait);
    nc.wait_for_saves();
    assert_eq!(nc.recipient_list(), old);
    assert_eq!(storage.saved(), Some(old.to_vec()));
    assert_eq!(storage.saves(), 1);
}

#[test]
fn a_second_write_waits_until_the_first_has_landed() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let first = [make_dest_device(7)];
    let second = [make_dest_device(8)];
    // The first save is held while the second request stages, so the second
    // finds the class busy however slowly the test runs.
    let held = storage.hold();
    let wait = staged(nc.stage_write(P::RECIPIENT_LIST, None, &framed(&first)));
    held.started.recv_timeout(WAIT).unwrap();
    let queued = busy(nc.stage_write(P::RECIPIENT_LIST, None, &framed(&second)));
    drop(held.go);
    block_on(&wait);
    // Saved, but not yet taken: the second still waits.
    assert!(!queued.is_ready());
    write(&mut nc, &first).unwrap();
    nc.release_staged_write(&wait);
    block_on(&queued);
    let wait = stage_saved(&mut nc, &second);
    write(&mut nc, &second).unwrap();
    nc.release_staged_write(&wait);
    nc.wait_for_saves();
    assert_eq!(nc.recipient_list(), second);
    assert_eq!(storage.saved(), Some(second.to_vec()));
}

#[test]
fn a_staged_write_its_request_never_made_leaves_storage_with_the_served_list() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let served = [make_dest_device(7)];
    write(&mut nc, &served).unwrap();
    let wait = stage_saved(&mut nc, &[make_dest_device(9)]);
    assert_eq!(storage.saved(), Some(vec![make_dest_device(9)]));
    // The request failed before its write. Releasing it saves the served
    // list at once, so a restart right after serves what the class served.
    nc.release_staged_write(&wait);
    nc.wait_for_saves();
    assert_eq!(storage.saved(), Some(served.to_vec()));
    drop(nc);
    assert_eq!(persistent(&storage).recipient_list(), served);
}

#[test]
fn a_dropped_staged_write_leaves_configured_destinations_unsaved() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    nc.add_destination(make_dest_device(1)).unwrap();
    let wait = stage_saved(&mut nc, &[make_dest_device(9)]);
    // The staged list reached storage, but no write took it: storage goes
    // back to holding no written list, so the configuration still applies
    // at the next start.
    nc.release_staged_write(&wait);
    nc.wait_for_saves();
    assert_eq!(
        storage.snapshot(),
        Some(NotificationClassSnapshot {
            recipient_list: None
        })
    );
    drop(nc);
    let mut rebuilt = persistent(&storage);
    assert!(!rebuilt.recipient_list_saved());
    rebuilt.add_destination(make_dest_device(2)).unwrap();
    assert_eq!(rebuilt.recipient_list(), [make_dest_device(2)]);
}

#[test]
fn a_write_nobody_staged_supersedes_a_staged_one() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let staged_list = [make_dest_device(7)];
    let direct = [make_dest_device(8)];
    let wait = stage_saved(&mut nc, &staged_list);
    // A write made without staging, by application code holding the guard,
    // saves in place and drops the staged list.
    write(&mut nc, &direct).unwrap();
    assert_eq!(storage.saved(), Some(direct.to_vec()));
    // The staged write's request arrives late: it saves again, and wins.
    write(&mut nc, &staged_list).unwrap();
    nc.release_staged_write(&wait);
    nc.wait_for_saves();
    assert_eq!(nc.recipient_list(), staged_list);
    assert_eq!(storage.saved(), Some(staged_list.to_vec()));
}

#[test]
fn a_forgotten_staged_write_is_dropped_by_the_next_write_that_stages() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let served = [make_dest_device(7)];
    write(&mut nc, &served).unwrap();
    // The forgotten write's save ends after `before` and before `after`.
    let before = Instant::now();
    let _forgotten = stage_saved(&mut nc, &[make_dest_device(9)]);
    let after = Instant::now();
    // A stage judged within the lifetime finds the class busy. Its request
    // never comes back, so a stage judged once the lifetime is over drops
    // it; these are the checks the next write's stage makes, at fixed
    // instants.
    let class_storage = nc.storage.as_mut().unwrap();
    let waiting = class_storage
        .busy_at(before)
        .expect("the staged write holds the class");
    assert!(
        class_storage
            .busy_at(after + STAGED_WRITE_LIFETIME)
            .is_none()
    );
    assert!(waiting.is_ready());
    // The next write's stage puts storage back to the served list before
    // its own save.
    let list = [make_dest_device(8)];
    let wait = staged(nc.stage_write(P::RECIPIENT_LIST, None, &framed(&list)));
    block_on(&wait);
    write(&mut nc, &list).unwrap();
    nc.release_staged_write(&wait);
    nc.wait_for_saves();
    assert_eq!(nc.recipient_list(), list);
    assert_eq!(storage.saved(), Some(list.to_vec()));
    // The first write, the forgotten one, the correction and the new one.
    assert_eq!(storage.saves(), 4);
}

#[test]
fn an_operation_task_call_drops_a_forgotten_staged_write_once_its_lifetime_is_over() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let served = [make_dest_device(7)];
    write(&mut nc, &served).unwrap();
    // A save still running never expires, however late the call.
    let held = storage.hold();
    let wait = staged(nc.stage_write(P::RECIPIENT_LIST, None, &framed(&[make_dest_device(9)])));
    held.started.recv_timeout(WAIT).unwrap();
    assert!(!nc.advance_monotonic_time_internal(Duration::ZERO));
    nc.advance_monotonic_time_internal(Duration::from_secs(3600));
    drop(held.go);
    block_on(&wait);
    // Its request never comes back. The first call that finds the save
    // finished starts the count on the operation task's clock; storage keeps
    // the list nobody served until a lifetime has passed.
    let start = Duration::from_secs(3601);
    nc.advance_monotonic_time_internal(start);
    nc.advance_monotonic_time_internal(start + STAGED_WRITE_LIFETIME - Duration::from_millis(1));
    nc.wait_for_saves();
    assert_eq!(storage.saved(), Some(vec![make_dest_device(9)]));
    // The call a lifetime later drops it and puts storage back.
    nc.advance_monotonic_time_internal(start + STAGED_WRITE_LIFETIME);
    nc.wait_for_saves();
    assert_eq!(storage.saved(), Some(served.to_vec()));
    assert_eq!(nc.recipient_list(), served);
    // A release that comes after all changes nothing more.
    nc.release_staged_write(&wait);
    nc.wait_for_saves();
    assert_eq!(storage.saves(), 3);
}

#[test]
fn staging_skips_writes_the_class_does_not_save_or_will_refuse() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let list = framed(&[make_dest_device(7)]);
    let skipped = |step| matches!(step, StageStep::Skip);
    assert!(skipped(nc.stage_write(
        P::DESCRIPTION,
        None,
        &PropertyValue::CharacterString("alarms".into())
    )));
    assert!(skipped(nc.stage_write(P::RECIPIENT_LIST, Some(1), &list)));
    // A malformed list is refused by the write itself.
    assert!(skipped(nc.stage_write(
        P::RECIPIENT_LIST,
        None,
        &PropertyValue::ApplicationData(vec![0xFF]),
    )));
    let mut in_memory = NotificationClass::new(2, "NC-2").unwrap();
    assert!(skipped(in_memory.stage_write(
        P::RECIPIENT_LIST,
        None,
        &list
    )));
    nc.wait_for_saves();
    assert_eq!(storage.saves(), 0);
}

// A class can go while a write is still staged for a request that never
// came back, as when the server stops mid-request and the database is
// dropped (#1363). Storage then goes back to the list the class served.

#[test]
fn a_class_dropped_with_a_staged_write_puts_storage_back_to_the_served_list() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let served = [make_dest_device(7)];
    write(&mut nc, &served).unwrap();
    let _forgotten = stage_saved(&mut nc, &[make_dest_device(9)]);
    assert_eq!(storage.saved(), Some(vec![make_dest_device(9)]));
    // The class goes before any lifetime check could drop the staged write.
    drop(nc);
    assert_eq!(storage.saved(), Some(served.to_vec()));
    // The first write, the staged one and the put-back.
    assert_eq!(storage.saves(), 3);
    assert_eq!(persistent(&storage).recipient_list(), served);
}

#[test]
fn a_class_dropped_with_a_staged_write_and_no_written_list_saves_none() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    nc.add_destination(make_dest_device(1)).unwrap();
    let _forgotten = stage_saved(&mut nc, &[make_dest_device(9)]);
    drop(nc);
    // No write set the list, so storage holds none and the configuration
    // still applies at the next start.
    assert_eq!(
        storage.snapshot(),
        Some(NotificationClassSnapshot {
            recipient_list: None
        })
    );
    assert!(!persistent(&storage).recipient_list_saved());
}

#[test]
fn a_class_dropped_after_its_staged_write_was_made_keeps_the_new_list() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    write(&mut nc, &[make_dest_device(7)]).unwrap();
    let new = [make_dest_device(8)];
    let wait = stage_saved(&mut nc, &new);
    write(&mut nc, &new).unwrap();
    nc.release_staged_write(&wait);
    drop(nc);
    assert_eq!(storage.saved(), Some(new.to_vec()));
    assert_eq!(storage.saves(), 2);
    assert_eq!(persistent(&storage).recipient_list(), new);
}

#[test]
fn a_class_dropped_with_a_staged_write_whose_save_failed_leaves_storage_alone() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let served = [make_dest_device(7)];
    write(&mut nc, &served).unwrap();
    storage.fail.store(true, Ordering::SeqCst);
    let _failed = stage_saved(&mut nc, &[make_dest_device(9)]);
    // Storage works again, so a save at the drop would land and count.
    storage.fail.store(false, Ordering::SeqCst);
    drop(nc);
    assert_eq!(storage.saved(), Some(served.to_vec()));
    assert_eq!(storage.saves(), 1);
    assert_eq!(persistent(&storage).recipient_list(), served);
}

#[test]
fn a_class_dropped_while_its_staged_save_runs_puts_storage_back_after_it() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let served = [make_dest_device(7)];
    write(&mut nc, &served).unwrap();
    let held = storage.hold();
    let _forgotten =
        staged(nc.stage_write(P::RECIPIENT_LIST, None, &framed(&[make_dest_device(9)])));
    held.started.recv_timeout(WAIT).unwrap();
    // The drop waits for the saves it queues, so it runs on a thread of its
    // own while the staged save is held.
    let dropping = std::thread::spawn(move || drop(nc));
    drop(held.go);
    dropping.join().unwrap();
    // The staged save landed first, then the put-back.
    assert_eq!(storage.saved(), Some(served.to_vec()));
    assert_eq!(storage.saves(), 3);
}

#[test]
fn settling_forgotten_writes_puts_storage_back_and_frees_the_class() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let served = [make_dest_device(7)];
    write(&mut nc, &served).unwrap();
    let _forgotten = stage_saved(&mut nc, &[make_dest_device(9)]);
    // What the server's stop() does once it has joined every request.
    let settled = nc.settle_forgotten_writes().expect("the class saves");
    block_on(&settled);
    assert_eq!(storage.saved(), Some(served.to_vec()));
    assert_eq!(nc.recipient_list(), served);
    // The class is free: the next write stages at once.
    let list = [make_dest_device(8)];
    let wait = staged(nc.stage_write(P::RECIPIENT_LIST, None, &framed(&list)));
    block_on(&wait);
    write(&mut nc, &list).unwrap();
    nc.release_staged_write(&wait);
    nc.wait_for_saves();
    assert_eq!(storage.saved(), Some(list.to_vec()));
    // Nothing is staged now, and a class without storage has nothing to settle.
    assert!(nc.settle_forgotten_writes().unwrap().is_ready());
    assert!(
        NotificationClass::new(2, "NC-2")
            .unwrap()
            .settle_forgotten_writes()
            .is_none()
    );
}
