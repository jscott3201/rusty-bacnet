//! Which device a watched address holds when nothing names it (#2603),
//! against the lab device on the hub's own layer: only an answer that names
//! the device confirms it, a read by instance never does, a forged row's
//! instance doesn't turn restart reads off, a sender that can't be told
//! apart is polled, and a flood of forged announcements holds values only
//! while the device is being asked.

use std::sync::atomic::Ordering;
use std::time::Duration;

use bacnet_types::primitives::PropertyValue;
use tokio::time::Instant;

use super::hub::*;
use super::takeover_tests::{present, refuted, set, subscribed, wait_for};
use super::test_lab::{LAB, Lab, READ_PROPERTY, RPM, SUBSCRIBE_COV, ai, object, settle};
use super::*;

/// A forged I-Am names another device at the address before a watch there
/// starts: the point starts with that instance, unconfirmed, and the read
/// of its restart time goes to it. However the device there answers that
/// read, it names nothing: the device's own name, read by the wildcard
/// instance, is what the point takes, and it stays Live on it.
async fn forged_row_then(lab: Lab) {
    lab.announce(LAB + 1);
    settle().await;
    let lease = subscribed(&lab).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    set(&lab, 66.0).await;
    wait_for(&lease, |_| present(&lease) == Some(66.0)).await;
    let snapshot = lease.snapshot();
    assert_eq!((snapshot.state, snapshot.why), (Phase::Live, None));
    assert_eq!(lease.shared.confirmed(), Some(LAB));
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(lease.snapshot().state, Phase::Live);
    drop(lease);
    lab.stop().await;
}

/// [`forged_row_then`], the device answering a read of any Device object.
#[tokio::test(start_paused = true)]
async fn a_restart_read_of_a_forged_instance_names_nothing_any_device() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.any_device.store(true, Ordering::Relaxed);
    *lab.quirks.restart_time.lock().unwrap() = Some(1);
    forged_row_then(lab).await;
}

/// [`forged_row_then`], the device answering a missing Device object with
/// another error than unknown-object.
#[tokio::test(start_paused = true)]
async fn a_restart_read_of_a_forged_instance_names_nothing_other_error() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.deny_missing.store(true, Ordering::Relaxed);
    forged_row_then(lab).await;
}

/// A restart-time read finding no Device object of the instance read (a
/// forged row's) learns nothing of restarts: once the device named itself,
/// its restart time is read for an announcement, and a moved one
/// subscribes its point again.
#[tokio::test(start_paused = true)]
async fn a_missing_device_object_leaves_restart_detection_on() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.announce(LAB + 1);
    settle().await;
    let lease = subscribed(&lab).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(lease.shared.confirmed(), Some(LAB));
    *lab.quirks.restart_time.lock().unwrap() = Some(1);
    lab.announce(LAB);
    tokio::time::sleep(Duration::from_secs(20)).await;
    tokio::time::sleep(Duration::from_secs(301)).await;
    *lab.quirks.restart_time.lock().unwrap() = Some(2);
    let from = Instant::now();
    lab.announce(LAB);
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(lab.sent(READ_PROPERTY, from), 1, "its restart time");
    assert_eq!(lab.sent(SUBSCRIBE_COV, from), 1, "restarted");
    drop(lease);
    lab.stop().await;
}

/// A device that refuses the wildcard read, its point started from a stale
/// row: a notification naming another sender is a claim; the watched
/// instance's Device object is missing and the claimed one doesn't answer
/// either, so the one the table names now is read, answers, and the point
/// takes it; the claim is refuted.
#[tokio::test(start_paused = true)]
async fn a_missing_device_and_claim_fall_back_to_the_tables_instance() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.refuse_rpm.store(true, Ordering::Relaxed);
    lab.quirks.refuse_cov.store(true, Ordering::Relaxed);
    lab.quirks.refuse_wildcard.store(true, Ordering::Relaxed);
    let low = lab.client.low_level();
    low.add_device(LAB + 1, &[2]).await.unwrap();
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    wait_for(&lease, |s| s.polls >= 1).await;
    assert_eq!(lease.shared.identity(), Some(LAB + 1));
    low.add_device(LAB, &[2]).await.unwrap();
    lab.notify(lease.shared.process, LAB + 5, ai(1), 150.0);
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(lease.shared.identity(), Some(LAB));
    assert_eq!(refuted(&lab), 1);
    assert_ne!(present(&lease), Some(150.0));
    assert_eq!(lease.snapshot().state, Phase::Live);
    drop(lease);
    lab.stop().await;
}

/// A notification taken while the start's first read runs, with no
/// instance known at the address, doesn't spare the device the start's
/// read of its name: that read comes before the subscription.
#[tokio::test(start_paused = true)]
async fn a_notification_during_the_first_read_leaves_the_name_read() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.client.low_level().clear_devices().await;
    lab.quirks.slow_rpm_ack.store(true, Ordering::Relaxed);
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    while lab.sent(RPM, Instant::now() - Duration::from_secs(60)) == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    lab.notify(lease.shared.process, LAB, ai(1), 12.0);
    settle().await;
    lab.quirks.slow_rpm_ack.store(false, Ordering::Relaxed);
    wait_for(&lease, |s| s.source == Some(Source::Cov)).await;
    let services = lab.services();
    let read = services.iter().position(|s| *s == READ_PROPERTY);
    let subscribe = services.iter().position(|s| *s == SUBSCRIBE_COV);
    assert!(read < subscribe, "{services:?}");
    assert_eq!(lease.shared.confirmed(), Some(LAB));
    drop(lease);
    lab.stop().await;
}

/// A device whose notifications name another sender, and which answers a
/// read of that sender's Device object as itself: the name can't be taken,
/// so its point is polled instead.
#[tokio::test(start_paused = true)]
async fn a_sender_probe_answered_as_the_device_has_the_points_polled() {
    const GATEWAY: u32 = LAB + 7;
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.self_device.store(true, Ordering::Relaxed);
    let lease = subscribed(&lab).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    lab.notify(lease.shared.process, GATEWAY, ai(1), 55.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let snapshot = lease.snapshot();
    assert_eq!(snapshot.source, Some(Source::Poll), "{snapshot:?}");
    assert_eq!(snapshot.why, Some(Why::ForeignSender { sender: GATEWAY }));
    assert_ne!(present(&lease), Some(55.0));
    drop(lease);
    lab.stop().await;
}

/// A flood of forged announcements, each naming a new instance at the
/// address of a confirmed polled point whose polls can't ask which device
/// answers: the device is asked at 0, 60 and 180 s only, and between asks
/// the polls' values are taken; a watch started during the flood gets its
/// values too.
#[tokio::test(start_paused = true)]
async fn a_flood_of_new_instances_holds_no_values_between_asks() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.refuse_cov.store(true, Ordering::Relaxed);
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    wait_for(&lease, |s| s.polls >= 1).await;
    assert_eq!(lease.shared.confirmed(), Some(LAB));
    lab.quirks
        .reject_wildcard_rpm
        .store(true, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(15)).await;
    let from = Instant::now();
    let mut late = None;
    for second in 0..200u32 {
        lab.announce(LAB + 100 + second);
        if second % 20 == 10 {
            set(&lab, second as f32).await;
        }
        if second % 20 == 0 && second > 0 {
            let want = (second - 10) as f32;
            assert_eq!(present(&lease), Some(want), "at {second} s");
        }
        if second == 100 {
            late = Some(lab.hub.watch(object(ai(2))).unwrap());
        }
        if second == 150 {
            let value = PropertyValue::Real(77.0);
            lab.server
                .set_present_value_local(&ai(2), value)
                .await
                .unwrap();
        }
        // Between the asks at 60 and 180 s: polled, not held.
        if second == 175 {
            let late = late.as_ref().unwrap();
            assert_eq!(present(late), Some(77.0), "{:?}", late.snapshot());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let late = late.unwrap();
    assert_eq!(present(&late), Some(77.0), "{:?}", late.snapshot());
    assert_eq!(late.snapshot().state, Phase::Live);
    assert_eq!(lab.sent(READ_PROPERTY, from), 3, "asked at 0, 60, 180 s");
    drop((lease, late));
    lab.stop().await;
}

/// A device that refuses the wildcard read, its watch started on a forged
/// row: nothing names the device, and its restart-time read confirms
/// nothing. Its notifications cue a check; the read of the watched
/// instance doesn't show that device, so the device's own instance is
/// read, answers, and the point takes it (unconfirmed): its values flow.
async fn refusing_the_wildcard_on_a_forged_row(lab: Lab) {
    lab.quirks.refuse_wildcard.store(true, Ordering::Relaxed);
    lab.announce(LAB + 1);
    settle().await;
    let lease = subscribed(&lab).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(lease.shared.confirmed(), None, "no read named it");
    set(&lab, 66.0).await;
    wait_for(&lease, |_| present(&lease) == Some(66.0)).await;
    assert_eq!(lease.shared.identity(), Some(LAB));
    assert_eq!(lease.shared.confirmed(), None);
    let snapshot = lease.snapshot();
    assert_eq!((snapshot.state, snapshot.why), (Phase::Live, None));
    drop(lease);
    lab.stop().await;
}

/// [`refusing_the_wildcard_on_a_forged_row`], the device answering a read
/// of a missing Device object with another error.
#[tokio::test(start_paused = true)]
async fn a_wildcard_refuser_on_a_forged_row_is_repointed_other_error() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.deny_missing.store(true, Ordering::Relaxed);
    refusing_the_wildcard_on_a_forged_row(lab).await;
}

/// [`refusing_the_wildcard_on_a_forged_row`], the device answering a read
/// of another Device object with its own identifier.
#[tokio::test(start_paused = true)]
async fn a_wildcard_refuser_on_a_forged_row_is_repointed_own_identifier() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.self_device.store(true, Ordering::Relaxed);
    refusing_the_wildcard_on_a_forged_row(lab).await;
}

/// The restart-time read queued for a forged row's instance draws another
/// error than unknown-object: that instance isn't the device's, so the
/// answer doesn't turn restart reads off; the device's own restart time is
/// read for its announcements, and a moved one subscribes again.
#[tokio::test(start_paused = true)]
async fn a_forged_instances_missing_restart_time_leaves_detection_on() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.deny_missing.store(true, Ordering::Relaxed);
    lab.announce(LAB + 1);
    settle().await;
    let lease = subscribed(&lab).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(lease.shared.confirmed(), Some(LAB));
    *lab.quirks.restart_time.lock().unwrap() = Some(1);
    lab.announce(LAB);
    tokio::time::sleep(Duration::from_secs(321)).await;
    *lab.quirks.restart_time.lock().unwrap() = Some(2);
    let from = Instant::now();
    lab.announce(LAB);
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(lab.sent(READ_PROPERTY, from), 1, "its restart time");
    assert_eq!(lab.sent(SUBSCRIBE_COV, from), 1, "restarted");
    drop(lease);
    lab.stop().await;
}
