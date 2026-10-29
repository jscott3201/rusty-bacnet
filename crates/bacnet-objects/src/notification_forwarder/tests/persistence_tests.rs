//! Subscribed_Recipients across a restart (Clause 12.51.9).

use super::storage::{MemoryPersistence, persistent};
use super::*;
use bacnet_types::enums::PropertyIdentifier as P;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn write(nf: &mut NotificationForwarderObject, list: &[BACnetEventNotificationSubscription]) {
    nf.write_property(
        P::SUBSCRIBED_RECIPIENTS,
        None,
        framed_subscriptions(list),
        None,
    )
    .unwrap();
}

fn minutes(nf: &NotificationForwarderObject) -> Vec<u32> {
    nf.subscriptions()
        .iter()
        .map(|entry| entry.time_remaining)
        .collect()
}

/// Run the operation task's call at `at` and let the save it queues land.
fn tick(nf: &mut NotificationForwarderObject, at: Duration) -> bool {
    let lapsed = nf.advance_monotonic_time_internal(at);
    nf.wait_for_saves();
    lapsed
}

#[test]
fn subscribed_recipients_restart_lands_between_the_time_left_and_the_last_subscription() {
    let storage = Arc::new(MemoryPersistence::default());
    let (clock, set) = manual_clock();
    let mut before = persistent(&storage);
    before.bind_monotonic_clock_internal(Some(Arc::clone(&clock)));
    let a = |minutes| subscription(device(7), 1, minutes);
    let b = |minutes| subscription(device(8), 1, minutes);
    let c = |minutes| subscription(address(0, &[9]), 2, minutes);

    write(&mut before, &[a(10), b(30)]);
    assert_eq!(storage.saved(), [a(10), b(30)]);
    // Four minutes on, a third entry is added; the two held ones keep their
    // deadlines, and each saved entry carries the minutes it has left.
    set(4 * MINUTE);
    write(&mut before, &[a(6), b(26), c(20)]);
    assert_eq!(storage.saved(), [a(6), b(26), c(20)]);
    // Three more minutes pass with no change, then the device stops.
    set(7 * MINUTE);
    let at_stop = minutes(&before);
    assert_eq!(at_stop, [3, 23, 17]);
    drop(before);

    let mut after = persistent(&storage);
    after.bind_monotonic_clock_internal(Some(clock));
    let restored = minutes(&after);
    let last_subscribed = [10, 30, 20];
    for ((restored, stopped), subscribed) in restored.iter().zip(at_stop).zip(last_subscribed) {
        assert!(
            (stopped..=subscribed).contains(restored),
            "{restored} minutes must lie between {stopped} and {subscribed}"
        );
    }
    assert_eq!(restored, [6, 26, 20]);

    // An entry that lapses after the restart leaves the saved copy too.
    set(13 * MINUTE);
    assert!(tick(&mut after, 13 * MINUTE));
    assert_eq!(storage.saved(), [b(20), c(14)]);
}

#[test]
fn subscribed_recipients_write_that_cannot_be_saved_is_refused() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut nf = persistent(&storage);
    write(&mut nf, &[subscription(device(7), 1, 10)]);
    let counters = nf.save_counters();
    storage.fail.store(true, Ordering::SeqCst);
    assert_refused(
        nf.write_property(
            P::SUBSCRIBED_RECIPIENTS,
            None,
            framed_subscriptions(&[subscription(device(8), 1, 10)]),
            None,
        ),
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM,
    );
    assert_eq!(nf.subscriptions(), [subscription(device(7), 1, 10)]);
    assert_eq!(storage.saved(), [subscription(device(7), 1, 10)]);
    assert_eq!(counters.failed_saves(), 1);
}

/// One boot of a forwarder on `storage`: a fresh monotonic clock from zero,
/// the operation task's calls every ten seconds for `run`, then a stop.
/// Returns the minutes the entry served when the forwarder was built.
fn boot(
    storage: &Arc<MemoryPersistence>,
    first_write: Option<&[BACnetEventNotificationSubscription]>,
    run: Duration,
) -> u32 {
    let (clock, set) = manual_clock();
    let mut nf = persistent(storage);
    nf.bind_monotonic_clock_internal(Some(clock));
    if let Some(list) = first_write {
        write(&mut nf, list);
    }
    let restored = minutes(&nf)[0];
    let mut at = Duration::ZERO;
    while at < run {
        at += Duration::from_secs(10);
        set(at);
        nf.advance_monotonic_time_internal(at);
    }
    // Dropping the forwarder lets its queued saves land.
    restored
}

#[test]
fn subscribed_recipients_keep_running_out_across_repeated_restarts() {
    let storage = Arc::new(MemoryPersistence::default());
    let entry = [subscription(device(7), 1, 1440)];
    // Each boot runs ten minutes. The saved copy follows the falling
    // minutes, so every restart restores less than the one before.
    let restored = [
        boot(&storage, Some(&entry), 10 * MINUTE),
        boot(&storage, None, 10 * MINUTE),
        boot(&storage, None, 10 * MINUTE),
    ];
    assert_eq!(restored, [1440, 1430, 1420]);
    assert_eq!(storage.saved(), [subscription(device(7), 1, 1410)]);
}

#[test]
fn subscribed_recipients_minute_saves_come_at_most_once_a_minute() {
    let storage = Arc::new(MemoryPersistence::default());
    let (clock, set) = manual_clock();
    let mut nf = persistent(&storage);
    nf.bind_monotonic_clock_internal(Some(clock));
    // Two entries whose minutes fall half a minute apart.
    write(&mut nf, &[subscription(device(7), 1, 10)]);
    set(MINUTE / 2);
    write(
        &mut nf,
        &[
            subscription(device(7), 1, 10),
            subscription(device(8), 1, 10),
        ],
    );
    set(MINUTE);
    tick(&mut nf, MINUTE);
    assert_eq!(
        storage.saved(),
        [
            subscription(device(7), 1, 9),
            subscription(device(8), 1, 10)
        ]
    );
    // The second entry's minute falls at 90 s, inside the minute since the
    // last save: nothing is saved until 120 s.
    let ninety = MINUTE + MINUTE / 2;
    set(ninety);
    tick(&mut nf, ninety);
    assert_eq!(
        storage.saved(),
        [
            subscription(device(7), 1, 9),
            subscription(device(8), 1, 10)
        ]
    );
    set(2 * MINUTE);
    tick(&mut nf, 2 * MINUTE);
    assert_eq!(
        storage.saved(),
        [subscription(device(7), 1, 8), subscription(device(8), 1, 9)]
    );
}

#[test]
fn a_failed_save_is_counted_and_retried_a_minute_later() {
    let storage = Arc::new(MemoryPersistence::default());
    let (clock, set) = manual_clock();
    let mut nf = persistent(&storage);
    let counters = nf.save_counters();
    nf.bind_monotonic_clock_internal(Some(clock));
    write(&mut nf, &[subscription(device(7), 1, 10)]);
    storage.fail.store(true, Ordering::SeqCst);
    set(MINUTE);
    tick(&mut nf, MINUTE);
    assert_eq!(counters.failed_saves(), 1);
    assert_eq!(storage.saved(), [subscription(device(7), 1, 10)]);

    // Storage is back; the retry waits for the minute to pass.
    storage.fail.store(false, Ordering::SeqCst);
    let ninety = MINUTE + MINUTE / 2;
    set(ninety);
    tick(&mut nf, ninety);
    assert_eq!(storage.saved(), [subscription(device(7), 1, 10)]);
    set(2 * MINUTE);
    tick(&mut nf, 2 * MINUTE);
    assert_eq!(storage.saved(), [subscription(device(7), 1, 8)]);
    assert_eq!(counters.failed_saves(), 1);
}

#[test]
fn subscribed_recipients_saved_out_of_range_refuses_the_forwarder() {
    let storage = Arc::new(MemoryPersistence::default());
    storage.preload(
        ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, 1).unwrap(),
        ForwarderSnapshot {
            recipient_list: None,
            subscribed_recipients: vec![subscription(device(7), 1, 0)],
        },
    );
    assert!(NotificationForwarderObject::with_persistence(1, "NF", storage).is_err());
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_file() -> PathBuf {
    let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!(
            "rusty-bacnet-forwarder-{}-{serial}",
            std::process::id()
        ))
        .join("lists")
}

#[test]
fn file_persistence_round_trips_and_refuses_another_objects_file() {
    let path = temp_file();
    let storage = FileNotificationForwarderPersistence::new(&path).unwrap();
    let forwarder = ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, 1).unwrap();
    assert_eq!(storage.load(forwarder).unwrap(), None);
    let snapshot = ForwarderSnapshot {
        recipient_list: Some(vec![
            destination(device(3), 4, true),
            destination(address(5, &[1, 2, 3, 4, 0xBA, 0xC0]), 0, false),
        ]),
        subscribed_recipients: vec![
            subscription(device(7), 1, 10),
            subscription(address(5, &[1, 2, 3, 4, 0xBA, 0xC0]), 9, 1440),
        ],
    };
    storage.save(forwarder, &snapshot).unwrap();
    assert_eq!(storage.load(forwarder).unwrap().unwrap(), snapshot);
    // Either list may be empty, and Recipient_List absent.
    let lists_alone = [
        ForwarderSnapshot {
            recipient_list: Some(Vec::new()),
            ..snapshot.clone()
        },
        ForwarderSnapshot {
            recipient_list: None,
            ..snapshot.clone()
        },
        ForwarderSnapshot {
            subscribed_recipients: Vec::new(),
            ..snapshot.clone()
        },
        ForwarderSnapshot::default(),
    ];
    for alone in lists_alone {
        storage.save(forwarder, &alone).unwrap();
        assert_eq!(storage.load(forwarder).unwrap().unwrap(), alone);
    }

    storage.save(forwarder, &snapshot).unwrap();
    let other = ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, 2).unwrap();
    assert!(storage.load(other).is_err());
    let good = std::fs::read(&path).unwrap();
    let mut truncated = good.clone();
    truncated.truncate(good.len() - 1);
    std::fs::write(&path, &truncated).unwrap();
    assert!(storage.load(forwarder).is_err());
    // A Recipient_List length past the end of the file, a presence octet
    // other than 0 or 1, and an absent list with a length.
    let mut overlong = good.clone();
    overlong[13..17].copy_from_slice(&u32::MAX.to_be_bytes());
    let mut unknown = good.clone();
    unknown[12] = 2;
    let mut absent = good;
    absent[12] = 0;
    for bad in [overlong, unknown, absent] {
        std::fs::write(&path, &bad).unwrap();
        assert!(storage.load(forwarder).is_err());
    }
    std::fs::write(&path, b"not a forwarder file").unwrap();
    assert!(storage.load(forwarder).is_err());

    assert!(FileNotificationForwarderPersistence::new("").is_err());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn file_persistence_keeps_a_forwarder_list_across_a_rebuild() {
    let path = temp_file();
    let storage = || {
        Arc::new(FileNotificationForwarderPersistence::new(&path).unwrap())
            as Arc<dyn NotificationForwarderPersistence>
    };
    let mut nf = NotificationForwarderObject::with_persistence(4, "NF", storage()).unwrap();
    write(&mut nf, &[subscription(device(7), 3, 15)]);
    nf.write_property(
        P::RECIPIENT_LIST,
        None,
        framed_destinations(&[destination(device(2), 6, false)]),
        None,
    )
    .unwrap();
    drop(nf);
    let rebuilt = NotificationForwarderObject::with_persistence(4, "NF", storage()).unwrap();
    assert_eq!(rebuilt.subscriptions(), [subscription(device(7), 3, 15)]);
    assert_eq!(rebuilt.recipient_list(), [destination(device(2), 6, false)]);
    assert!(rebuilt.recipient_list_saved());
    // No temporary file is left beside the list.
    assert!(!path.with_file_name("lists.tmp").exists());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn file_persistence_refuses_a_file_past_its_size_or_entry_caps() {
    use crate::notification_forwarder::persistence::MAX_FILE_BYTES;
    use crate::subscribed_recipients::MAX_SUBSCRIBED_RECIPIENTS;
    let path = temp_file();
    let storage = FileNotificationForwarderPersistence::new(&path).unwrap();
    let forwarder = ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, 1).unwrap();
    let refusal = |storage: &FileNotificationForwarderPersistence| {
        storage.load(forwarder).unwrap_err().to_string()
    };
    // Both lists at their caps load.
    let destinations: Vec<_> = (0..MAX_RECIPIENT_LIST_DESTINATIONS as u32)
        .map(|n| destination(device(n), n, false))
        .collect();
    let subscriptions: Vec<_> = (0..MAX_SUBSCRIBED_RECIPIENTS as u32)
        .map(|n| subscription(device(n), n, 10))
        .collect();
    let full = ForwarderSnapshot {
        recipient_list: Some(destinations.clone()),
        subscribed_recipients: subscriptions.clone(),
    };
    storage.save(forwarder, &full).unwrap();
    assert_eq!(storage.load(forwarder).unwrap().unwrap(), full);

    // One entry past either cap is refused. The backend saves what it is
    // given, so a file like this can only come from elsewhere.
    let mut too_many = destinations;
    too_many.push(destination(device(999), 1, false));
    let past_recipient_cap = ForwarderSnapshot {
        recipient_list: Some(too_many),
        ..full.clone()
    };
    let mut too_many = subscriptions;
    too_many.push(subscription(device(999), 1, 10));
    let past_subscription_cap = ForwarderSnapshot {
        subscribed_recipients: too_many,
        ..full.clone()
    };
    for past_cap in [past_recipient_cap, past_subscription_cap] {
        storage.save(forwarder, &past_cap).unwrap();
        assert!(refusal(&storage).contains("more entries than the cap"));
    }

    // A file past the size cap is refused before any of it is decoded; one
    // at the cap is read, and here refused for what it holds.
    storage.save(forwarder, &full).unwrap();
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
