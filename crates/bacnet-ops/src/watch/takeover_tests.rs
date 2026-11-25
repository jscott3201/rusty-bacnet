//! Which device answers at a watched address (#2603), against the lab
//! device on the hub's own layer, where the device at the address is always
//! the lab's: a forged claim of another device costs one read and ends
//! nothing, a device naming another sender in its notifications keeps its
//! watch after one read, a ReadProperty poll across a table change waits
//! for the device's word, the client forgets what it learned of COV there,
//! missed announcements are caught up, and the device's restarts are told
//! by its Time_Of_Device_Restart or its restart notification.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bacnet_client::client::{
    COVEvent, COVNotificationDelivery, DeviceEvent, DeviceEventKind, ReceivedCOVNotification,
    Station,
};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_types::MacAddr;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Segmentation};
use bacnet_types::primitives::PropertyValue;
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;

use super::hub::*;
use super::shared::{DeviceSlot, Registry, Shared};
use super::test_lab::{LAB, Lab, READ_PROPERTY, SUBSCRIBE_COV, ai, lab_at, object, oid, settle};
use super::*;

/// AI 1's Present_Value as `lease` holds it.
fn present(lease: &Lease) -> Option<f32> {
    let snapshot = lease.snapshot();
    let (_, raw) = snapshot
        .rows
        .iter()
        .find(|(p, _)| *p == PropertyIdentifier::PRESENT_VALUE)?;
    let raw = raw.as_ref().ok()?;
    Some(f32::from_be_bytes(raw.get(1..5)?.try_into().ok()?))
}

async fn wait_for(lease: &Lease, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let snapshot = lease.snapshot();
        if done(&snapshot) {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "{snapshot:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn set(lab: &Lab, value: f32) {
    let value = PropertyValue::Real(value);
    lab.server
        .set_present_value_local(&ai(1), value)
        .await
        .unwrap();
}

/// A subscribed lease on AI 1 of the lab device.
async fn subscribed(lab: &Lab) -> Lease {
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    wait_for(&lease, |s| s.source == Some(Source::Cov)).await;
    settle().await;
    lease
}

fn refuted(lab: &Lab) -> u64 {
    lab.hub.inner.registry.refuted.load(Ordering::Relaxed)
}

/// A stray or forged I-Am names another device at the address: the device
/// there is asked once, says it is still the watched one, and the watch
/// goes on, subscribed, taking its values; the same claim again within the
/// lifetime sends nothing.
#[tokio::test(start_paused = true)]
async fn a_forged_announcement_costs_one_read_and_ends_nothing() {
    let lab = Lab::start(WatchOptions::default()).await;
    let lease = subscribed(&lab).await;
    let from = Instant::now();
    lab.announce(LAB + 1);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(lab.sent(READ_PROPERTY, from), 1, "one read of the Device");
    assert_eq!(refuted(&lab), 1);
    let snapshot = lease.snapshot();
    assert_eq!(
        (snapshot.state, snapshot.source),
        (Phase::Live, Some(Source::Cov))
    );
    set(&lab, 33.0).await;
    wait_for(&lease, |_| present(&lease) == Some(33.0)).await;
    // Its own announcement costs one read of its restart time; the claim
    // again waits a lifetime.
    lab.announce(LAB);
    lab.announce(LAB + 1);
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(
        lab.sent(READ_PROPERTY, from),
        2,
        "the next waits a lifetime"
    );
    assert_eq!(lease.snapshot().state, Phase::Live);
    drop(lease);
    lab.stop().await;
}

/// A device whose notifications name another device as their sender (a
/// gateway naming itself, say): the first is dropped and the device asked
/// once; it answers as the watched device, so its notifications are taken
/// from then on, and the client remembers that for the address.
#[tokio::test(start_paused = true)]
async fn a_device_naming_another_sender_keeps_its_watch_after_one_read() {
    const GATEWAY: u32 = LAB + 7;
    let lab = Lab::start(WatchOptions::default()).await;
    let lease = subscribed(&lab).await;
    let process = lease.shared.process;
    let from = Instant::now();
    lab.notify(process, GATEWAY, ai(1), 55.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_ne!(present(&lease), Some(55.0), "dropped until the device says");
    assert_eq!(lab.sent(READ_PROPERTY, from), 1);
    lab.notify(process, GATEWAY, ai(1), 56.0);
    settle().await;
    assert_eq!(present(&lease), Some(56.0));
    assert_eq!(lab.client.initiator(&lab_at()), Some(GATEWAY));
    assert_eq!(lease.snapshot().state, Phase::Live);
    assert_eq!(lab.sent(READ_PROPERTY, from), 1);
    drop(lease);
    lab.stop().await;
}

/// A device read by ReadProperty alone, polled: the table comes to name
/// another device at the address, with no announcement. The next poll's
/// values aren't taken; the device is asked once, says it is the watched
/// one, and the polls' values are taken again.
#[tokio::test(start_paused = true)]
async fn a_readproperty_poll_across_a_table_change_waits_for_the_device() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.refuse_cov.store(true, Ordering::Relaxed);
    lab.quirks.refuse_rpm.store(true, Ordering::Relaxed);
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    wait_for(&lease, |s| s.source == Some(Source::Poll)).await;
    let low = lab.client.low_level();
    low.add_device(LAB + 1, &[2]).await.unwrap();
    set(&lab, 44.0).await;
    let polls = lease.snapshot().polls;
    wait_for(&lease, |_| refuted(&lab) == 1).await;
    assert_eq!(lease.snapshot().polls, polls, "not taken meanwhile");
    wait_for(&lease, |_| present(&lease) == Some(44.0)).await;
    assert_ne!(lease.snapshot().state, Phase::Gone);
    drop(lease);
    lab.stop().await;
}

/// The client forgets a COV refusal learned at an address once another
/// device announces itself there, and every one after a burst of the
/// discovery table's changes it couldn't keep up with.
#[tokio::test(start_paused = true)]
async fn a_cov_refusal_is_forgotten_when_the_device_at_the_address_changes() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.refuse_cov.store(true, Ordering::Relaxed);
    let known = || lab.client.cov_known(&lab_at(), &Watched::Object);
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    wait_for(&lease, |s| s.source == Some(Source::Poll)).await;
    assert_eq!(known(), CovSupport::Refused);
    // The same device announcing itself keeps it.
    lab.announce(LAB);
    settle().await;
    assert_eq!(known(), CovSupport::Refused);
    lab.announce(LAB + 1);
    settle().await;
    assert_eq!(known(), CovSupport::Unknown, "another device there");
    drop(lease);
    // Learned again, then a burst past the client's channel.
    lab.announce(LAB);
    settle().await;
    let lease = lab.hub.watch(object(ai(2))).unwrap();
    wait_for(&lease, |s| s.source == Some(Source::Poll)).await;
    assert_eq!(known(), CovSupport::Refused);
    for _ in 0..100 {
        lab.announce(LAB);
    }
    settle().await;
    assert_eq!(known(), CovSupport::Unknown, "changes missed");
    drop(lease);
    lab.stop().await;
}

/// Announcements missed: the table names another instance at the address
/// now, though nothing told of it. Once the receiver says it fell behind,
/// the scheduler reads the table, asks the device once, and the watch goes
/// on as the device says.
#[tokio::test(start_paused = true)]
async fn missed_announcements_are_caught_up_from_the_table() {
    let lab = Lab::start(WatchOptions::default()).await;
    let lease = subscribed(&lab).await;
    let low = lab.client.low_level();
    low.add_device(LAB + 1, &[2]).await.unwrap();
    let from = Instant::now();
    lab.hub.inner.registry.missed();
    settle().await;
    assert_eq!(lab.sent(READ_PROPERTY, from), 1, "the device asked");
    assert_eq!(refuted(&lab), 1);
    assert_eq!(lease.snapshot().state, Phase::Live);
    drop(lease);
    lab.stop().await;
}

/// A device with Time_Of_Device_Restart, read once for an announcement (at
/// most once a lifetime). The first read only learns it, within the first
/// subscription's lifetime: nothing is subscribed again. A lifetime on,
/// changed (it restarted), its point is.
#[tokio::test(start_paused = true)]
async fn the_restart_time_decides_whether_an_announcement_resubscribes() {
    let start = Instant::now();
    let lab = Lab::start(WatchOptions::default()).await;
    *lab.quirks.restart_time.lock().unwrap() = Some(1);
    let lease = subscribed(&lab).await;
    assert_eq!(lab.sent(READ_PROPERTY, start), 0, "nothing until announced");
    let from = Instant::now();
    lab.announce(LAB);
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(lab.sent(READ_PROPERTY, from), 1, "one read");
    assert_eq!(lab.sent(SUBSCRIBE_COV, from), 0, "not restarted");
    // A lifetime on, unchanged: one read, nothing subscribed again.
    tokio::time::sleep(Duration::from_secs(300)).await;
    let from = Instant::now();
    lab.announce(LAB);
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(lab.sent(READ_PROPERTY, from), 1);
    assert_eq!(lab.sent(SUBSCRIBE_COV, from), 0, "unchanged");
    tokio::time::sleep(Duration::from_secs(300)).await;
    *lab.quirks.restart_time.lock().unwrap() = Some(2);
    let from = Instant::now();
    lab.announce(LAB);
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(lab.sent(READ_PROPERTY, from), 1);
    assert_eq!(
        lab.sent(SUBSCRIBE_COV, from),
        1,
        "restarted: subscribed again"
    );
    drop(lease);
    lab.stop().await;
}

/// The device's restart notification (unsubscribed, process identifier 0,
/// about its Device object) subscribes its point again without a read.
#[tokio::test(start_paused = true)]
async fn a_restart_notification_subscribes_again() {
    let lab = Lab::start(WatchOptions::default()).await;
    let lease = subscribed(&lab).await;
    tokio::time::sleep(Duration::from_secs(40)).await;
    let from = Instant::now();
    let restart = vec![(PropertyIdentifier::TIME_OF_DEVICE_RESTART, vec![0x21, 9])];
    lab.notify_values(0, LAB, oid(ObjectType::DEVICE, LAB), restart);
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(lab.sent(SUBSCRIBE_COV, from), 1);
    assert_eq!(lab.sent(READ_PROPERTY, from), 0);
    drop(lease);
    lab.stop().await;
}

/// A notification from the lab's MAC under process `process`, sent by
/// device `sender`.
fn notification(process: u32, sender: u32) -> COVEvent {
    COVEvent::Object(ReceivedCOVNotification {
        notification: COVNotificationRequest {
            subscriber_process_identifier: process,
            initiating_device_identifier: oid(ObjectType::DEVICE, sender),
            monitored_object_identifier: ai(1),
            time_remaining: 100,
            list_of_values: Vec::new(),
        },
        source_mac: MacAddr::from_slice(&[2]),
        source_network: None,
        source_address: None,
        delivery: COVNotificationDelivery::Unconfirmed,
    })
}

fn device_event(instance: u32, mac: u8) -> DeviceEvent {
    DeviceEvent {
        kind: DeviceEventKind::Updated,
        device: DiscoveredDevice {
            object_identifier: oid(ObjectType::DEVICE, instance),
            mac_address: MacAddr::from_slice(&[mac]),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
            vendor_id: 42,
            last_seen: std::time::Instant::now(),
            last_announced: None,
            source_network: None,
            source_address: None,
        },
    }
}

/// The receiver falls behind on the discovery table's changes, more than
/// its channel holds: each watched address is told it missed some, and a
/// notification another device sends under a point's process identifier
/// is dropped all the same, its sender noted for the scheduler.
#[tokio::test(start_paused = true)]
async fn a_receiver_behind_on_the_table_still_drops_another_senders_notification() {
    let registry = Arc::new(Registry::new(0x100, Duration::ZERO, Vec::new(), false));
    let slot = Arc::new(DeviceSlot::new());
    let shared = Arc::new(Shared::new(object(ai(1)), 0x100, 4));
    {
        let mut entry = shared.lock();
        entry.identity = Some(LAB);
        entry.target = Some(Station::local(&[2]));
    }
    {
        let mut table = registry.table();
        table.devices.insert(lab_at(), Arc::clone(&slot));
        table.by_process.insert(0x100, Arc::clone(&shared));
    }
    let (cov_tx, notifications) = broadcast::channel(4);
    let (events_tx, events) = broadcast::channel(4);
    let (devices_tx, devices) = broadcast::channel(64);
    let (stop_tx, stop) = watch::channel(false);
    for n in 0..100u32 {
        devices_tx.send(device_event(LAB + 10 + n, 50)).unwrap();
    }
    cov_tx.send(notification(0x100, LAB + 1)).unwrap();
    let heard = super::dispatch::Heard {
        events,
        devices,
        own: Box::new(|| None),
    };
    let task = tokio::spawn(super::dispatch::run(
        Arc::clone(&registry),
        notifications,
        heard,
        stop,
    ));
    settle().await;
    let notes = slot.take_notes();
    assert!(notes.missed, "missed");
    assert_eq!(notes.senders, [LAB + 1]);
    assert_eq!(shared.snapshot().notifications, 0, "dropped");
    assert_eq!(registry.foreign.load(Ordering::Relaxed), 1);
    cov_tx.send(notification(0x100, LAB)).unwrap();
    settle().await;
    assert_eq!(shared.snapshot().notifications, 1, "its own device's taken");
    stop_tx.send_replace(true);
    task.await.unwrap();
    drop(events_tx);
}
