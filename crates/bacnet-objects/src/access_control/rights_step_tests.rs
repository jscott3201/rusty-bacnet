//! What an Access Rights object's staged steps hold (#2526): only what their
//! writes set. The stage works the request out on one working copy and
//! saves it once; each write is made on the rules served as it is taken,
//! and the served state storage would go back to follows each write without
//! both arrays being copied again.

use super::test_storage::{
    MemoryPersistence, assert_refused, block_on, octets, persistent, write, write_positive,
    zone_rule,
};
use super::*;
use crate::access_control::rights_writes::grown_rule;
use crate::durable::{DurableWrites, PendingWrite, SaveWait, StageStep};
use bacnet_types::enums::PropertyIdentifier as P;
use std::sync::Arc;

fn pending(property: P, index: Option<u32>, value: PropertyValue) -> PendingWrite {
    PendingWrite {
        property,
        array_index: index,
        value,
    }
}

/// Stage `writes` and wait for their save; the wait releases them.
fn staged(rights: &mut AccessRightsObject, writes: &[PendingWrite]) -> SaveWait {
    let StageStep::Staged(wait) = rights.stage_writes(writes) else {
        panic!("the writes didn't stage");
    };
    block_on(&wait);
    wait
}

fn make(rights: &mut AccessRightsObject, write: &PendingWrite) -> Result<(), Error> {
    let PendingWrite {
        property,
        array_index,
        value,
    } = write.clone();
    self::write(rights, property, array_index, value)
}

fn rule(instance: u32) -> PropertyValue {
    octets(&[zone_rule(instance)])
}

/// Every claim, the later ones included, serves its request's prefix and
/// takes its step, and storage ends where the stage saved it, once.
#[test]
fn each_claim_makes_its_write_on_the_rules_served() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    write_positive(&mut rights, &[zone_rule(1)]).unwrap();
    let writes = [
        pending(
            P::POSITIVE_ACCESS_RULES,
            Some(0),
            PropertyValue::Unsigned(3),
        ),
        pending(P::POSITIVE_ACCESS_RULES, Some(3), rule(3)),
        pending(P::POSITIVE_ACCESS_RULES, Some(1), rule(4)),
        pending(P::NEGATIVE_ACCESS_RULES, None, rule(5)),
        pending(P::POSITIVE_ACCESS_RULES, Some(2), rule(6)),
        pending(P::ENABLE, None, PropertyValue::Boolean(false)),
    ];
    let (z, g) = (zone_rule, grown_rule);
    let served = [
        vec![z(1), g(), g()],
        vec![z(1), g(), z(3)],
        vec![z(4), g(), z(3)],
        vec![z(4), g(), z(3)],
        vec![z(4), z(6), z(3)],
        vec![z(4), z(6), z(3)],
    ];
    let wait = staged(&mut rights, &writes);
    let saved = storage.snapshot();
    for (at, (write, positive)) in writes.iter().zip(&served).enumerate() {
        make(&mut rights, write).unwrap();
        assert_eq!(
            rights.positive_access_rules(),
            positive.as_slice(),
            "write {at}"
        );
        assert_eq!(
            rights.has_staged_write(),
            at + 1 < writes.len(),
            "write {at}"
        );
    }
    assert_eq!(rights.negative_access_rules(), [z(5)]);
    assert!(!rights.enable());
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    assert_eq!(storage.snapshot(), saved);
    assert_eq!(storage.saves(), 2);
}

/// An object dropped part way through a request puts storage back to what
/// it served then: the writes it made, not the rules before the request nor
/// the ones the stage saved (#1363). The served state it keeps follows each
/// write.
#[test]
fn a_drop_part_way_puts_storage_back_to_the_writes_made() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    write_positive(&mut rights, &[zone_rule(1), zone_rule(2)]).unwrap();
    let writes = [
        pending(P::POSITIVE_ACCESS_RULES, Some(2), rule(7)),
        pending(P::NEGATIVE_ACCESS_RULES, None, rule(8)),
        pending(P::POSITIVE_ACCESS_RULES, Some(1), rule(9)),
    ];
    let _wait = staged(&mut rights, &writes);
    for write in &writes[..2] {
        make(&mut rights, write).unwrap();
    }
    drop(rights);
    let kept = storage.snapshot().unwrap();
    assert_eq!(
        kept.positive_access_rules,
        Some(vec![zone_rule(1), zone_rule(7)])
    );
    assert_eq!(kept.negative_access_rules, Some(vec![zone_rule(8)]));
    assert_eq!(storage.saves(), 3);
}

/// A later write of a request is checked against the rules the earlier ones
/// leave, not the ones served: an element past the end of an array the
/// request shortens stops the stage, and is refused as made.
#[test]
fn a_later_write_is_checked_against_the_rules_the_earlier_ones_leave() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    write_positive(&mut rights, &[zone_rule(1), zone_rule(2), zone_rule(3)]).unwrap();
    let writes = [
        pending(
            P::POSITIVE_ACCESS_RULES,
            None,
            octets(&[zone_rule(4), zone_rule(5)]),
        ),
        pending(P::POSITIVE_ACCESS_RULES, Some(3), rule(9)),
    ];
    let wait = staged(&mut rights, &writes);
    let two = Some(vec![zone_rule(4), zone_rule(5)]);
    assert_eq!(storage.snapshot().unwrap().positive_access_rules, two);
    make(&mut rights, &writes[0]).unwrap();
    assert_refused(
        make(&mut rights, &writes[1]),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_ARRAY_INDEX,
    );
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    assert_eq!(storage.snapshot().unwrap().positive_access_rules, two);
    assert_eq!(storage.saves(), 2);
}

/// The application's rules, given while a peer's element write to the same
/// array is staged, are a counted change: the staged step gives way, and
/// the write is made, and saved, on the rules the application gave. Before
/// #2526 the write took the step worked out on the old rules, undoing the
/// application's; with the step taken on the new ones, storage would keep
/// the old.
#[test]
fn a_setter_during_a_stage_makes_the_staged_write_give_way() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    rights
        .set_positive_access_rules([zone_rule(1), zone_rule(2)])
        .unwrap();
    let write = pending(P::POSITIVE_ACCESS_RULES, Some(1), rule(9));
    let wait = staged(&mut rights, core::slice::from_ref(&write));
    rights
        .set_positive_access_rules([zone_rule(3), zone_rule(4), zone_rule(5)])
        .unwrap();
    make(&mut rights, &write).unwrap();
    let made = vec![zone_rule(9), zone_rule(4), zone_rule(5)];
    assert_eq!(rights.positive_access_rules(), made.as_slice());
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    assert_eq!(
        storage.snapshot().unwrap().positive_access_rules,
        Some(made)
    );
}
