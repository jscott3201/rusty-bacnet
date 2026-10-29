//! The hub against an in-process lab device over a loopback link. What the
//! hub holds is checked against the device's own subscription table
//! (Active_COV_Subscriptions), not only the hub's counters.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bacnet_client::client::{CovTarget, ReceivedCOVNotification};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::shared::Registry;
use super::test_lab::{LAB, Lab, ai, object, oid, settle};
use super::*;
use crate::format::decode_all;

/// The Present_Value among a snapshot's rows.
pub(super) fn present_value(snapshot: &Snapshot) -> Option<f32> {
    let (_, value) = snapshot
        .rows
        .iter()
        .find(|(p, _)| *p == PropertyIdentifier::PRESENT_VALUE)?;
    match decode_all(value.as_ref().ok()?)?.as_slice() {
        [PropertyValue::Real(value)] => Some(*value),
        _ => None,
    }
}

/// Acceptance: two listeners on one point make one BACnet subscription,
/// finite, and the last one leaving cancels it, as the device's own table
/// shows.
#[tokio::test(start_paused = true)]
async fn two_leases_share_one_subscription_until_the_last_goes() {
    let lab = Lab::start(WatchOptions::default()).await;
    let first = lab.hub.watch(object(ai(1))).unwrap();
    let second = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    let table = lab.table().await;
    assert_eq!(table.len(), 1, "{table:?}");
    let entry = &table[0];
    assert!(entry.time_remaining > 0 && entry.time_remaining <= 300);
    assert_eq!(entry.monitored_property_reference.object_identifier, ai(1));
    let snapshot = first.snapshot();
    assert_eq!(snapshot.source, Some(Source::Cov));
    assert_eq!(snapshot.state, State::Live);
    assert_eq!(present_value(&snapshot), Some(10.0));
    assert_eq!(lab.hub.stats().points, 1);
    drop(first);
    settle().await;
    assert_eq!(lab.table().await.len(), 1, "one listener still holds it");
    drop(second);
    settle().await;
    assert!(
        lab.table().await.is_empty(),
        "the last listener cancelled it"
    );
    assert_eq!(lab.hub.stats().tasks, 0);
    lab.stop().await;
}

/// No lifetime-0 subscription can be built: the type refuses zero, and
/// the class table calls zero a write.
#[test]
fn no_lifetime_is_indefinite() {
    assert!(Lifetime::new(0).is_none());
    assert_eq!(Lifetime::new(1).unwrap().secs(), 1);
    assert_eq!(class::of_subscribe(Lifetime::DEFAULT.secs()), Class::Read);
    assert_eq!(class::of_subscribe(0), Class::Write);
    let fast = WatchOptions {
        poll_every: Duration::from_secs(9),
        ..WatchOptions::default()
    };
    assert!(fast.validate().is_err(), "polls go 10 s apart or slower");
}

/// Acceptance: a device that refuses COV for an object is polled, every
/// 10 s, and the snapshot says so.
#[tokio::test(start_paused = true)]
async fn a_refused_subscription_falls_back_to_polling() {
    let lab = Lab::start(WatchOptions::default()).await;
    let class = lab
        .hub
        .watch(object(oid(ObjectType::NOTIFICATION_CLASS, 0)))
        .unwrap();
    settle().await;
    let snapshot = class.snapshot();
    assert_eq!(snapshot.source, Some(Source::Poll), "{snapshot:?}");
    assert!(
        snapshot.note.as_deref().unwrap().starts_with("COV refused"),
        "{snapshot:?}"
    );
    assert!(
        matches!(snapshot.why, Some(Why::Refused(_))),
        "{snapshot:?}"
    );
    assert!(lab.table().await.is_empty());
    tokio::time::sleep(Duration::from_secs(25)).await;
    assert_eq!(class.snapshot().polls, 2, "a poll every 10 s");
    assert_eq!(lab.hub.stats().polled, 1);
    lab.stop().await;
}

/// Acceptance: a burst of notifications makes few signals, the folded ones
/// are counted, and a read afterwards has the latest value.
#[tokio::test(start_paused = true)]
async fn a_notification_storm_is_coalesced() {
    let lab = Lab::start(WatchOptions::default()).await;
    let mut lease = lab.hub.watch(object(ai(2))).unwrap();
    settle().await;
    let before = lease.snapshot().seq;
    for step in 0..40u8 {
        lab.server
            .set_present_value_local(&ai(2), PropertyValue::Real(f32::from(step) + 20.0))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let snapshot = lease.snapshot();
    assert_eq!(present_value(&snapshot), Some(59.0), "the latest value");
    assert!(snapshot.notifications >= 40, "{snapshot:?}");
    assert!(snapshot.coalesced >= 30, "{snapshot:?}");
    assert!(snapshot.seq > before);
    // The listener sees one pending signal, not forty.
    lease.changed().await;
    let pending = tokio::time::timeout(Duration::from_millis(1), lease.changed()).await;
    assert!(pending.is_err(), "no second signal is queued");
    lab.stop().await;
}

/// Renewal goes before the lifetime ends, and the device's table shows the
/// lifetime restarted.
#[tokio::test(start_paused = true)]
async fn a_subscription_is_renewed_before_it_lapses() {
    let lab = Lab::start(WatchOptions::default()).await;
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    tokio::time::sleep(Duration::from_secs(239)).await;
    assert_eq!(lease.snapshot().renewals, 0);
    let remaining = lab.table().await[0].time_remaining;
    assert!(remaining <= 62, "{remaining}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(lease.snapshot().renewals, 1);
    let remaining = lab.table().await[0].time_remaining;
    assert!(remaining >= 298, "{remaining}");
    lab.stop().await;
}

/// A device that goes silent fails its renewal: the watch says it is
/// unreachable and polls, backing off, and subscribes again once the
/// device answers.
#[tokio::test(start_paused = true)]
async fn a_silent_device_is_reported() {
    let lab = Lab::start(WatchOptions::default()).await;
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    lab.quirks.muted.store(true, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(245)).await;
    let snapshot = lease.snapshot();
    assert_eq!(snapshot.state, State::Unreachable, "{snapshot:?}");
    assert_eq!(snapshot.source, Some(Source::Poll));
    // The poll that followed failed too: its error is the note, while the
    // reason the point is polled stays the renewal's.
    assert!(
        matches!(snapshot.why, Some(Why::Lapsed(Some(_)))),
        "{snapshot:?}"
    );
    assert!(
        snapshot.polls == 0 && !snapshot.note.as_deref().unwrap().contains("renewal"),
        "{snapshot:?}"
    );
    lab.quirks.muted.store(false, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(320)).await;
    let snapshot = lease.snapshot();
    assert_eq!(snapshot.source, Some(Source::Cov), "{snapshot:?}");
    assert_eq!(snapshot.state, State::Live);
    lab.stop().await;
}

/// SubscribeCOVProperty for a property watch.
#[tokio::test(start_paused = true)]
async fn a_property_watch_subscribes_to_the_property() {
    let lab = Lab::start(WatchOptions::default()).await;
    let point = Point {
        device: LAB,
        object: ai(3),
        watched: Watched::Property {
            property: PropertyIdentifier::PRESENT_VALUE,
            index: None,
        },
    };
    let lease = lab.hub.watch(point).unwrap();
    settle().await;
    let table = lab.table().await;
    assert_eq!(table.len(), 1);
    assert_eq!(
        table[0].monitored_property_reference.property_identifier,
        PropertyIdentifier::PRESENT_VALUE.to_raw()
    );
    lab.server
        .set_present_value_local(&ai(3), PropertyValue::Real(33.0))
        .await
        .unwrap();
    settle().await;
    let snapshot = lease.snapshot();
    assert_eq!(present_value(&snapshot), Some(33.0));
    assert_eq!(snapshot.rows.len(), 1, "only the watched property");
    drop(lease);
    settle().await;
    assert!(lab.table().await.is_empty());
    lab.stop().await;
}

/// The cap counts points still cancelling; a listener joining as the last
/// one leaves gets a fresh entry, and only its subscription remains.
#[tokio::test(start_paused = true)]
async fn the_cap_holds_and_a_rejoin_gets_a_fresh_entry() {
    let options = WatchOptions {
        max_points: 2,
        ..WatchOptions::default()
    };
    let lab = Lab::start(options).await;
    let one = lab.hub.watch(object(ai(1))).unwrap();
    let _two = lab.hub.watch(object(ai(2))).unwrap();
    assert_eq!(
        lab.hub.watch(object(ai(3))).unwrap_err(),
        WatchError::Full { max: 2 }
    );
    settle().await;
    drop(one);
    // Still cancelling: the cap counts it.
    assert!(lab.hub.watch(object(ai(3))).is_err());
    let again = lab.hub.watch(object(ai(1)));
    assert!(again.is_err(), "the old entry is still cancelling");
    settle().await;
    let again = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    let table = lab.table().await;
    assert_eq!(table.len(), 2, "{table:?}");
    assert_eq!(again.snapshot().source, Some(Source::Cov));
    lab.stop().await;
}

/// Shutdown cancels every subscription and refuses new watches.
#[tokio::test(start_paused = true)]
async fn shutdown_cancels_every_subscription() {
    let lab = Lab::start(WatchOptions::default()).await;
    let a = lab.hub.watch(object(ai(1))).unwrap();
    let b = lab.hub.watch(object(ai(2))).unwrap();
    settle().await;
    assert_eq!(lab.table().await.len(), 2);
    assert!(lab.hub.shutdown(Duration::from_secs(30)).await);
    assert!(lab.table().await.is_empty());
    assert_eq!(a.snapshot().state, State::Stopped);
    assert_eq!(lab.hub.watch(object(ai(3))).unwrap_err(), WatchError::Shut);
    drop((a, b));
    lab.stop().await;
}

/// A listener joining as the last one leaves, in the same instant: the old
/// entry cancels its own subscription, the new one keeps its own, and the
/// new lease goes on receiving notifications.
#[tokio::test(start_paused = true)]
async fn joining_as_the_last_leaves_keeps_one_live_subscription() {
    let lab = Lab::start(WatchOptions::default()).await;
    let old = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    drop(old);
    let new = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    let table = lab.table().await;
    assert_eq!(table.len(), 1, "{table:?}");
    assert_eq!(new.snapshot().source, Some(Source::Cov));
    lab.server
        .set_present_value_local(&ai(1), PropertyValue::Real(42.0))
        .await
        .unwrap();
    settle().await;
    assert_eq!(present_value(&new.snapshot()), Some(42.0));
    lab.stop().await;
}

/// A notification for `point`'s object, from the lab device's MAC,
/// carrying `values` (property, index).
fn notification(
    object: ObjectIdentifier,
    values: &[(PropertyIdentifier, Option<u32>)],
) -> ReceivedCOVNotification {
    ReceivedCOVNotification {
        notification: bacnet_services::cov::COVNotificationRequest {
            subscriber_process_identifier: PROCESS_BASE,
            initiating_device_identifier: oid(ObjectType::DEVICE, LAB),
            monitored_object_identifier: object,
            time_remaining: 100,
            list_of_values: values
                .iter()
                .map(
                    |&(property, index)| bacnet_types::constructed::BACnetPropertyValue {
                        property_identifier: property,
                        property_array_index: index,
                        value: vec![0x44, 0x42, 0x28, 0, 0],
                        priority: None,
                    },
                )
                .collect(),
        },
        source_mac: bacnet_types::MacAddr::from_slice(&[2]),
        source_network: None,
        source_address: None,
        delivery: bacnet_client::client::COVNotificationDelivery::Unconfirmed,
    }
}

/// A device can't grow an object watch's rows by reporting properties it
/// doesn't watch; a property watch keeps only its property and element.
#[test]
fn notifications_cannot_grow_the_rows() {
    let point = object(ai(1));
    let mut rows = Vec::new();
    let flood: Vec<(PropertyIdentifier, Option<u32>)> = (0..500)
        .map(|n| (PropertyIdentifier::from_raw(600 + n), None))
        .chain([(PropertyIdentifier::PRESENT_VALUE, None)])
        .collect();
    dispatch::merge(
        &mut rows,
        &point,
        &POINT_PROPERTIES,
        false,
        &notification(ai(1), &flood),
    );
    assert_eq!(rows.len(), 1, "only the point property joins");
    for _ in 0..3 {
        dispatch::merge(
            &mut rows,
            &point,
            &POINT_PROPERTIES,
            false,
            &notification(ai(1), &flood),
        );
    }
    assert_eq!(rows.len(), 1);
    assert!(rows.len() <= MAX_ROWS);

    let element = Point {
        device: LAB,
        object: oid(ObjectType::ANALOG_OUTPUT, 1),
        watched: Watched::Property {
            property: PropertyIdentifier::PRIORITY_ARRAY,
            index: Some(8),
        },
    };
    let target = CovTarget::local(&[2]);
    let other = notification(
        element.object,
        &[(PropertyIdentifier::PRIORITY_ARRAY, Some(9))],
    );
    assert!(!dispatch::belongs(&element, Some(&target), &other));
    let mine = notification(
        element.object,
        &[(PropertyIdentifier::PRIORITY_ARRAY, Some(8))],
    );
    assert!(dispatch::belongs(&element, Some(&target), &mine));
    let mut rows = Vec::new();
    dispatch::merge(&mut rows, &element, &[], false, &other);
    assert!(rows.is_empty());
    dispatch::merge(&mut rows, &element, &[], false, &mine);
    assert_eq!(rows.len(), 1);
    // A whole array sent without an index isn't the element; one value
    // without an index is.
    let mut array = notification(
        element.object,
        &[(PropertyIdentifier::PRIORITY_ARRAY, None)],
    );
    array.notification.list_of_values[0].value = [0x00; 16].to_vec();
    assert!(!dispatch::belongs(&element, Some(&target), &array));
    let mut kept = rows.clone();
    dispatch::merge(&mut kept, &element, &[], false, &array);
    assert_eq!(kept, rows, "the previous value stays");
    let single = notification(
        element.object,
        &[(PropertyIdentifier::PRIORITY_ARRAY, None)],
    );
    assert!(dispatch::belongs(&element, Some(&target), &single));
    let whole = Point {
        watched: Watched::Property {
            property: PropertyIdentifier::PRIORITY_ARRAY,
            index: None,
        },
        ..element
    };
    assert!(!dispatch::belongs(&whole, Some(&target), &mine));
}

/// A property watch that keeps Status_Flags takes in its property
/// whichever order a notification lists them, into empty rows (a first
/// read that failed whole) as into rows it has.
#[test]
fn a_property_watch_takes_its_property_whatever_the_order() {
    let point = Point {
        device: LAB,
        object: ai(1),
        watched: Watched::Property {
            property: PropertyIdentifier::PRESENT_VALUE,
            index: None,
        },
    };
    let pv = PropertyIdentifier::PRESENT_VALUE;
    let flags = PropertyIdentifier::STATUS_FLAGS;
    for order in [[flags, pv], [pv, flags]] {
        let values = [(order[0], None), (order[1], None)];
        let mut rows = Vec::new();
        let merged = dispatch::merge(&mut rows, &point, &[], true, &notification(ai(1), &values));
        let kept: Vec<PropertyIdentifier> = merged.iter().map(|(p, _)| *p).collect();
        assert_eq!(kept, order, "{order:?}");
        assert_eq!(rows.len(), 2);
    }
}

/// Notifications the hub's receiver falls behind on are counted.
#[tokio::test(start_paused = true)]
async fn lagged_notifications_are_counted() {
    let registry = Arc::new(Registry::new(
        PROCESS_BASE,
        Duration::from_secs(1),
        POINT_PROPERTIES.to_vec(),
        false,
    ));
    let (sender, receiver) = tokio::sync::broadcast::channel(1);
    for _ in 0..4 {
        sender.send(notification(ai(1), &[])).unwrap();
    }
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(dispatch::run(Arc::clone(&registry), receiver, stopped));
    settle().await;
    assert_eq!(registry.lagged.load(Ordering::Relaxed), 3);
    stop.send_replace(true);
    task.await.unwrap();
}

/// A point whose device isn't in the table yet is reported so, and is
/// subscribed once the device appears.
#[tokio::test(start_paused = true)]
async fn a_device_that_appears_later_is_subscribed() {
    let lab = Lab::start(WatchOptions::default()).await;
    let point = Point {
        device: 2002,
        object: ai(1),
        watched: Watched::Object,
    };
    let lease = lab.hub.watch(point).unwrap();
    settle().await;
    let snapshot = lease.snapshot();
    assert_eq!(snapshot.state, State::Unreachable);
    assert!(snapshot.note.as_deref().unwrap().contains("device table"));
    lab.engine.client().add_device(2002, &[2]).await.unwrap();
    tokio::time::sleep(Duration::from_secs(30)).await;
    let snapshot = lease.snapshot();
    assert_eq!(snapshot.source, Some(Source::Cov), "{snapshot:?}");
    assert_eq!(snapshot.state, State::Live);
    assert_eq!(snapshot.note, None);
    assert_eq!(present_value(&snapshot), Some(10.0));
    assert_eq!(lab.table().await.len(), 1);
    lab.stop().await;
}
