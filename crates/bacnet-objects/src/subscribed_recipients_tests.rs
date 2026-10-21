//! Subscribed_Recipients storage (#1049): whole-list writes, the minute
//! countdown, expiry at the deadline, the range and size limits, and the
//! deadlines a rewrite keeps.

use super::*;
use crate::common::assert_list_element_refused;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use std::sync::atomic::{AtomicU64, Ordering};

const MINUTE: Duration = Duration::from_secs(60);
const NANO: Duration = Duration::from_nanos(1);

fn device(instance: u32, minutes: u32) -> BACnetEventNotificationSubscription {
    BACnetEventNotificationSubscription {
        recipient: BACnetRecipient::Device(
            ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        ),
        process_identifier: instance,
        issue_confirmed_notifications: false,
        time_remaining: minutes,
    }
}

fn address(minutes: u32) -> BACnetEventNotificationSubscription {
    BACnetEventNotificationSubscription {
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 5,
            mac_address: bacnet_types::MacAddr::from_slice(&[10, 0, 0, 9, 0xBA, 0xC0]),
        }),
        process_identifier: 77,
        issue_confirmed_notifications: true,
        time_remaining: minutes,
    }
}

fn framed(subscriptions: &[BACnetEventNotificationSubscription]) -> PropertyValue {
    let mut buf = BytesMut::new();
    encode_event_notification_subscription_list(&mut buf, subscriptions).unwrap();
    PropertyValue::ApplicationData(buf.to_vec())
}

fn stored(subscriptions: &[BACnetEventNotificationSubscription]) -> SubscribedRecipients {
    let mut store = SubscribedRecipients::new();
    store.write(framed(subscriptions)).unwrap();
    store
}

/// Minutes left per entry, in list order.
fn minutes(store: &SubscribedRecipients) -> Vec<u32> {
    store
        .subscriptions()
        .iter()
        .map(|subscription| subscription.time_remaining)
        .collect()
}

#[test]
fn written_entries_read_back_in_order() {
    let entries = [device(1, 60), address(1440)];
    let store = stored(&entries);
    assert_eq!(store.subscriptions(), entries);
    assert_eq!(store.read(), framed(&entries));
    assert_eq!(SubscribedRecipients::new().read(), framed(&[]));
}

#[test]
fn time_remaining_counts_down_in_whole_minutes_rounded_up() {
    let mut store = stored(&[device(1, 2)]);
    assert_eq!(minutes(&store), [2]);
    store.advance_by(NANO);
    assert_eq!(minutes(&store), [2], "part of a minute rounds up");
    store.advance_by(MINUTE - NANO);
    assert_eq!(minutes(&store), [1], "one whole minute left");
    store.advance_by(MINUTE - NANO);
    assert_eq!(minutes(&store), [1], "a live entry never reads as zero");
}

#[test]
fn an_entry_lapses_at_its_deadline() {
    let mut store = stored(&[device(1, 1), device(2, 2)]);
    assert_eq!(store.next_deadline(), Some(MINUTE));
    assert!(!store.advance_by(MINUTE - NANO));
    assert_eq!(minutes(&store), [1, 2]);
    assert!(store.advance_by(NANO), "the first entry lapses");
    assert_eq!(store.subscriptions(), [device(2, 1)]);
    assert_eq!(store.next_deadline(), Some(2 * MINUTE));
    assert!(store.advance_by(MINUTE));
    assert_eq!(store.read(), framed(&[]));
    assert_eq!(store.next_deadline(), None);
}

#[test]
fn a_bound_clock_drives_reads_and_expiry() {
    let nanos = Arc::new(AtomicU64::new(0));
    let clock = Arc::clone(&nanos);
    let mut store = SubscribedRecipients::new();
    store.bind_monotonic_clock(Some(Arc::new(move || {
        Duration::from_nanos(clock.load(Ordering::SeqCst))
    })));
    let set = |at: Duration| nanos.store(u64::try_from(at.as_nanos()).unwrap(), Ordering::SeqCst);
    set(10 * MINUTE);
    store.write(framed(&[device(1, 3)])).unwrap();
    assert_eq!(store.next_deadline(), Some(13 * MINUTE));
    set(11 * MINUTE + NANO);
    assert_eq!(minutes(&store), [2]);
    // A read skips a lapsed entry before the operation task drops it.
    set(13 * MINUTE);
    assert_eq!(store.read(), framed(&[]));
    assert!(store.advance_to(13 * MINUTE));
    assert_eq!(store.next_deadline(), None);
}

#[test]
fn unbinding_the_clock_keeps_the_time_each_entry_has_left() {
    let nanos = Arc::new(AtomicU64::new(0));
    let clock = Arc::clone(&nanos);
    let mut store = SubscribedRecipients::new();
    store.bind_monotonic_clock(Some(Arc::new(move || {
        Duration::from_nanos(clock.load(Ordering::SeqCst))
    })));
    nanos.store(
        u64::try_from((10 * MINUTE).as_nanos()).unwrap(),
        Ordering::SeqCst,
    );
    store.write(framed(&[device(1, 5)])).unwrap();
    nanos.store(
        u64::try_from((12 * MINUTE).as_nanos()).unwrap(),
        Ordering::SeqCst,
    );
    assert_eq!(minutes(&store), [3]);
    // Back on counted time, the entry still has three minutes left.
    store.bind_monotonic_clock(None);
    assert_eq!(minutes(&store), [3]);
    assert!(!store.advance_by(3 * MINUTE - NANO));
    assert_eq!(minutes(&store), [1]);
    assert!(store.advance_by(NANO));
    assert_eq!(store.subscriptions(), []);
}

#[test]
fn time_remaining_must_be_one_minute_to_a_day() {
    let mut store = stored(&[device(1, 5)]);
    for refused in [0, MAX_SUBSCRIPTION_MINUTES + 1, u32::MAX] {
        assert_list_element_refused(
            store.write(framed(&[device(2, 5), device(3, refused)])),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            2,
            &format!("{refused} minutes"),
        );
        assert_eq!(store.subscriptions(), [device(1, 5)], "nothing changes");
    }
    store
        .write(framed(&[device(2, 1), device(3, MAX_SUBSCRIPTION_MINUTES)]))
        .unwrap();
    assert_eq!(minutes(&store), [1, MAX_SUBSCRIPTION_MINUTES]);
}

#[test]
fn a_full_list_refuses_another_entry_but_not_a_renewal() {
    let full: Vec<_> = (1..=MAX_SUBSCRIBED_RECIPIENTS as u32)
        .map(|instance| device(instance, 5))
        .collect();
    let mut store = stored(&full);
    let mut over = full.clone();
    over.push(device(1, 9));
    over.push(device(999, 5));
    assert_list_element_refused(
        store.write(framed(&over)),
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        MAX_SUBSCRIBED_RECIPIENTS as u32 + 2,
        "the entry past the cap",
    );
    assert_eq!(store.subscriptions(), full, "nothing changes");
    over.pop();
    store.write(framed(&over)).unwrap();
    assert_eq!(store.subscriptions().len(), MAX_SUBSCRIBED_RECIPIENTS);
    assert_eq!(store.subscriptions()[0], device(1, 9), "renewed in place");
}

#[test]
fn entries_naming_the_same_recipient_and_process_are_one_entry() {
    let mut renewed = device(1, 10);
    renewed.issue_confirmed_notifications = true;
    let store = stored(&[device(1, 5), address(5), renewed.clone()]);
    assert_eq!(store.subscriptions(), [renewed, address(5)]);
    // The same device under another process identifier is another entry.
    let mut other_process = device(1, 5);
    other_process.process_identifier = 2;
    let store = stored(&[device(1, 5), other_process.clone()]);
    assert_eq!(store.subscriptions(), [device(1, 5), other_process]);
}

#[test]
fn a_rewrite_keeps_the_deadline_of_each_entry_it_leaves_alone() {
    let mut store = stored(&[device(1, 2)]);
    store.advance_by(MINUTE / 2);
    // Read back with 90 seconds left, the entry serves as 2 minutes.
    let mut list = store.subscriptions();
    assert_eq!(list, [device(1, 2)]);
    list.push(device(2, 3));
    store.write(framed(&list)).unwrap();
    assert_eq!(store.next_deadline(), Some(2 * MINUTE), "not stretched");
    assert!(store.advance_by(MINUTE + MINUTE / 2));
    assert_eq!(store.subscriptions(), [device(2, 2)]);
}

#[test]
fn a_rewrite_keeps_a_deadline_when_a_minute_boundary_falls_between_read_and_write() {
    // Entry with 2 minutes. Read 30 seconds in: it serves 2. The next minute
    // boundary of its countdown is at 60 seconds; the write lands after it.
    let mut store = stored(&[device(1, 2)]);
    store.advance_by(MINUTE / 2);
    let mut list = store.subscriptions();
    assert_eq!(list, [device(1, 2)]);
    list.push(device(2, 3));
    store.advance_by(MINUTE / 2 + NANO);
    assert_eq!(minutes(&store), [1], "the boundary has passed");
    store.write(framed(&list)).unwrap();
    assert_eq!(store.next_deadline(), Some(2 * MINUTE), "not stretched");
    assert_eq!(minutes(&store), [1, 3]);
}

#[test]
fn a_rewrite_restarts_an_entry_written_two_minutes_or_more_off() {
    let mut store = stored(&[device(1, 5)]);
    store.advance_by(MINUTE / 2);
    assert_eq!(minutes(&store), [5]);
    for written in [3, 7] {
        let mut store = store.clone();
        store.write(framed(&[device(1, written)])).unwrap();
        assert_eq!(
            store.next_deadline(),
            Some(MINUTE / 2 + MINUTE * written),
            "{written}"
        );
    }
}

#[test]
fn a_rewrite_with_other_members_restarts_the_lifetime() {
    let mut store = stored(&[device(1, 2), device(2, 2)]);
    store.advance_by(MINUTE / 2);
    let mut confirmed = device(2, 2);
    confirmed.issue_confirmed_notifications = true;
    store
        .write(framed(&[device(1, 5), confirmed.clone()]))
        .unwrap();
    // Both restart at the write, 30 seconds in.
    assert_eq!(store.subscriptions(), [device(1, 5), confirmed]);
    assert_eq!(store.next_deadline(), Some(MINUTE / 2 + 2 * MINUTE));
}

#[test]
fn only_a_framed_list_of_whole_entries_is_written() {
    let mut store = stored(&[device(1, 5)]);
    let PropertyValue::ApplicationData(bytes) = framed(&[device(2, 5)]) else {
        unreachable!()
    };
    for refused in [
        PropertyValue::List(vec![PropertyValue::Unsigned(5)]),
        PropertyValue::Unsigned(5),
        PropertyValue::ApplicationData(bytes[..bytes.len() - 1].to_vec()),
        PropertyValue::ApplicationData([&bytes[..], &[0x21, 0x05]].concat()),
    ] {
        let result = store.write(refused.clone());
        assert!(
            matches!(
                result,
                Err(Error::Protocol { class, code })
                    if class == ErrorClass::PROPERTY
                        && code == ErrorCode::INVALID_DATA_TYPE
            ),
            "{refused:?}: {result:?}"
        );
        assert_eq!(store.subscriptions(), [device(1, 5)], "nothing changes");
    }
}

#[test]
fn a_full_list_of_the_longest_entries_reads_in_1184_octets() {
    let longest: Vec<_> = (0..MAX_SUBSCRIBED_RECIPIENTS)
        .map(|index| BACnetEventNotificationSubscription {
            recipient: BACnetRecipient::Address(BACnetAddress {
                network_number: u16::MAX,
                mac_address: bacnet_types::MacAddr::from_slice(
                    &[index as u8; BACnetAddress::MAX_MAC_LEN],
                ),
            }),
            process_identifier: u32::MAX,
            issue_confirmed_notifications: true,
            time_remaining: MAX_SUBSCRIPTION_MINUTES,
        })
        .collect();
    let PropertyValue::ApplicationData(bytes) = stored(&longest).read() else {
        unreachable!()
    };
    assert_eq!(bytes.len(), 1184);
}
