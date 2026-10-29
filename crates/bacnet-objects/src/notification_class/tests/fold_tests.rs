//! A request's Recipient_List writes stage one save (#1423), and a write the
//! class won't make leaves a write staged for another request alone (#1424).

use super::super::*;
use super::make_dest_device;
use super::persistence_tests::{assert_refused, framed, write};
use super::storage::{MemoryPersistence, WAIT, block_on, persistent};
use crate::durable::{DurableWrites, PendingWrite, SaveWait, StageStep};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use std::sync::Arc;

fn staged(step: StageStep) -> SaveWait {
    match step {
        StageStep::Staged(wait) => wait,
        other => panic!("expected a staged write, got {other:?}"),
    }
}

fn list_write(list: &[BACnetDestination]) -> PendingWrite {
    PendingWrite {
        property: P::RECIPIENT_LIST,
        array_index: None,
        value: framed(list),
    }
}

#[test]
fn a_request_writing_the_list_twice_stages_one_save_of_the_last() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let first = [make_dest_device(7)];
    let last = [make_dest_device(8), make_dest_device(9)];
    let held = storage.hold();
    let wait = staged(nc.stage_writes(&[list_write(&first), list_write(&last)]));
    let saving = held.started.recv_timeout(WAIT).unwrap();
    assert_eq!(saving.recipient_list, Some(last.to_vec()));
    drop(held.go);
    block_on(&wait);
    // Each write takes its step, the class serving the list it leaves.
    write(&mut nc, &first).unwrap();
    assert_eq!(nc.recipient_list(), first);
    write(&mut nc, &last).unwrap();
    nc.release_staged_write(&wait);
    nc.wait_for_saves();
    assert_eq!(nc.recipient_list(), last);
    assert_eq!(storage.saved(), Some(last.to_vec()));
    assert_eq!(storage.saves(), 1);
}

#[test]
fn a_write_the_class_wont_make_leaves_another_requests_staged_write_alone() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let new = [make_dest_device(8)];
    let held = storage.hold();
    let wait = staged(nc.stage_write(P::RECIPIENT_LIST, None, &framed(&new)));
    held.started.recv_timeout(WAIT).unwrap();
    // A malformed list, and a NULL the server answers as a success that
    // changes nothing (#1396), come by another path while the save runs.
    for value in [vec![0x21, 0x01], vec![0x00]] {
        assert_refused(
            nc.write_property(
                P::RECIPIENT_LIST,
                None,
                PropertyValue::ApplicationData(value),
                None,
            ),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert!(matches!(
        nc.stage_write(P::RECIPIENT_LIST, None, &framed(&[])),
        StageStep::Busy(_)
    ));
    drop(held.go);
    block_on(&wait);
    // Its request takes the save it staged, without saving again.
    write(&mut nc, &new).unwrap();
    nc.release_staged_write(&wait);
    nc.wait_for_saves();
    assert_eq!(nc.recipient_list(), new);
    assert_eq!(storage.saves(), 1);
}
