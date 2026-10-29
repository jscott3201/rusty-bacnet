//! Recipient_List across a restart (Clause 12.51.8, #1256).

use super::storage::{MemoryPersistence, persistent};
use super::*;
use bacnet_types::enums::PropertyIdentifier as P;
use std::sync::atomic::Ordering;

fn write_recipients(
    nf: &mut NotificationForwarderObject,
    list: &[BACnetDestination],
) -> Result<(), Error> {
    nf.write_property(P::RECIPIENT_LIST, None, framed_destinations(list), None)
}

#[test]
fn a_written_recipient_list_reads_back_after_a_rebuild() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let list = [
        destination(device(10), 3, true),
        destination(address(7, &[192, 168, 1, 9, 0xBA, 0xC0]), 0, false),
    ];
    write_recipients(&mut nf, &list).unwrap();
    assert_eq!(storage.snapshot().recipient_list, Some(list.to_vec()));
    drop(nf);

    let rebuilt = persistent(&storage);
    assert!(rebuilt.recipient_list_saved());
    assert_eq!(rebuilt.recipient_list(), list);
    assert_eq!(
        rebuilt.read_property(P::RECIPIENT_LIST, None).unwrap(),
        framed_destinations(&list)
    );
}

#[test]
fn saved_recipient_list_wins_over_configured_destinations() {
    let storage = Arc::new(MemoryPersistence::default());
    let configured = destination(device(1), 1, false);
    let written = destination(device(2), 2, true);

    // First boot: storage holds nothing, so the configured destination is
    // the list. Configuration is not saved.
    let mut first = persistent(&storage);
    assert!(!first.recipient_list_saved());
    first.add_destination(configured.clone()).unwrap();
    assert_eq!(first.recipient_list(), std::slice::from_ref(&configured));
    first.advance_monotonic_time_internal(Duration::ZERO);
    first.wait_for_saves();
    assert_eq!(storage.saves(), 0);
    // An operator then replaces the list over the network, and it is saved.
    write_recipients(&mut first, std::slice::from_ref(&written)).unwrap();
    assert!(first.recipient_list_saved());
    drop(first);

    // Second boot: the application configures the same destination again,
    // but the written list wins.
    let mut second = persistent(&storage);
    assert!(second.recipient_list_saved());
    second.add_destination(configured).unwrap();
    assert_eq!(second.recipient_list(), std::slice::from_ref(&written));
    // A configured destination is still checked.
    let mut too_long = destination(device(3), 1, false);
    too_long.recipient = address(1, &[0; 19]);
    assert_refused(
        second.add_destination(too_long),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    second.advance_monotonic_time_internal(Duration::ZERO);
    second.wait_for_saves();
    assert_eq!(storage.snapshot().recipient_list, Some(vec![written]));
}

#[test]
fn configured_destinations_apply_at_every_start_until_a_write() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut first = persistent(&storage);
    first
        .add_destination(destination(device(1), 1, false))
        .unwrap();
    // A subscription write saves the lists, but no Recipient_List with them.
    first
        .write_property(
            P::SUBSCRIBED_RECIPIENTS,
            None,
            framed_subscriptions(&[subscription(device(7), 1, 10)]),
            None,
        )
        .unwrap();
    assert_eq!(storage.snapshot().recipient_list, None);
    drop(first);
    // The application's next configuration applies.
    let mut second = persistent(&storage);
    assert!(!second.recipient_list_saved());
    second
        .add_destination(destination(device(2), 1, false))
        .unwrap();
    assert_eq!(second.recipient_list(), [destination(device(2), 1, false)]);
    assert_eq!(second.subscriptions(), [subscription(device(7), 1, 10)]);
}

#[test]
fn a_saved_empty_recipient_list_still_wins() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut first = persistent(&storage);
    first
        .add_destination(destination(device(1), 1, false))
        .unwrap();
    write_recipients(&mut first, &[]).unwrap();
    drop(first);
    let mut second = persistent(&storage);
    second
        .add_destination(destination(device(1), 1, false))
        .unwrap();
    assert!(second.recipient_list().is_empty());
    assert!(second.recipient_list_saved());
}

#[test]
fn a_recipient_list_write_that_cannot_be_saved_is_refused() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let kept = [destination(device(1), 1, false)];
    write_recipients(&mut nf, &kept).unwrap();
    let counters = nf.save_counters();
    storage.fail.store(true, Ordering::SeqCst);
    assert_refused(
        write_recipients(&mut nf, &[destination(device(2), 1, false)]),
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM,
    );
    assert_eq!(nf.recipient_list(), kept);
    assert_eq!(storage.snapshot().recipient_list, Some(kept.to_vec()));
    assert_eq!(counters.failed_saves(), 1);
}

#[test]
fn a_recipient_list_write_saves_the_subscriptions_with_it() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    let subscriptions = [subscription(device(7), 1, 10)];
    nf.write_property(
        P::SUBSCRIBED_RECIPIENTS,
        None,
        framed_subscriptions(&subscriptions),
        None,
    )
    .unwrap();
    let list = [destination(device(1), 1, false)];
    write_recipients(&mut nf, &list).unwrap();
    assert_eq!(
        storage.snapshot(),
        ForwarderSnapshot {
            recipient_list: Some(list.to_vec()),
            subscribed_recipients: subscriptions.to_vec(),
        }
    );
}

#[test]
fn a_saved_recipient_list_past_the_cap_refuses_the_forwarder() {
    let storage = Arc::new(MemoryPersistence::default());
    storage.preload(
        ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, 1).unwrap(),
        ForwarderSnapshot {
            recipient_list: Some(
                (0..=MAX_RECIPIENT_LIST_DESTINATIONS as u32)
                    .map(|instance| destination(device(instance), 1, false))
                    .collect(),
            ),
            subscribed_recipients: Vec::new(),
        },
    );
    assert!(NotificationForwarderObject::with_persistence(1, "NF", storage).is_err());
}
