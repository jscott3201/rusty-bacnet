//! The hub against an in-process lab device over a loopback link. What the
//! hub holds is checked against the device's own subscription table
//! (Active_COV_Subscriptions), not only the hub's counters.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bacnet_client::client::{COVEvent, ReceivedCOVNotification, Station};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::hub::Phase;
use super::hub::*;
use super::shared::Registry;
use super::test_lab::{LAB, Lab, ai, lab_at, object, oid, routed_at, settle};
use super::*;
use crate::format::decode_all;
use std::time::Duration;
use tokio::sync::watch;

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
    assert_eq!(snapshot.state, Phase::Live);
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

/// No lifetime-0 subscription can be built: the type refuses zero (the
/// `bacnet` tool's class table calls zero a write, `bacnet_tool::class`).
#[test]
fn no_lifetime_is_indefinite() {
    assert!(Lifetime::new(0).is_none());
    assert_eq!(Lifetime::new(1).unwrap().secs(), 1);
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
    assert_eq!(snapshot.state, Phase::Unreachable, "{snapshot:?}");
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
    assert_eq!(snapshot.state, Phase::Live);
    lab.stop().await;
}

/// SubscribeCOVProperty for a property watch.
#[tokio::test(start_paused = true)]
async fn a_property_watch_subscribes_to_the_property() {
    let lab = Lab::start(WatchOptions::default()).await;
    let point = Point {
        device: lab_at(),
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
    assert_eq!(a.snapshot().state, Phase::Stopped);
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

/// `received` as the client's stream carries it.
pub(super) fn event(received: &ReceivedCOVNotification) -> COVEvent {
    COVEvent::Object(received.clone())
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
        device: lab_at(),
        object: oid(ObjectType::ANALOG_OUTPUT, 1),
        watched: Watched::Property {
            property: PropertyIdentifier::PRIORITY_ARRAY,
            index: Some(8),
        },
    };
    let target = Station::local(&[2]);
    let other = notification(
        element.object,
        &[(PropertyIdentifier::PRIORITY_ARRAY, Some(9))],
    );
    assert!(!dispatch::belongs(
        &element,
        Some(&target),
        &event(&other),
        None
    ));
    let mine = notification(
        element.object,
        &[(PropertyIdentifier::PRIORITY_ARRAY, Some(8))],
    );
    assert!(dispatch::belongs(
        &element,
        Some(&target),
        &event(&mine),
        None
    ));
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
    assert!(!dispatch::belongs(
        &element,
        Some(&target),
        &event(&array),
        None
    ));
    let mut kept = rows.clone();
    dispatch::merge(&mut kept, &element, &[], false, &array);
    assert_eq!(kept, rows, "the previous value stays");
    let single = notification(
        element.object,
        &[(PropertyIdentifier::PRIORITY_ARRAY, None)],
    );
    assert!(dispatch::belongs(
        &element,
        Some(&target),
        &event(&single),
        None
    ));
    let whole = Point {
        watched: Watched::Property {
            property: PropertyIdentifier::PRIORITY_ARRAY,
            index: None,
        },
        ..element.clone()
    };
    assert!(!dispatch::belongs(
        &whole,
        Some(&target),
        &event(&mine),
        None
    ));
}

/// A property watch that keeps Status_Flags takes in its property
/// whichever order a notification lists them, into empty rows (a first
/// read that failed whole) as into rows it has.
#[test]
fn a_property_watch_takes_its_property_whatever_the_order() {
    let point = Point {
        device: lab_at(),
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
        sender.send(event(&notification(ai(1), &[]))).unwrap();
    }
    let (stop, stopped) = watch::channel(false);
    let (_events, events) = tokio::sync::broadcast::channel(1);
    let (_devices, devices) = tokio::sync::broadcast::channel(1);
    let heard = dispatch::Heard {
        events,
        devices,
        own: Box::new(|| None),
    };
    let task = tokio::spawn(dispatch::run(
        Arc::clone(&registry),
        receiver,
        heard,
        stopped,
    ));
    settle().await;
    assert_eq!(registry.lagged.load(Ordering::Relaxed), 3);
    stop.send_replace(true);
    task.await.unwrap();
}

/// The hub subscribes per object or property, so a COV-multiple
/// notification answers none of its watches, even one carrying a watch's
/// process identifier from its device: the receiver counts it and leaves the
/// entry alone (#2654).
#[tokio::test(start_paused = true)]
async fn cov_multiple_notifications_are_counted_and_left_alone() {
    use bacnet_client::client::{COVNotificationDelivery, ReceivedCOVNotificationMultiple};
    use bacnet_services::cov_multiple::{
        COVNotificationItem, COVNotificationMultipleRequest, COVNotificationValue,
    };
    let registry = Arc::new(Registry::new(
        PROCESS_BASE,
        Duration::ZERO,
        POINT_PROPERTIES.to_vec(),
        false,
    ));
    let shared = Arc::new(super::shared::Shared::new(object(ai(1)), PROCESS_BASE, 4));
    registry
        .table()
        .by_process
        .insert(PROCESS_BASE, Arc::clone(&shared));
    shared.lock().target = Some(Station::local(&[2]));
    let multiple = COVEvent::Multiple(ReceivedCOVNotificationMultiple {
        notification: COVNotificationMultipleRequest {
            subscriber_process_identifier: PROCESS_BASE,
            initiating_device_identifier: oid(ObjectType::DEVICE, LAB),
            time_remaining: 100,
            timestamp: None,
            list_of_cov_notifications: vec![COVNotificationItem {
                monitored_object_identifier: ai(1),
                list_of_values: vec![COVNotificationValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x28, 0, 0],
                    time_of_change: None,
                }],
            }],
        },
        source_mac: bacnet_types::MacAddr::from_slice(&[2]),
        source_network: None,
        source_address: None,
        delivery: COVNotificationDelivery::Unconfirmed,
    });
    let (sender, receiver) = tokio::sync::broadcast::channel(4);
    sender.send(multiple.clone()).unwrap();
    let single = notification(ai(1), &[(PropertyIdentifier::PRESENT_VALUE, None)]);
    sender.send(event(&single)).unwrap();
    sender.send(multiple).unwrap();
    let (stop, stopped) = watch::channel(false);
    let (_events, events) = tokio::sync::broadcast::channel(1);
    let (_devices, devices) = tokio::sync::broadcast::channel(1);
    let heard = dispatch::Heard {
        events,
        devices,
        own: Box::new(|| None),
    };
    let task = tokio::spawn(dispatch::run(
        Arc::clone(&registry),
        receiver,
        heard,
        stopped,
    ));
    settle().await;
    assert_eq!(registry.cov_multiple_ignored.load(Ordering::Relaxed), 2);
    assert_eq!(registry.lagged.load(Ordering::Relaxed), 0);
    // Only the single-object notification reached the entry.
    assert_eq!(shared.lock().snapshot.notifications, 1);
    stop.send_replace(true);
    task.await.unwrap();
}

/// A remote point with no route known is reported so and sends nothing;
/// once the discovery table holds a router for its network, it is read
/// through that router.
#[tokio::test(start_paused = true)]
async fn a_route_that_appears_later_is_used() {
    let lab = Lab::start(WatchOptions::default()).await;
    let point = Point {
        device: routed_at(),
        ..object(ai(1))
    };
    let lease = lab.hub.watch(point).unwrap();
    settle().await;
    let snapshot = lease.snapshot();
    assert_eq!(snapshot.state, Phase::Unreachable);
    assert_eq!(snapshot.why, Some(Why::Missing));
    assert!(snapshot.note.as_deref().unwrap().contains("no route"));
    assert!(lab.services().is_empty(), "{:?}", lab.services());
    lab.route().await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(
        lab.services().first() == Some(&super::test_lab::RPM),
        "read through the router: {:?}",
        lab.services()
    );
    drop(lease);
    lab.stop().await;
}

/// A notification carrying the watched property clears the error a read
/// left for the whole object; one carrying only another property doesn't.
#[tokio::test(start_paused = true)]
async fn a_notified_value_clears_the_whole_objects_error() {
    let registry = Arc::new(Registry::new(
        PROCESS_BASE,
        Duration::ZERO,
        POINT_PROPERTIES.to_vec(),
        false,
    ));
    let shared = Arc::new(super::shared::Shared::new(object(ai(1)), PROCESS_BASE, 4));
    registry
        .table()
        .by_process
        .insert(PROCESS_BASE, Arc::clone(&shared));
    {
        let mut entry = shared.lock();
        entry.target = Some(Station::local(&[2]));
        entry.failure = Some(crate::format::PropertyError::NoValue);
    }
    let flags = notification(ai(1), &[(PropertyIdentifier::STATUS_FLAGS, None)]);
    registry.deliver(&event(&flags), None);
    assert!(shared.lock().failure.is_some(), "not the watched property");
    let value = notification(ai(1), &[(PropertyIdentifier::PRESENT_VALUE, None)]);
    registry.deliver(&event(&value), None);
    assert!(shared.lock().failure.is_none(), "a value of the property");
}

/// A first read the link never sent fails the point the same way whether
/// the watch reads the whole object or one property: `Phase::Failed`, with
/// no row of text standing in for a value (#2610). Before, the
/// one-property watch kept the refusal as a row. With no subscription the
/// point is polled at once, and the poll is refused the same way, so the
/// state seen is the poll's as well as the first read's; the rows are the
/// first read's alone, which a failed poll leaves as they were.
#[tokio::test(start_paused = true)]
async fn a_first_read_never_sent_fails_both_kinds_of_watch_alike() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.unsent_reads.store(true, Ordering::Relaxed);
    // No subscription either, whose first notification would bring the
    // values a read couldn't: the point stays as its first read left it.
    lab.quirks.refuse_cov.store(true, Ordering::Relaxed);
    let property = Point {
        device: lab_at(),
        object: ai(2),
        watched: Watched::Property {
            property: PropertyIdentifier::PRESENT_VALUE,
            index: None,
        },
    };
    let leases = [
        lab.hub.watch(object(ai(1))).unwrap(),
        lab.hub.watch(property).unwrap(),
    ];
    settle().await;
    for lease in &leases {
        let snapshot = lease.snapshot();
        // The first read left no row: a refusal kept as a row would show.
        assert!(snapshot.rows.is_empty(), "{:?}", snapshot.rows);
        assert_eq!(snapshot.state, Phase::Failed, "{:?}", lease.point());
        let note = snapshot.note.as_deref().unwrap_or_default();
        assert!(note.contains("the link refused the send"), "{note}");
    }
    drop(leases);
    settle().await;
    lab.stop().await;
}

/// A subscription the client's own link failed to send is no refusal by
/// the device: nothing is remembered against its COV support, and the
/// point is asked again after the backoff and subscribes, where it used to
/// be polled for good.
#[tokio::test(start_paused = true)]
async fn a_subscription_never_sent_is_asked_again() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.unsent_cov.store(true, Ordering::Relaxed);
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    assert!(lab.table().await.is_empty());
    let device = lab.client.device(LAB).await.unwrap();
    assert_ne!(device.cov_support(), CovSupport::Refused);
    drop(device);
    assert_ne!(lease.snapshot().cov, CovSupport::Refused);
    // The link is back: the retry subscribes.
    lab.quirks.unsent_cov.store(false, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(3600)).await;
    assert_eq!(lab.table().await.len(), 1, "{:?}", lease.snapshot());
    assert_eq!(lease.snapshot().source, Some(Source::Cov));
    drop(lease);
    settle().await;
    lab.stop().await;
}
