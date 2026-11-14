//! The application's watch ([`Client::watch`], #2560 slice 6) against the
//! lab device: updates in order, the latest value for a slow consumer, the
//! fallback to polling, a silent device, renewal, dropping, and what is
//! learned of COV.

use std::sync::atomic::Ordering;
use std::time::Duration;

use bacnet_types::primitives::PropertyValue;
use futures_util::{FutureExt, StreamExt};
use tokio::time::Instant;

use super::test_lab::{Lab, SUBSCRIBE_COV, ai, lab_at, settle};
use super::*;
use crate::Device;
use bacnet_transport::test_transport::TestTransport;

/// The next update, within a minute of the paused clock.
pub(super) async fn next(watch: &mut Watch) -> Update {
    tokio::time::timeout(Duration::from_secs(60), watch.next())
        .await
        .expect("an update within a minute")
        .expect("the watch goes on")
}

/// The update waiting now, if any: the latest of what came.
pub(super) fn ready(watch: &mut Watch) -> Option<Update> {
    watch.next().now_or_never().flatten()
}

/// The latest of the updates waiting once the lab has settled.
pub(super) async fn latest(watch: &mut Watch) -> Option<Update> {
    settle().await;
    let mut latest = None;
    while let Some(update) = ready(watch) {
        latest = Some(update);
    }
    latest
}

/// The value an update carries, as a number.
pub(super) fn number(update: &Update) -> Option<f64> {
    update.value.as_ref().and_then(crate::Value::as_f64)
}

/// A handle on the lab device, pinned to its row: its one-octet MAC is no
/// address the loopback link's grammar writes.
async fn lab_device(lab: &Lab) -> Device<TestTransport> {
    let low = lab.client.low_level();
    let row = low.get_device(super::test_lab::LAB).await.unwrap().unwrap();
    let device = lab.client.device(&row).await.unwrap();
    assert_eq!(device.address(), lab_at());
    device
}

/// Acceptance: a point subscribes, its first update carries the first
/// read's value named by its units, and each change after comes as a
/// notification, in order.
#[tokio::test(start_paused = true)]
async fn notifications_arrive_in_order_once_subscribed() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    let first = next(&mut watch).await;
    assert_eq!(first.point, device.point("ai:1").unwrap());
    assert_eq!(number(&first), Some(10.0), "{first:?}");
    assert_eq!(first.value.as_ref().unwrap().text, "10 %");
    // The first read, or the notification the device sent on taking the
    // subscription, folded into it.
    assert!(
        matches!(first.via, Some(Via::First | Via::Notification)),
        "{first:?}"
    );
    assert_eq!(
        (first.source, first.state, first.why.clone()),
        (Some(Source::Cov), State::Live, None)
    );
    assert_eq!(first.cov, CovSupport::Accepted);
    // The device may notify the value it had on taking the subscription.
    if let Some(update) = latest(&mut watch).await {
        assert_eq!(number(&update), Some(10.0), "{update:?}");
    }
    for value in [20.0, 30.0, 40.0] {
        lab.server
            .set_present_value_local(&ai(1), PropertyValue::Real(value))
            .await
            .unwrap();
        let update = next(&mut watch).await;
        assert_eq!(number(&update), Some(f64::from(value)), "{update:?}");
        assert_eq!(update.via, Some(Via::Notification));
    }
    assert_eq!(lab.table().await.len(), 1, "one subscription");
    drop((watch, device));
    settle().await;
    lab.stop().await;
}

/// Acceptance: a consumer that falls behind gets the latest value, once,
/// and nothing piles up: the changes it missed are counted as folded.
#[tokio::test(start_paused = true)]
async fn a_slow_consumer_keeps_only_the_latest() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut watch = lab.client.watch([device.point("ai:2").unwrap()]).unwrap();
    next(&mut watch).await;
    latest(&mut watch).await;
    let before = watch.folded();
    for step in 0..10u8 {
        lab.server
            .set_present_value_local(&ai(2), PropertyValue::Real(f32::from(step) + 50.0))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let latest = ready(&mut watch).expect("one update waits");
    assert_eq!(number(&latest), Some(59.0), "{latest:?}");
    assert!(ready(&mut watch).is_none(), "and only one");
    assert_eq!(watch.folded() - before, 9, "nine changes folded into it");
    drop((watch, device));
    settle().await;
    lab.stop().await;
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
    let update = next(&mut first).await;
    assert_eq!(update.source, Some(Source::Poll), "{update:?}");
    assert!(matches!(update.why, Some(Why::Refused(_))), "{update:?}");
    assert_eq!(update.cov, CovSupport::Refused);
    assert_eq!(number(&update), Some(10.0));
    assert_eq!(lab.client.cov_support(&lab_at()), CovSupport::Refused);
    // Polled every 10 s.
    lab.server
        .set_present_value_local(&ai(1), PropertyValue::Real(12.0))
        .await
        .unwrap();
    let polled = next(&mut first).await;
    assert_eq!((polled.via, number(&polled)), (Some(Via::Poll), Some(12.0)));
    let from = Instant::now();
    let mut second = lab.client.watch([device.point("ai:2").unwrap()]).unwrap();
    let update = next(&mut second).await;
    assert_eq!(update.source, Some(Source::Poll), "{update:?}");
    assert_eq!(update.cov, CovSupport::Refused);
    assert_eq!(lab.sent(SUBSCRIBE_COV, from), 0, "not asked again");
    drop((first, second, device));
    settle().await;
    lab.stop().await;
}

/// What the client knows of COV: nothing at first, the device's own list
/// once a summary reads it, and what the device did once asked; a refusal
/// for one object stays that point's own.
#[tokio::test(start_paused = true)]
async fn cov_support_is_learned_from_the_list_and_the_answers() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    assert_eq!(lab.client.cov_support(&lab_at()), CovSupport::Unknown);
    device.describe_device().await.unwrap();
    assert_eq!(lab.client.cov_support(&lab_at()), CovSupport::Offered);
    let mut analog = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    assert_eq!(next(&mut analog).await.cov, CovSupport::Accepted);
    assert_eq!(lab.client.cov_support(&lab_at()), CovSupport::Accepted);
    // A notification class takes no subscription: refused for that point.
    let mut class = lab.client.watch([device.point("nc:1").unwrap()]).unwrap();
    let update = next(&mut class).await;
    assert_eq!(update.cov, CovSupport::Refused, "{update:?}");
    assert!(
        update.value.is_none() && update.error.is_some(),
        "{update:?}"
    );
    assert_eq!(
        lab.client.cov_support(&lab_at()),
        CovSupport::Accepted,
        "one object's refusal is its own"
    );
    drop((analog, class, device));
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
    next(&mut watch).await;
    latest(&mut watch).await;
    lab.quirks.muted.store(true, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(245)).await;
    let lost = latest(&mut watch)
        .await
        .expect("an update when the renewal failed");
    assert_eq!(lost.state, State::Unreachable, "{lost:?}");
    assert!(matches!(lost.why, Some(Why::Lapsed(_))), "{lost:?}");
    assert_eq!(number(&lost), Some(10.0), "the last value stays");
    lab.quirks.muted.store(false, Ordering::Relaxed);
    let back = next(&mut watch).await;
    assert_eq!((back.state, back.source), (State::Live, Some(Source::Poll)));
    tokio::time::sleep(Duration::from_secs(320)).await;
    let again = latest(&mut watch).await.unwrap_or(back);
    assert_eq!(
        (again.source, again.why.clone()),
        (Some(Source::Cov), None),
        "{again:?}"
    );
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
    next(&mut watch).await;
    latest(&mut watch).await;
    tokio::time::sleep(Duration::from_secs(241)).await;
    let remaining = lab.table().await[0].time_remaining;
    assert!(remaining >= 298, "renewed: {remaining}");
    if let Some(update) = latest(&mut watch).await {
        assert_eq!(update.via, Some(Via::Notification), "{update:?}");
        assert_eq!((update.source, &update.why), (Some(Source::Cov), &None));
        assert_eq!(number(&update), Some(10.0));
    }
    drop((watch, device));
    settle().await;
    lab.stop().await;
}

/// Acceptance: dropping the watch cancels its subscription on the wire,
/// and a point another watch holds stays.
#[tokio::test(start_paused = true)]
async fn dropping_a_watch_cancels_what_no_other_holds() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let (one, two) = (device.point("ai:1").unwrap(), device.point("ai:2").unwrap());
    let mut both = lab.client.watch([one.clone(), two]).unwrap();
    let mut only = lab.client.watch([one]).unwrap();
    next(&mut both).await;
    next(&mut only).await;
    assert_eq!(lab.table().await.len(), 2, "one subscription a point");
    drop(both);
    settle().await;
    let table = lab.table().await;
    assert_eq!(table.len(), 1, "{table:?}");
    assert_eq!(
        table[0].monitored_property_reference.object_identifier,
        ai(1)
    );
    let services = lab.services();
    assert_eq!(services.last(), Some(&SUBSCRIBE_COV), "the cancel went");
    drop((only, device));
    settle().await;
    assert!(lab.table().await.is_empty());
    lab.stop().await;
}

/// A watch of nothing ends at once; a point named twice is watched once.
#[tokio::test(start_paused = true)]
async fn an_empty_watch_ends_and_a_point_named_twice_is_one() {
    let lab = Lab::start(WatchOptions::default()).await;
    let mut empty = lab.client.watch([]).unwrap();
    assert!(empty.next().await.is_none());
    let device = lab_device(&lab).await;
    let point = device.point("ai:1").unwrap();
    let mut twice = lab.client.watch([point.clone(), point]).unwrap();
    assert_eq!(twice.points().count(), 1);
    next(&mut twice).await;
    assert_eq!(lab.table().await.len(), 1);
    drop((twice, device));
    settle().await;
    lab.stop().await;
}

/// The point grammar: an object, or its Present_Value, is an object
/// watch; any other property, or an element, a property watch; ALL is
/// refused. A point writes back in the grammar after its address.
#[tokio::test(start_paused = true)]
async fn points_come_from_the_property_grammar() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    for text in ["ai:1", "ai:1 pv", "analog-input:1 present-value"] {
        let point = device.point(text).unwrap();
        assert_eq!(point.watched, Watched::Object, "{text}");
        assert_eq!(point.to_string(), "mstp:2 analog-input:1");
    }
    let element = device.point("ao:2 priority-array[8]").unwrap();
    assert_eq!(
        element.watched,
        Watched::Property {
            property: bacnet_types::enums::PropertyIdentifier::PRIORITY_ARRAY,
            index: Some(8),
        }
    );
    assert_eq!(
        element.to_string(),
        "mstp:2 analog-output:2 priority-array[8]"
    );
    assert!(matches!(
        device.point("ai:1 all"),
        Err(crate::OpsError::Parse { .. })
    ));
    drop(device);
    lab.stop().await;
}

/// A following handle's move of another instance from the device's
/// address speaks for another device: its points stay. One of the
/// device's own instance gives them up.
#[tokio::test(start_paused = true)]
async fn a_move_of_another_instance_leaves_the_points() {
    use crate::device::ClientEvent;
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    next(&mut watch).await;
    latest(&mut watch).await;
    let elsewhere = crate::DeviceAddress::local(&[9]).unwrap();
    let moved = |instance| ClientEvent::Moved {
        instance,
        from: lab_at(),
        to: elsewhere.clone(),
    };
    lab.client.shared.events.send(moved(4242)).unwrap();
    assert!(latest(&mut watch).await.is_none(), "another device's move");
    lab.client
        .shared
        .events
        .send(moved(super::test_lab::LAB))
        .unwrap();
    let update = latest(&mut watch).await.expect("its own move");
    assert_eq!(update.state, State::Gone);
    assert_eq!(update.why, Some(Why::Moved { to: elsewhere }));
    drop((watch, device));
    settle().await;
    lab.stop().await;
}
