//! How a watched point's state goes ([`State`], #2560 slice 6), against
//! the lab device: a refusal of COV remembered by address, a full
//! subscription table asked again, a silent device, a given-up point that
//! a poll's end can't revive, an error that a notified value clears, a
//! move of another instance, and the cancels of a silent device.

use std::sync::atomic::Ordering;
use std::time::Duration;

use bacnet_types::primitives::PropertyValue;
use tokio::time::Instant;

use super::State;
use super::hub::*;
use super::stream_tests::{lab_device, latest, next, number, own_hub, until};
use super::test_lab::{LAB, Lab, RPM, SUBSCRIBE_COV, ai, lab_at, oid, settle};
use super::*;
use crate::device::ClientEvent;
use bacnet_types::enums::ObjectType;

fn polled_for(update: &Update) -> Option<&Why> {
    match &update.state {
        State::Polled { why } => Some(why),
        _ => None,
    }
}

/// Acceptance: a device that rejects SubscribeCOV as an unknown service is
/// polled, the update says why, and the client remembers it by address:
/// a later point on the device is polled without asking again.
#[tokio::test(start_paused = true)]
async fn a_device_refusing_cov_is_polled_and_not_asked_again() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.refuse_cov.store(true, Ordering::Relaxed);
    let device = lab_device(&lab).await;
    let mut first = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    let update = until(&mut first, |u| polled_for(u).is_some()).await;
    assert!(
        matches!(polled_for(&update), Some(Why::Refused(_))),
        "{update:?}"
    );
    assert_eq!(update.cov, CovSupport::Refused);
    assert_eq!(number(&update), Some(10.0));
    assert_eq!(device.cov_support(), CovSupport::Refused);
    // Polled every 10 s.
    lab.server
        .set_present_value_local(&ai(1), PropertyValue::Real(12.0))
        .await
        .unwrap();
    let polled = until(&mut first, |u| u.via == Some(Via::Poll)).await;
    assert_eq!(number(&polled), Some(12.0));
    let from = Instant::now();
    let mut second = lab.client.watch([device.point("ai:2").unwrap()]).unwrap();
    let update = until(&mut second, |u| polled_for(u).is_some()).await;
    assert_eq!(update.cov, CovSupport::Refused);
    assert_eq!(lab.sent(SUBSCRIBE_COV, from), 0, "not asked again");
    drop((first, second, device));
    settle().await;
    lab.stop().await;
}

/// What a device's handle knows of COV: nothing at first, the device's own
/// list once a summary reads it, and what the device did once asked; a
/// refusal for one object stays that point's own.
#[tokio::test(start_paused = true)]
async fn cov_support_is_learned_from_the_list_and_the_answers() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    assert_eq!(device.cov_support(), CovSupport::Unknown);
    device.describe_device().await.unwrap();
    assert_eq!(device.cov_support(), CovSupport::Offered);
    let mut analog = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    let update = until(&mut analog, |u| u.state == State::Subscribed).await;
    assert_eq!(update.cov, CovSupport::Accepted);
    assert_eq!(device.cov_support(), CovSupport::Accepted);
    // A notification class takes no subscription: refused for that point.
    let mut class = lab.client.watch([device.point("nc:1").unwrap()]).unwrap();
    let update = until(&mut class, |u| polled_for(u).is_some()).await;
    assert_eq!(update.cov, CovSupport::Refused, "{update:?}");
    assert!(
        update.value.is_none() && update.error.is_some(),
        "{update:?}"
    );
    assert_eq!(
        device.cov_support(),
        CovSupport::Accepted,
        "one object's refusal is its own"
    );
    drop((analog, class, device));
    settle().await;
    lab.stop().await;
}

/// A full subscription table is no refusal: the point is polled, says the
/// table is full, and is subscribed once the retry finds room.
#[tokio::test(start_paused = true)]
async fn a_full_table_is_polled_and_asked_again() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.full_cov.store(1, Ordering::Relaxed);
    let device = lab_device(&lab).await;
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    let update = until(&mut watch, |u| polled_for(u).is_some()).await;
    assert!(
        matches!(polled_for(&update), Some(Why::Full(_))),
        "{update:?}"
    );
    assert_ne!(update.cov, CovSupport::Refused, "a full table frees up");
    assert_eq!(device.cov_support(), CovSupport::Unknown);
    let update = until(&mut watch, |u| u.state == State::Subscribed).await;
    assert_eq!(update.cov, CovSupport::Accepted);
    assert_eq!(lab.table().await.len(), 1);
    drop((watch, device));
    settle().await;
    lab.stop().await;
}

/// A device that goes silent: its renewal fails, the update says it is
/// unreachable and why, and once it answers again it is polled, then
/// subscribed again.
#[tokio::test(start_paused = true)]
async fn a_silent_device_is_reported_and_comes_back() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    latest(&mut watch).await;
    lab.quirks.muted.store(true, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(245)).await;
    let lost = latest(&mut watch)
        .await
        .expect("an update when the renewal failed");
    assert!(
        matches!(
            lost.state,
            State::Unreachable {
                why: Why::Lapsed(_)
            }
        ),
        "{lost:?}"
    );
    assert_eq!(number(&lost), Some(10.0), "the last value stays");
    lab.quirks.muted.store(false, Ordering::Relaxed);
    let back = until(&mut watch, |u| polled_for(u).is_some()).await;
    assert!(
        matches!(polled_for(&back), Some(Why::Lapsed(_))),
        "{back:?}"
    );
    until(&mut watch, |u| u.state == State::Subscribed).await;
    drop((watch, device));
    settle().await;
    lab.stop().await;
}

/// The subscription is renewed while a watch holds it: the watch stays
/// subscribed, whatever the device notifies on taking the renewal.
#[tokio::test(start_paused = true)]
async fn a_watch_keeps_its_subscription_renewed() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    latest(&mut watch).await;
    tokio::time::sleep(Duration::from_secs(241)).await;
    let remaining = lab.table().await[0].time_remaining;
    assert!(remaining >= 298, "renewed: {remaining}");
    if let Some(update) = latest(&mut watch).await {
        assert_eq!(update.via, Some(Via::Notification), "{update:?}");
        assert_eq!(update.state, State::Subscribed);
        assert_eq!(number(&update), Some(10.0));
    }
    drop((watch, device));
    settle().await;
    lab.stop().await;
}

/// A point given up while a poll of its device is out stays given up when
/// the poll ends, whether the device answered it or not.
#[tokio::test(start_paused = true)]
async fn a_poll_ending_after_a_move_leaves_the_point_given_up() {
    for answered in [false, true] {
        let lab = Lab::start(WatchOptions::default()).await;
        lab.quirks.refuse_cov.store(true, Ordering::Relaxed);
        let device = lab_device(&lab).await;
        let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
        until(&mut watch, |u| polled_for(u).is_some()).await;
        latest(&mut watch).await;
        match answered {
            true => lab.quirks.slow_rpm_ack.store(true, Ordering::Relaxed),
            false => lab.quirks.muted.store(true, Ordering::Relaxed),
        }
        let from = Instant::now();
        while lab.sent(RPM, from) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let elsewhere = crate::DeviceAddress::local(&[9]).unwrap();
        let moved = ClientEvent::Moved {
            instance: LAB,
            from: lab_at(),
            to: elsewhere.clone(),
        };
        lab.client.shared.events.send(moved).unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
        let last = latest(&mut watch).await.expect("given up");
        let gone = State::Gone {
            why: Why::Moved { to: elsewhere },
        };
        assert_eq!(last.state, gone, "answered {answered}: {last:?}");
        drop((watch, device));
        lab.quirks.muted.store(false, Ordering::Relaxed);
        settle().await;
        lab.stop().await;
    }
}

/// A first read that fails for the whole object, then a subscription the
/// device takes: the read's error goes with the first notified value.
#[tokio::test(start_paused = true)]
async fn a_notified_value_clears_the_first_reads_error() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    lab.quirks.fail_rpm.store(true, Ordering::Relaxed);
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    lab.server
        .set_present_value_local(&ai(1), PropertyValue::Real(42.0))
        .await
        .unwrap();
    let update = until(&mut watch, |u| number(u) == Some(42.0)).await;
    assert_eq!(update.state, State::Subscribed);
    assert!(update.error.is_none(), "a stale error: {update:?}");
    drop((watch, device));
    settle().await;
    lab.stop().await;
}

/// A following handle's move of another instance from the device's
/// address speaks for another device: its points stay. One of the
/// device's own instance gives them up.
#[tokio::test(start_paused = true)]
async fn a_move_of_another_instance_leaves_the_points() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    latest(&mut watch).await;
    let elsewhere = crate::DeviceAddress::local(&[9]).unwrap();
    let moved = |instance| ClientEvent::Moved {
        instance,
        from: lab_at(),
        to: elsewhere.clone(),
    };
    lab.client.shared.events.send(moved(4242)).unwrap();
    assert!(latest(&mut watch).await.is_none(), "another device's move");
    lab.client.shared.events.send(moved(LAB)).unwrap();
    let update = latest(&mut watch).await.expect("its own move");
    assert_eq!(
        update.state,
        State::Gone {
            why: Why::Moved { to: elsewhere }
        }
    );
    drop((watch, device));
    settle().await;
    lab.stop().await;
}

/// Dropping every watch of a silent device lets the hub go after one
/// cancel's wait: the first cancel goes unanswered, and the rest are left
/// to lapse rather than each waiting its turn.
#[tokio::test(start_paused = true)]
async fn dropping_the_watches_of_a_silent_device_is_prompt() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let points = [
        device.point("ai:1").unwrap(),
        device.point("ai:2").unwrap(),
        device.point("ai:3").unwrap(),
        device
            .point((oid(ObjectType::BINARY_VALUE, 1), PRESENT))
            .unwrap(),
    ];
    let mut watch = lab.client.watch(points).unwrap();
    while lab.table().await.len() < 4 {
        next(&mut watch).await;
    }
    lab.quirks.muted.store(true, Ordering::Relaxed);
    let subscriptions = lab
        .services()
        .iter()
        .filter(|s| **s == SUBSCRIBE_COV)
        .count();
    drop(watch);
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert!(!own_hub(&lab), "the hub let go within one cancel's wait");
    let sent = lab
        .services()
        .iter()
        .filter(|s| **s == SUBSCRIBE_COV)
        .count();
    assert_eq!(sent - subscriptions, 1, "one cancel sent");
    lab.quirks.muted.store(false, Ordering::Relaxed);
    drop(device);
    lab.stop().await;
}

const PRESENT: bacnet_types::enums::PropertyIdentifier =
    bacnet_types::enums::PropertyIdentifier::PRESENT_VALUE;

/// Tell the hub that a handle following `instance` moved it from the lab
/// device's address.
fn moved_away(lab: &Lab, instance: u32) {
    let to = crate::DeviceAddress::local(&[9]).unwrap();
    let from = lab_at();
    let moved = ClientEvent::Moved { instance, from, to };
    lab.client.shared.events.send(moved).unwrap();
}

/// A start in flight when its device's points are given up, answered after
/// the table came to name another instance at the address, brings the
/// given-up point back no more than any other start would: the table must
/// name the point's instance there when the start is answered.
#[tokio::test(start_paused = true)]
async fn a_start_from_before_the_move_does_not_revive() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut first = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut first, |u| u.state == State::Subscribed).await;
    latest(&mut first).await;
    lab.quirks.slow_rpm_ack.store(true, Ordering::Relaxed);
    let from = Instant::now();
    let mut late = lab.client.watch([device.point("ai:2").unwrap()]).unwrap();
    while lab.sent(RPM, from) == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Another device at the address now, and the point's device elsewhere.
    lab.client
        .low_level()
        .add_device(LAB + 1, &[2])
        .await
        .unwrap();
    moved_away(&lab, LAB);
    until(&mut first, |u| matches!(u.state, State::Gone { .. })).await;
    lab.quirks.slow_rpm_ack.store(false, Ordering::Relaxed);
    until(&mut late, |u| u.state == State::Subscribed).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let after = latest(&mut first).await;
    assert!(after.is_none(), "revived by a stale start: {after:?}");
    drop((first, late, device));
    settle().await;
    lab.stop().await;
}

/// A move told while the hub doesn't know the device's instance (no row
/// names it) gives the points up for the instance the move names, so they
/// still come back once the address answers for it.
#[tokio::test(start_paused = true)]
async fn a_move_told_before_the_instance_is_known_can_be_undone() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    lab.client.low_level().clear_devices().await;
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    moved_away(&lab, LAB);
    until(&mut watch, |u| matches!(u.state, State::Gone { .. })).await;
    // The table names the instance at the address again, and a start
    // there is answered.
    lab.client.low_level().add_device(LAB, &[2]).await.unwrap();
    let mut late = lab.client.watch([device.point("ai:2").unwrap()]).unwrap();
    until(&mut late, |u| u.state == State::Subscribed).await;
    until(&mut watch, |u| u.state == State::Subscribed).await;
    drop((watch, late, device));
    settle().await;
    lab.stop().await;
}

/// Two give-ups at one address under two instances: each point comes back
/// only when the address answers for its own instance.
#[tokio::test(start_paused = true)]
async fn each_moved_point_comes_back_for_its_own_instance() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut first = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut first, |u| u.state == State::Subscribed).await;
    moved_away(&lab, LAB);
    until(&mut first, |u| matches!(u.state, State::Gone { .. })).await;
    // Another device's instance named at the address: a point started
    // there now is that device's, and it moves away too.
    let other = LAB + 1;
    lab.client
        .low_level()
        .add_device(other, &[2])
        .await
        .unwrap();
    let mut second = lab.client.watch([device.point("ai:2").unwrap()]).unwrap();
    until(&mut second, |u| u.state == State::Subscribed).await;
    assert!(
        latest(&mut first).await.is_none(),
        "not the first's instance"
    );
    moved_away(&lab, other);
    until(&mut second, |u| matches!(u.state, State::Gone { .. })).await;
    // The address answers for the other instance: only its point is back.
    let mut third = lab.client.watch([device.point("ai:3").unwrap()]).unwrap();
    until(&mut third, |u| u.state == State::Subscribed).await;
    until(&mut second, |u| u.state == State::Subscribed).await;
    let after = latest(&mut first).await;
    assert!(
        after.is_none(),
        "the first's instance wasn't answered: {after:?}"
    );
    drop((first, second, third, device));
    settle().await;
    lab.stop().await;
}
