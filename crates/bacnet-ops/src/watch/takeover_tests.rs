//! Another device at a watched address (#2603), against the lab device on
//! the hub's own layer: what the address sends once it names another
//! instance is never a point's (a notification under the point's process
//! identifier, a poll answered across the change), the client forgets what
//! it learned of COV there, and announcements missed by a receiver that
//! fell behind are caught up by reading the discovery table.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bacnet_client::client::{
    COVEvent, COVNotificationDelivery, DeviceEvent, DeviceEventKind, ReceivedCOVNotification,
};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_types::MacAddr;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Segmentation};
use bacnet_types::primitives::PropertyValue;
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;

use super::hub::*;
use super::shared::{DeviceSlot, Registry};
use super::test_lab::{LAB, Lab, RPM, ai, lab_at, object, oid, settle};
use super::*;

const REPLACED: Why = Why::Replaced {
    watched: LAB,
    found: LAB + 1,
};

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

/// Every Present_Value `samples` carried so far.
fn sampled(samples: &mut Samples) -> Vec<f32> {
    let mut values = Vec::new();
    while let Some(sample) = samples.try_next() {
        for (property, raw) in sample.values.iter().flatten() {
            if *property == PropertyIdentifier::PRESENT_VALUE
                && let Ok(raw) = raw
                && let Some(bytes) = raw.get(1..5)
            {
                values.push(f32::from_be_bytes(bytes.try_into().unwrap()));
            }
        }
    }
    values
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

/// The new device notifies under the point's own subscriber process
/// identifier right after announcing itself: the notification isn't the
/// point's, though one from the point's device a moment before was.
#[tokio::test(start_paused = true)]
async fn a_notification_from_the_new_device_is_never_the_points() {
    let lab = Lab::start(WatchOptions::default()).await;
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    let mut samples = lease.samples();
    wait_for(&lease, |s| s.source == Some(Source::Cov)).await;
    let process = lease.shared.process;
    lab.notify(process, LAB, ai(1), 55.0);
    settle().await;
    assert_eq!(present(&lease), Some(55.0), "its own device's");
    lab.announce(LAB + 1);
    lab.notify(process, LAB + 1, ai(1), 77.0);
    settle().await;
    let snapshot = lease.snapshot();
    assert_eq!(
        (snapshot.state, snapshot.why),
        (Phase::Gone, Some(REPLACED))
    );
    assert_eq!(present(&lease), Some(55.0));
    assert!(!sampled(&mut samples).contains(&77.0));
    // Later ones neither, while the address names the other device.
    lab.notify(process, LAB + 1, ai(1), 78.0);
    lab.notify(process, LAB, ai(1), 79.0);
    settle().await;
    assert_eq!(present(&lease), Some(55.0));
    drop(lease);
    lab.stop().await;
}

/// A polled point's poll is out when the address comes to name another
/// device, by an announcement (`announced`) or a change of the discovery
/// table no announcement told of: its answer, read from whichever device
/// answered, isn't the point's.
async fn poll_across_a_change(announced: bool) {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.refuse_cov.store(true, Ordering::Relaxed);
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    let mut samples = lease.samples();
    wait_for(&lease, |s| s.source == Some(Source::Poll)).await;
    let set = PropertyValue::Real(55.0);
    lab.server
        .set_present_value_local(&ai(1), set)
        .await
        .unwrap();
    lab.quirks.slow_rpm_ack.store(true, Ordering::Relaxed);
    let from = Instant::now();
    while lab.sent(RPM, from) == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if announced {
        lab.announce(LAB + 1);
    } else {
        let low = lab.client.low_level();
        low.add_device(LAB + 1, &[2]).await.unwrap();
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let snapshot = lease.snapshot();
    assert_eq!(
        (snapshot.state, snapshot.why),
        (Phase::Gone, Some(REPLACED))
    );
    assert_eq!(present(&lease), Some(10.0), "the read before the change");
    assert!(!sampled(&mut samples).contains(&55.0));
    let polls = lab.sent(RPM, Instant::now());
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(lab.sent(RPM, Instant::now()), polls, "no longer polled");
    drop(lease);
    lab.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_poll_answered_across_an_announcement_is_never_the_points() {
    poll_across_a_change(true).await;
}

#[tokio::test(start_paused = true)]
async fn a_poll_answered_across_a_table_change_is_never_the_points() {
    poll_across_a_change(false).await;
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
/// the scheduler reads the table and gives the point up, sending nothing.
#[tokio::test(start_paused = true)]
async fn missed_announcements_are_caught_up_from_the_table() {
    let lab = Lab::start(WatchOptions::default()).await;
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    wait_for(&lease, |s| s.source == Some(Source::Cov)).await;
    let low = lab.client.low_level();
    low.add_device(LAB + 1, &[2]).await.unwrap();
    let services = lab.services().len();
    lab.hub.inner.registry.missed();
    settle().await;
    let snapshot = lease.snapshot();
    assert_eq!(
        (snapshot.state, snapshot.why),
        (Phase::Gone, Some(REPLACED))
    );
    assert_eq!(lab.services().len(), services, "nothing sent");
    drop(lease);
    lab.stop().await;
}

/// A notification for no point the test's receiver holds.
fn notification() -> COVEvent {
    COVEvent::Object(ReceivedCOVNotification {
        notification: COVNotificationRequest {
            subscriber_process_identifier: 1,
            initiating_device_identifier: oid(ObjectType::DEVICE, LAB),
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
/// its channel holds: each watched address is told it missed some, whether
/// the receiver finds out waiting for the next change or taking them in
/// before a notification.
#[tokio::test(start_paused = true)]
async fn a_receiver_behind_on_the_table_tells_every_scheduler() {
    let registry = Arc::new(Registry::new(0x100, Duration::ZERO, Vec::new(), false));
    let slot = Arc::new(DeviceSlot::new());
    registry.table().devices.insert(lab_at(), Arc::clone(&slot));
    let (cov_tx, notifications) = broadcast::channel(4);
    let (events_tx, events) = broadcast::channel(4);
    let (devices_tx, devices) = broadcast::channel(64);
    let (stop_tx, stop) = watch::channel(false);
    for n in 0..100u32 {
        devices_tx.send(device_event(LAB + 10 + n, 50)).unwrap();
    }
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
    assert_eq!(slot.take_heard(), (Vec::new(), true), "missed");
    // Behind again, found taking them in before a notification, which the
    // receiver looks at first.
    for n in 0..100u32 {
        devices_tx.send(device_event(LAB + 10 + n, 50)).unwrap();
    }
    devices_tx.send(device_event(LAB, 2)).unwrap();
    cov_tx.send(notification()).unwrap();
    settle().await;
    let (heard, missed) = slot.take_heard();
    assert_eq!((heard, missed), (vec![LAB], true));
    stop_tx.send_replace(true);
    task.await.unwrap();
    drop((cov_tx, events_tx));
}
