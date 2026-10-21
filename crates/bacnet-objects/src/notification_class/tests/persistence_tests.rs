//! A Notification Class's Recipient_List across a restart (Clause 12.21.8,
//! #1315).

use super::super::*;
use super::make_dest_device;
use super::storage::{persistent, MemoryPersistence};
use bacnet_encoding::constructed::encode_destination_list;
use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::BytesMut;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub(super) fn framed(list: &[BACnetDestination]) -> PropertyValue {
    let mut buf = BytesMut::new();
    encode_destination_list(&mut buf, list).unwrap();
    PropertyValue::ApplicationData(buf.to_vec())
}

pub(super) fn write(nc: &mut NotificationClass, list: &[BACnetDestination]) -> Result<(), Error> {
    nc.write_property(PropertyIdentifier::RECIPIENT_LIST, None, framed(list), None)
}

/// A destination naming an address on network 7.
fn address_destination(mac: &[u8]) -> BACnetDestination {
    BACnetDestination {
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 7,
            mac_address: bacnet_types::MacAddr::from_slice(mac),
        }),
        process_identifier: 0,
        issue_confirmed_notifications: false,
        ..make_dest_device(1)
    }
}

pub(super) fn assert_refused(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    match result {
        Err(Error::Protocol {
            class: got_class,
            code: got_code,
        }) => assert_eq!(
            (got_class, got_code),
            (class, code),
            "expected {class:?} / {code:?}"
        ),
        other => panic!("expected {class:?} / {code:?}, got {other:?}"),
    }
}

#[test]
fn a_written_recipient_list_reads_back_after_a_rebuild() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let list = [
        make_dest_device(10),
        address_destination(&[192, 168, 1, 9, 0xBA, 0xC0]),
    ];
    write(&mut nc, &list).unwrap();
    assert_eq!(storage.saved(), Some(list.to_vec()));
    drop(nc);

    let rebuilt = persistent(&storage);
    assert!(rebuilt.recipient_list_saved());
    assert_eq!(rebuilt.recipient_list(), list);
    assert_eq!(
        rebuilt
            .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
            .unwrap(),
        framed(&list)
    );
}

#[test]
fn saved_recipient_list_wins_over_configured_destinations() {
    let storage = Arc::new(MemoryPersistence::default());
    let configured = make_dest_device(1);
    let written = make_dest_device(2);

    // First boot: storage holds nothing, so the configured destination is
    // the list, and it is not saved.
    let mut first = persistent(&storage);
    assert!(!first.recipient_list_saved());
    first.add_destination(configured.clone()).unwrap();
    assert_eq!(first.recipient_list(), std::slice::from_ref(&configured));
    first.wait_for_saves();
    assert_eq!(storage.saves(), 0);
    // An operator then replaces the list over the network, and it is saved.
    write(&mut first, std::slice::from_ref(&written)).unwrap();
    assert!(first.recipient_list_saved());
    drop(first);

    // Second boot: the application configures the same destination again,
    // but the written list wins.
    let mut second = persistent(&storage);
    assert!(second.recipient_list_saved());
    second.add_destination(configured).unwrap();
    assert_eq!(second.recipient_list(), std::slice::from_ref(&written));
    // A configured destination is still checked.
    assert_refused(
        second.add_destination(address_destination(&[0; 19])),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    second.wait_for_saves();
    assert_eq!(storage.saves(), 1);
    assert_eq!(storage.saved(), Some(vec![written]));
}

#[test]
fn configured_destinations_apply_at_every_start_until_a_write() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut first = persistent(&storage);
    first.add_destination(make_dest_device(1)).unwrap();
    // Writes to other properties save nothing.
    first
        .write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("alarms".into()),
            None,
        )
        .unwrap();
    first.wait_for_saves();
    assert_eq!(storage.snapshot(), None);
    drop(first);
    // The application's next configuration applies.
    let mut second = persistent(&storage);
    assert!(!second.recipient_list_saved());
    second.add_destination(make_dest_device(2)).unwrap();
    assert_eq!(second.recipient_list(), [make_dest_device(2)]);
    // A class without persistence always takes configured destinations.
    let mut in_memory = NotificationClass::new(2, "NC-2").unwrap();
    write(&mut in_memory, &[make_dest_device(3)]).unwrap();
    in_memory.add_destination(make_dest_device(4)).unwrap();
    assert!(!in_memory.recipient_list_saved());
    assert_eq!(
        in_memory.recipient_list(),
        [make_dest_device(3), make_dest_device(4)]
    );
}

#[test]
fn a_saved_empty_recipient_list_still_wins() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut first = persistent(&storage);
    first.add_destination(make_dest_device(1)).unwrap();
    write(&mut first, &[]).unwrap();
    drop(first);
    let mut second = persistent(&storage);
    second.add_destination(make_dest_device(1)).unwrap();
    assert!(second.recipient_list().is_empty());
    assert!(second.recipient_list_saved());
}

#[test]
fn a_recipient_list_write_that_cannot_be_saved_is_refused() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nc = persistent(&storage);
    let kept = [make_dest_device(1)];
    write(&mut nc, &kept).unwrap();
    storage.fail.store(true, Ordering::SeqCst);
    assert_refused(
        write(&mut nc, &[make_dest_device(2)]),
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM,
    );
    assert_eq!(nc.recipient_list(), kept);
    assert_eq!(storage.saved(), Some(kept.to_vec()));
    // A list the write itself refuses fails as before, without a save.
    storage.fail.store(false, Ordering::SeqCst);
    assert_refused(
        nc.write_property(
            PropertyIdentifier::RECIPIENT_LIST,
            None,
            PropertyValue::ApplicationData(vec![0xFF]),
            None,
        ),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_eq!(storage.saves(), 1);
}

#[test]
fn a_saved_recipient_list_a_write_would_refuse_refuses_the_class() {
    let class = ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap();
    let past_the_cap = (0..=MAX_RECIPIENT_LIST_DESTINATIONS as u32)
        .map(make_dest_device)
        .collect();
    for saved in [past_the_cap, vec![address_destination(&[0; 19])]] {
        let storage = Arc::new(MemoryPersistence::default());
        storage.preload(
            class,
            NotificationClassSnapshot {
                recipient_list: Some(saved),
            },
        );
        assert!(NotificationClass::with_persistence(1, "NC", storage).is_err());
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_file() -> PathBuf {
    let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!(
            "rusty-bacnet-notification-class-{}-{serial}",
            std::process::id()
        ))
        .join("recipients")
}

#[test]
fn file_persistence_round_trips_and_refuses_another_objects_file() {
    let path = temp_file();
    let storage = FileNotificationClassPersistence::new(&path).unwrap();
    assert_eq!(storage.path(), path);
    let class = ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap();
    assert_eq!(storage.load(class).unwrap(), None);
    let snapshot = NotificationClassSnapshot {
        recipient_list: Some(vec![
            make_dest_device(3),
            address_destination(&[1, 2, 3, 4, 0xBA, 0xC0]),
        ]),
    };
    // The list may be empty, or absent.
    for saved in [
        snapshot.clone(),
        NotificationClassSnapshot {
            recipient_list: Some(Vec::new()),
        },
        NotificationClassSnapshot::default(),
    ] {
        storage.save(class, &saved).unwrap();
        assert_eq!(storage.load(class).unwrap(), Some(saved));
    }

    storage.save(class, &snapshot).unwrap();
    let other = ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 2).unwrap();
    assert!(storage.load(other).is_err());
    let good = std::fs::read(&path).unwrap();
    let mut truncated = good.clone();
    truncated.truncate(good.len() - 1);
    // After the 12-octet header: a presence octet other than 0 or 1, an
    // absent list followed by octets, and no presence octet at all.
    let mut unknown = good.clone();
    unknown[12] = 2;
    let mut absent = good.clone();
    absent[12] = 0;
    let header_only = good[..12].to_vec();
    for bad in [truncated, unknown, absent, header_only] {
        std::fs::write(&path, &bad).unwrap();
        assert!(storage.load(class).is_err());
    }
    std::fs::write(&path, b"not a notification class file").unwrap();
    assert!(storage.load(class).is_err());
    // A Notification Forwarder's file is another format.
    let forwarder =
        crate::notification_forwarder::FileNotificationForwarderPersistence::new(&path).unwrap();
    crate::notification_forwarder::NotificationForwarderPersistence::save(
        &forwarder,
        class,
        &crate::notification_forwarder::ForwarderSnapshot::default(),
    )
    .unwrap();
    let refusal = storage.load(class).unwrap_err().to_string();
    assert!(refusal.contains("has no valid header"), "{refusal}");
    // And the other way round: a forwarder backend refuses a class's file.
    storage.save(class, &snapshot).unwrap();
    let refusal =
        crate::notification_forwarder::NotificationForwarderPersistence::load(&forwarder, class)
            .unwrap_err()
            .to_string();
    assert!(refusal.contains("has no valid header"), "{refusal}");

    assert!(FileNotificationClassPersistence::new("").is_err());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn file_persistence_keeps_a_class_list_across_a_rebuild() {
    let path = temp_file();
    let storage = || {
        Arc::new(FileNotificationClassPersistence::new(&path).unwrap())
            as Arc<dyn NotificationClassPersistence>
    };
    let mut nc = NotificationClass::with_persistence(4, "NC", storage()).unwrap();
    nc.add_destination(make_dest_device(1)).unwrap();
    write(&mut nc, &[make_dest_device(2)]).unwrap();
    drop(nc);
    let mut rebuilt = NotificationClass::with_persistence(4, "NC", storage()).unwrap();
    rebuilt.add_destination(make_dest_device(1)).unwrap();
    assert_eq!(rebuilt.recipient_list(), [make_dest_device(2)]);
    assert!(rebuilt.recipient_list_saved());
    // No temporary file is left beside the list.
    assert!(!path.with_file_name("recipients.tmp").exists());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn file_persistence_refuses_a_file_past_its_size_or_entry_caps() {
    use crate::notification_class::persistence::MAX_FILE_BYTES;
    let path = temp_file();
    let storage = FileNotificationClassPersistence::new(&path).unwrap();
    let class = ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap();
    let refusal =
        |storage: &FileNotificationClassPersistence| storage.load(class).unwrap_err().to_string();
    // A list at the cap loads.
    let destinations: Vec<_> = (0..MAX_RECIPIENT_LIST_DESTINATIONS as u32)
        .map(make_dest_device)
        .collect();
    let full = NotificationClassSnapshot {
        recipient_list: Some(destinations.clone()),
    };
    storage.save(class, &full).unwrap();
    assert_eq!(storage.load(class).unwrap(), Some(full.clone()));

    // One destination past the cap is refused. The backend saves what it is
    // given, so a file like this can only come from elsewhere.
    let mut too_many = destinations;
    too_many.push(make_dest_device(999));
    storage
        .save(
            class,
            &NotificationClassSnapshot {
                recipient_list: Some(too_many),
            },
        )
        .unwrap();
    assert!(refusal(&storage).contains("more entries than the cap"));

    // A file past the size cap is refused before any of it is decoded; one
    // at the cap is read, and here refused for what it holds.
    storage.save(class, &full).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.resize(usize::try_from(MAX_FILE_BYTES).unwrap() + 1, 0);
    std::fs::write(&path, &bytes).unwrap();
    assert!(refusal(&storage).contains("too large"));
    bytes.pop();
    std::fs::write(&path, &bytes).unwrap();
    let at_cap = refusal(&storage);
    assert!(!at_cap.contains("too large"), "{at_cap}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
