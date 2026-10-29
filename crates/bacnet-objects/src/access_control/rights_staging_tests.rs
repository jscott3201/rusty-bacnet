//! Access Rights saves that run off the database lock (#1392): the object
//! stages a rule-array or Enable write the way a Notification Class stages
//! its Recipient_List (#1315), and puts storage back when a staged write is
//! never made (#1363).

use super::test_storage::{
    MemoryPersistence, WAIT, assert_refused, block_on, octets, persistent, positive_only, write,
    write_positive, zone_rule,
};
use super::*;
use crate::durable::{DurableWrites, STAGED_WRITE_LIFETIME, SaveWait, StageStep};
use bacnet_types::enums::PropertyIdentifier as P;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

fn staged(step: StageStep) -> SaveWait {
    match step {
        StageStep::Staged(wait) => wait,
        other => panic!("expected a staged write, got {other:?}"),
    }
}

/// Stage a write of `value` to `property` at `index`, wait for its save, and
/// return the wait to release it with.
fn stage_saved(
    rights: &mut AccessRightsObject,
    property: P,
    index: Option<u32>,
    value: &PropertyValue,
) -> SaveWait {
    let wait = staged(rights.stage_write(property, index, value));
    block_on(&wait);
    wait
}

/// Stage a whole Positive_Access_Rules write of `rules` and wait for it.
fn stage_positive(rights: &mut AccessRightsObject, rules: &[BACnetAccessRule]) -> SaveWait {
    stage_saved(rights, P::POSITIVE_ACCESS_RULES, None, &octets(rules))
}

#[test]
fn a_staged_write_saves_while_the_object_serves_the_old_rules() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let old = [zone_rule(1)];
    write_positive(&mut rights, &old).unwrap();
    let held = storage.hold();
    let new = [zone_rule(2), zone_rule(3)];
    let wait = staged(rights.stage_write(P::POSITIVE_ACCESS_RULES, None, &octets(&new)));

    // The save runs on the writer thread and is held there. The object, and
    // so the database guard that would hold it, is free meanwhile: it
    // answers reads, and serves the old rules.
    let saving = held.started.recv_timeout(WAIT).unwrap();
    assert_eq!(saving, positive_only(&new));
    assert!(!wait.is_ready());
    assert_eq!(rights.positive_access_rules(), old);
    held.go.send(()).unwrap();
    block_on(&wait);

    // The write takes the saved rules without saving again.
    write_positive(&mut rights, &new).unwrap();
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    assert_eq!(rights.positive_access_rules(), new);
    assert_eq!(storage.snapshot(), Some(positive_only(&new)));
    assert_eq!(storage.saves(), 2);
}

#[test]
fn indexed_index_zero_and_enable_writes_stage_too() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    write_positive(&mut rights, &[zone_rule(1), zone_rule(2)]).unwrap();
    for (property, index, value) in [
        (P::POSITIVE_ACCESS_RULES, Some(2), octets(&[zone_rule(5)])),
        (
            P::NEGATIVE_ACCESS_RULES,
            Some(0),
            PropertyValue::Unsigned(1),
        ),
        (P::LOG_ENABLE, None, PropertyValue::Boolean(false)),
    ] {
        let wait = stage_saved(&mut rights, property, index, &value);
        write(&mut rights, property, index, value).unwrap();
        rights.release_staged_write(&wait);
    }
    rights.wait_for_saves();
    // Each write took its staged save: the first write and three staged.
    assert_eq!(storage.saves(), 4);
    let expected = AccessRightsSnapshot {
        positive_access_rules: Some(vec![zone_rule(1), zone_rule(5)]),
        negative_access_rules: Some(vec![super::super::rights_writes::grown_rule()]),
        enable: Some(false),
        accompaniment: None,
    };
    assert_eq!(storage.snapshot(), Some(expected));
    drop(rights);
    let rebuilt = persistent(&storage);
    assert_eq!(
        rebuilt.positive_access_rules(),
        [zone_rule(1), zone_rule(5)]
    );
    assert_eq!(rebuilt.negative_access_rules().len(), 1);
    assert!(!rebuilt.enable());
}

#[test]
fn a_staged_element_write_is_taken_only_by_a_write_at_its_index() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    write_positive(&mut rights, &[zone_rule(1), zone_rule(2)]).unwrap();
    let element = octets(&[zone_rule(7)]);
    let wait = stage_saved(&mut rights, P::POSITIVE_ACCESS_RULES, Some(1), &element);
    // A write of the same rule at another index, past the end, must not
    // take the staged array: it is refused as any such write is.
    assert_refused(
        write(
            &mut rights,
            P::POSITIVE_ACCESS_RULES,
            Some(3),
            element.clone(),
        ),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_ARRAY_INDEX,
    );
    // And one at index 2 saves for itself and sets element 2.
    write(&mut rights, P::POSITIVE_ACCESS_RULES, Some(2), element).unwrap();
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    let expected = [zone_rule(1), zone_rule(7)];
    assert_eq!(rights.positive_access_rules(), expected);
    assert_eq!(storage.snapshot(), Some(positive_only(&expected)));
}

#[test]
fn a_staged_write_whose_save_fails_is_refused_and_the_old_state_stays() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let old = [zone_rule(1)];
    write_positive(&mut rights, &old).unwrap();
    storage.fail.store(true, Ordering::SeqCst);
    let enable = PropertyValue::Boolean(false);
    let wait = stage_saved(&mut rights, P::LOG_ENABLE, None, &enable);
    assert_refused(
        write(&mut rights, P::LOG_ENABLE, None, enable),
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM,
    );
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    assert!(rights.enable());
    assert!(!rights.property_saved(P::LOG_ENABLE));
    assert_eq!(storage.snapshot(), Some(positive_only(&old)));
    assert_eq!(storage.saves(), 1);
}

#[test]
fn a_staged_write_its_request_never_made_leaves_storage_with_the_served_state() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let served = [zone_rule(1)];
    write_positive(&mut rights, &served).unwrap();
    let wait = stage_saved(
        &mut rights,
        P::NEGATIVE_ACCESS_RULES,
        None,
        &octets(&[zone_rule(9)]),
    );
    assert!(storage.snapshot().unwrap().negative_access_rules.is_some());
    // The request failed before its write. Releasing it saves the served
    // state at once: no written negative rules, so a configuration still
    // applies at the next start.
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    assert_eq!(storage.snapshot(), Some(positive_only(&served)));
    drop(rights);
    let mut rebuilt = persistent(&storage);
    rebuilt.set_negative_access_rules([zone_rule(2)]).unwrap();
    assert_eq!(rebuilt.negative_access_rules(), [zone_rule(2)]);
}

#[test]
fn an_operation_task_call_drops_a_forgotten_staged_write_once_its_lifetime_is_over() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let served = [zone_rule(1)];
    write_positive(&mut rights, &served).unwrap();
    let wait = stage_positive(&mut rights, &[zone_rule(9)]);
    // Its request never comes back. The first call that finds the save
    // finished starts the count on the operation task's clock.
    let start = Duration::from_secs(60);
    assert!(!rights.advance_monotonic_time_internal(start));
    rights
        .advance_monotonic_time_internal(start + STAGED_WRITE_LIFETIME - Duration::from_millis(1));
    rights.wait_for_saves();
    assert_eq!(storage.snapshot(), Some(positive_only(&[zone_rule(9)])));
    // The call a lifetime later drops it and puts storage back.
    rights.advance_monotonic_time_internal(start + STAGED_WRITE_LIFETIME);
    rights.wait_for_saves();
    assert_eq!(storage.snapshot(), Some(positive_only(&served)));
    // A release that comes after all changes nothing more.
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    assert_eq!(storage.saves(), 3);
}

#[test]
fn staging_skips_writes_the_object_does_not_save_or_will_refuse() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let skipped = |step| matches!(step, StageStep::Skip);
    let rules = octets(&[zone_rule(1)]);
    assert!(skipped(rights.stage_write(
        P::DESCRIPTION,
        None,
        &PropertyValue::CharacterString("doors".into())
    )));
    assert!(skipped(rights.stage_write(
        P::GLOBAL_IDENTIFIER,
        None,
        &PropertyValue::Unsigned(1)
    )));
    // Writes the object refuses anyway: malformed octets, an index past the
    // end, a NULL Enable, and an indexed Enable.
    for (property, index, value) in [
        (
            P::POSITIVE_ACCESS_RULES,
            None,
            PropertyValue::ApplicationData(vec![0xFF]),
        ),
        (P::NEGATIVE_ACCESS_RULES, Some(1), rules.clone()),
        (P::LOG_ENABLE, None, PropertyValue::Null),
        (P::LOG_ENABLE, Some(1), PropertyValue::Boolean(true)),
    ] {
        assert!(
            skipped(rights.stage_write(property, index, &value)),
            "{property:?} {index:?}"
        );
    }
    // A direct indexed Enable write gets what the handlers answer.
    assert_refused(
        write(
            &mut rights,
            P::LOG_ENABLE,
            Some(1),
            PropertyValue::Boolean(true),
        ),
        ErrorClass::PROPERTY,
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
    let mut in_memory = AccessRightsObject::new(2, "AR-2").unwrap();
    assert!(skipped(in_memory.stage_write(
        P::POSITIVE_ACCESS_RULES,
        None,
        &rules
    )));
    assert!(in_memory.settle_forgotten_writes().is_none());
    rights.wait_for_saves();
    assert_eq!(storage.saves(), 0);
}

// An object can go while a write is still staged for a request that never
// came back, as when the server stops mid-request and the database is
// dropped (#1363). Storage then goes back to what the object served.

#[test]
fn an_object_dropped_with_a_staged_write_puts_storage_back_to_the_served_state() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let served = [zone_rule(1)];
    write_positive(&mut rights, &served).unwrap();
    let _forgotten = stage_positive(&mut rights, &[zone_rule(9)]);
    assert_eq!(storage.snapshot(), Some(positive_only(&[zone_rule(9)])));
    // The object goes before any lifetime check could drop the staged write.
    drop(rights);
    assert_eq!(storage.snapshot(), Some(positive_only(&served)));
    // The first write, the staged one and the put-back.
    assert_eq!(storage.saves(), 3);
    assert_eq!(persistent(&storage).positive_access_rules(), served);
}

#[test]
fn an_object_dropped_with_a_staged_write_whose_save_failed_leaves_storage_alone() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let served = [zone_rule(1)];
    write_positive(&mut rights, &served).unwrap();
    storage.fail.store(true, Ordering::SeqCst);
    let _failed = stage_positive(&mut rights, &[zone_rule(9)]);
    // Storage works again, so a save at the drop would land and count.
    storage.fail.store(false, Ordering::SeqCst);
    drop(rights);
    assert_eq!(storage.snapshot(), Some(positive_only(&served)));
    assert_eq!(storage.saves(), 1);
}

#[test]
fn an_object_dropped_while_its_staged_save_runs_puts_storage_back_after_it() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let served = [zone_rule(1)];
    write_positive(&mut rights, &served).unwrap();
    let held = storage.hold();
    let _forgotten =
        staged(rights.stage_write(P::POSITIVE_ACCESS_RULES, None, &octets(&[zone_rule(9)])));
    held.started.recv_timeout(WAIT).unwrap();
    // The drop waits for the saves it queues, so it runs on a thread of its
    // own while the staged save is held.
    let dropping = std::thread::spawn(move || drop(rights));
    drop(held.go);
    dropping.join().unwrap();
    // The staged save landed first, then the put-back.
    assert_eq!(storage.snapshot(), Some(positive_only(&served)));
    assert_eq!(storage.saves(), 3);
}

#[test]
fn settling_forgotten_writes_puts_storage_back_and_frees_the_object() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let served = [zone_rule(1)];
    write_positive(&mut rights, &served).unwrap();
    let _forgotten = stage_positive(&mut rights, &[zone_rule(9)]);
    // What the server's stop() does once it has joined every request.
    let settled = rights.settle_forgotten_writes().expect("the object saves");
    block_on(&settled);
    assert_eq!(storage.snapshot(), Some(positive_only(&served)));
    assert_eq!(rights.positive_access_rules(), served);
    // The object is free: the next write stages at once.
    let next = [zone_rule(8)];
    let wait = stage_positive(&mut rights, &next);
    write_positive(&mut rights, &next).unwrap();
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    assert_eq!(storage.snapshot(), Some(positive_only(&next)));
    assert!(rights.settle_forgotten_writes().unwrap().is_ready());
}
