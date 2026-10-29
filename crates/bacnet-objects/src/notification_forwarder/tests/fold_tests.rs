//! A request's writes to both lists stage one save (#1423), and a write the
//! forwarder won't make leaves a write staged for another request alone
//! (#1424).

use super::storage::{MemoryPersistence, WAIT, block_on, persistent};
use super::*;
use crate::durable::{DurableWrites, PendingWrite, SaveWait, StageStep};
use bacnet_types::enums::PropertyIdentifier as P;

fn staged(step: StageStep) -> SaveWait {
    match step {
        StageStep::Staged(wait) => wait,
        other => panic!("expected a staged write, got {other:?}"),
    }
}

fn pending(property: P, value: PropertyValue) -> PendingWrite {
    PendingWrite {
        property,
        array_index: None,
        value,
    }
}

fn make(nf: &mut NotificationForwarderObject, write: &PendingWrite) -> Result<(), Error> {
    nf.write_property(write.property, None, write.value.clone(), None)
}

#[test]
fn a_request_writing_both_lists_stages_one_save() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let destinations = [destination(device(3), 1, false)];
    let first = [subscription(device(7), 1, 10)];
    let last = [
        subscription(device(7), 1, 10),
        subscription(device(8), 2, 20),
    ];
    let writes = [
        pending(P::RECIPIENT_LIST, framed_destinations(&destinations)),
        pending(
            P::DESCRIPTION,
            PropertyValue::CharacterString("relay".into()),
        ),
        pending(P::SUBSCRIBED_RECIPIENTS, framed_subscriptions(&first)),
        pending(P::SUBSCRIBED_RECIPIENTS, framed_subscriptions(&last)),
    ];
    let held = storage.hold();
    let wait = staged(nf.stage_writes(&writes));
    let saving = held.started.recv_timeout(WAIT).unwrap();
    assert_eq!(saving.recipient_list, Some(destinations.to_vec()));
    assert_eq!(saving.subscribed_recipients, last);
    drop(held.go);
    block_on(wait.clone());
    for write in &writes {
        make(&mut nf, write).unwrap();
        if write.property == P::RECIPIENT_LIST {
            // The first step is served before the request goes on.
            assert_eq!(nf.recipient_list(), destinations);
            assert!(nf.subscriptions().is_empty());
        }
    }
    nf.release_staged_write(&wait);
    nf.wait_for_saves();
    assert_eq!(nf.subscriptions(), last);
    assert_eq!(storage.saves(), 1);
    assert_eq!(storage.saved(), last);
}

#[test]
fn a_write_the_forwarder_wont_make_leaves_another_requests_staged_write_alone() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let new = [subscription(device(8), 2, 20)];
    let held = storage.hold();
    let wait = staged(nf.stage_write(P::SUBSCRIBED_RECIPIENTS, None, &framed_subscriptions(&new)));
    held.started.recv_timeout(WAIT).unwrap();
    // A subscription with no time left, which the forwarder refuses, and a
    // NULL the server answers as a success that changes nothing (#1396),
    // come by another path while the save runs.
    let refused = framed_subscriptions(&[subscription(device(9), 1, 0)]);
    assert_refused(
        make(&mut nf, &pending(P::SUBSCRIBED_RECIPIENTS, refused)),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    let null = PropertyValue::ApplicationData(vec![0x00]);
    for property in [P::SUBSCRIBED_RECIPIENTS, P::RECIPIENT_LIST] {
        assert_refused(
            make(&mut nf, &pending(property, null.clone())),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert!(matches!(
        nf.stage_write(P::RECIPIENT_LIST, None, &framed_destinations(&[])),
        StageStep::Busy(_)
    ));
    drop(held.go);
    block_on(wait.clone());
    // Its request takes the save it staged, without saving again.
    let write = pending(P::SUBSCRIBED_RECIPIENTS, framed_subscriptions(&new));
    make(&mut nf, &write).unwrap();
    nf.release_staged_write(&wait);
    nf.wait_for_saves();
    assert_eq!(nf.subscriptions(), new);
    assert_eq!(storage.saves(), 1);
}
