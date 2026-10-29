//! A request's several writes to an Access Rights object stage one save
//! (#1423), and a write the object won't make leaves a write staged for
//! another request alone (#1424).

use super::test_storage::{
    MemoryPersistence, WAIT, assert_refused, block_on, octets, persistent, positive_only, write,
    write_positive, zone_rule,
};
use super::*;
use crate::durable::{DurableWrites, PendingWrite, SaveWait, StageStep};
use bacnet_types::enums::PropertyIdentifier as P;
use std::sync::Arc;

fn staged(step: StageStep) -> SaveWait {
    match step {
        StageStep::Staged(wait) => wait,
        other => panic!("expected a staged write, got {other:?}"),
    }
}

fn pending(property: P, array_index: Option<u32>, value: PropertyValue) -> PendingWrite {
    PendingWrite {
        property,
        array_index,
        value,
    }
}

/// Make `write` as the handler would.
fn make(rights: &mut AccessRightsObject, write: &PendingWrite) -> Result<(), Error> {
    super::test_storage::write(
        rights,
        write.property,
        write.array_index,
        write.value.clone(),
    )
}

/// A head end provisioning the object: both rule arrays and Enable, then an
/// element write that edits the array the first write left.
fn provisioning() -> Vec<PendingWrite> {
    vec![
        pending(
            P::POSITIVE_ACCESS_RULES,
            None,
            octets(&[zone_rule(1), zone_rule(2)]),
        ),
        pending(P::NEGATIVE_ACCESS_RULES, None, octets(&[zone_rule(3)])),
        pending(P::LOG_ENABLE, None, PropertyValue::Boolean(false)),
        pending(P::POSITIVE_ACCESS_RULES, Some(2), octets(&[zone_rule(4)])),
    ]
}

/// What storage holds once every write of [`provisioning`] is made.
fn provisioned() -> AccessRightsSnapshot {
    AccessRightsSnapshot {
        positive_access_rules: Some(vec![zone_rule(1), zone_rule(4)]),
        negative_access_rules: Some(vec![zone_rule(3)]),
        enable: Some(false),
        accompaniment: None,
    }
}

#[test]
fn a_requests_writes_stage_one_save_and_each_takes_its_step() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let writes = provisioning();
    let held = storage.hold();
    let wait = staged(rights.stage_writes(&writes));
    // One save, of the state every write leaves, while the object serves
    // what it did.
    assert_eq!(held.started.recv_timeout(WAIT).unwrap(), provisioned());
    assert!(rights.positive_access_rules().is_empty());
    assert!(rights.enable());
    drop(held.go);
    block_on(&wait);

    // Each write takes its own step, so the object serves what the writes
    // made so far leave, and the last state only once all are made.
    make(&mut rights, &writes[0]).unwrap();
    assert_eq!(rights.positive_access_rules(), [zone_rule(1), zone_rule(2)]);
    assert!(rights.negative_access_rules().is_empty());
    make(&mut rights, &writes[1]).unwrap();
    make(&mut rights, &writes[2]).unwrap();
    assert!(!rights.enable());
    assert_eq!(rights.positive_access_rules(), [zone_rule(1), zone_rule(2)]);
    make(&mut rights, &writes[3]).unwrap();
    assert_eq!(rights.positive_access_rules(), [zone_rule(1), zone_rule(4)]);
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    // No write saved again, and the release found nothing to put back.
    assert_eq!(storage.saves(), 1);
    assert_eq!(storage.snapshot(), Some(provisioned()));
    drop(rights);
    let rebuilt = persistent(&storage);
    assert_eq!(
        rebuilt.positive_access_rules(),
        [zone_rule(1), zone_rule(4)]
    );
    assert_eq!(rebuilt.negative_access_rules(), [zone_rule(3)]);
    assert!(!rebuilt.enable());
}

#[test]
fn a_request_that_stops_part_way_puts_storage_back_to_the_steps_it_took() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let writes = provisioning();
    let wait = staged(rights.stage_writes(&writes));
    block_on(&wait);
    assert_eq!(storage.snapshot(), Some(provisioned()));
    // The request makes its first write, then stops at an attempt that
    // fails before its second reaches the object.
    make(&mut rights, &writes[0]).unwrap();
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    let first = [zone_rule(1), zone_rule(2)];
    assert_eq!(rights.positive_access_rules(), first);
    assert!(rights.enable());
    assert_eq!(storage.snapshot(), Some(positive_only(&first)));
    assert_eq!(storage.saves(), 2);
    // The object is free: the next write stages at once.
    let enable = PropertyValue::Boolean(false);
    let wait = staged(rights.stage_write(P::LOG_ENABLE, None, &enable));
    block_on(&wait);
    write(&mut rights, P::LOG_ENABLE, None, enable).unwrap();
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    assert_eq!(storage.saves(), 3);
}

#[test]
fn staging_ends_at_a_write_the_object_will_refuse_and_passes_a_null_by() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let rules = [zone_rule(1)];
    let writes = vec![
        // A NULL the object refuses as the wrong datatype, which the server
        // answers as a success that changes nothing (#1396).
        pending(P::LOG_ENABLE, None, PropertyValue::Null),
        pending(
            P::DESCRIPTION,
            None,
            PropertyValue::CharacterString("door".into()),
        ),
        pending(P::POSITIVE_ACCESS_RULES, None, octets(&rules)),
        // Past the end, so refused: the request makes nothing after it.
        pending(P::NEGATIVE_ACCESS_RULES, Some(1), octets(&rules)),
        pending(P::LOG_ENABLE, None, PropertyValue::Boolean(false)),
    ];
    let wait = staged(rights.stage_writes(&writes));
    block_on(&wait);
    assert_eq!(storage.snapshot(), Some(positive_only(&rules)));
    assert_refused(
        make(&mut rights, &writes[0]),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    make(&mut rights, &writes[1]).unwrap();
    make(&mut rights, &writes[2]).unwrap();
    assert_refused(
        make(&mut rights, &writes[3]),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_ARRAY_INDEX,
    );
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    assert_eq!(storage.saves(), 1);
    assert_eq!(storage.snapshot(), Some(positive_only(&rules)));
    assert!(rights.enable());
}

#[test]
fn a_write_the_object_wont_make_leaves_another_requests_staged_write_alone() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    write_positive(&mut rights, &[zone_rule(1)]).unwrap();
    let held = storage.hold();
    let new = [zone_rule(2), zone_rule(3)];
    let wait = staged(rights.stage_write(P::POSITIVE_ACCESS_RULES, None, &octets(&new)));
    held.started.recv_timeout(WAIT).unwrap();
    // While its save runs, writes come by another path that the object
    // refuses, or that are NULLs it refuses as the wrong datatype.
    for (property, index, value, code) in [
        (
            P::POSITIVE_ACCESS_RULES,
            Some(5),
            octets(&[zone_rule(9)]),
            ErrorCode::INVALID_ARRAY_INDEX,
        ),
        (
            P::POSITIVE_ACCESS_RULES,
            None,
            PropertyValue::ApplicationData(vec![0x21, 0x01]),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            P::LOG_ENABLE,
            None,
            PropertyValue::Null,
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            P::NEGATIVE_ACCESS_RULES,
            None,
            PropertyValue::ApplicationData(vec![0x00]),
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        assert_refused(
            write(&mut rights, property, index, value),
            ErrorClass::PROPERTY,
            code,
        );
    }
    // The staged write still holds the object.
    let enable = PropertyValue::Boolean(false);
    assert!(matches!(
        rights.stage_write(P::LOG_ENABLE, None, &enable),
        StageStep::Busy(_)
    ));
    drop(held.go);
    block_on(&wait);
    // Its request takes the save it staged, without saving again.
    write_positive(&mut rights, &new).unwrap();
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    assert_eq!(storage.saves(), 2);
    assert_eq!(storage.snapshot(), Some(positive_only(&new)));
}

#[test]
fn a_failed_save_of_several_steps_drops_them_all_and_storage_keeps_the_served_state() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let served = [zone_rule(1)];
    write_positive(&mut rights, &served).unwrap();
    storage
        .fail
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let writes = provisioning();
    let wait = staged(rights.stage_writes(&writes));
    block_on(&wait);
    assert!(rights.has_staged_write());
    // The first write gets the save's error, and with it every step goes:
    // none is left for the request's later writes to take.
    assert_refused(
        make(&mut rights, &writes[0]),
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM,
    );
    assert!(!rights.has_staged_write());
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    // Nothing was saved, nothing needs putting back, and the object serves
    // what it did.
    assert_eq!(storage.saves(), 1);
    assert_eq!(storage.snapshot(), Some(positive_only(&served)));
    assert_eq!(rights.positive_access_rules(), served);
    assert!(rights.negative_access_rules().is_empty());
    assert!(rights.enable());
}
