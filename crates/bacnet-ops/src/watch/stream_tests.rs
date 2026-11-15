//! The application's watch ([`Client::watch`], #2560 slice 6) against the
//! lab device: updates in order, the latest value for a slow consumer,
//! dropping, the cap, the client's one hub and its range, and the point
//! grammar.

use std::time::Duration;

use bacnet_types::primitives::{PropertyValue, StatusFlags};
use futures_util::{FutureExt, StreamExt};

use super::State;
use super::hub::*;
use super::test_lab::{LAB, Lab, SUBSCRIBE_COV, ai, lab_at, settle};
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

/// Updates until `done` holds for one, within ten minutes of the paused
/// clock.
pub(super) async fn until(watch: &mut Watch, done: impl Fn(&Update) -> bool) -> Update {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    loop {
        let update = tokio::time::timeout_at(deadline, watch.next())
            .await
            .expect("the update looked for, within ten minutes")
            .expect("the watch goes on");
        if done(&update) {
            return update;
        }
    }
}

/// The value an update carries, as a number.
pub(super) fn number(update: &Update) -> Option<f64> {
    update.value.as_ref().and_then(crate::Value::as_f64)
}

/// A handle on the lab device, pinned to its row: its one-octet MAC is no
/// address the loopback link's grammar writes.
pub(super) async fn lab_device(lab: &Lab) -> Device<TestTransport> {
    let low = lab.client.low_level();
    let row = low.get_device(LAB).await.unwrap().unwrap();
    let device = lab.client.device(&row).await.unwrap();
    assert_eq!(device.address(), lab_at());
    device
}

/// Whether the client's own hub is alive.
pub(super) fn own_hub(lab: &Lab) -> bool {
    lab.client
        .shared
        .hub
        .lock()
        .unwrap()
        .hub
        .upgrade()
        .is_some()
}

/// Acceptance: a point subscribes, its first update carries the first
/// read's value named by its units, with its Status_Flags and when it
/// came, and each change after comes as a notification, in order.
#[tokio::test(start_paused = true)]
async fn notifications_arrive_in_order_once_subscribed() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    let first = next(&mut watch).await;
    assert_eq!(first.point, device.point("ai:1").unwrap());
    assert_eq!(number(&first), Some(10.0), "{first:?}");
    assert_eq!(first.value.as_ref().unwrap().text, "10 %");
    assert_eq!(first.status_flags, Some(StatusFlags::empty()));
    assert!(first.updated.is_some_and(|at| at.elapsed().is_ok()));
    // The first read, or the notification the device sent on taking the
    // subscription, folded into it.
    assert!(
        matches!(first.via, Some(Via::First | Via::Notification)),
        "{first:?}"
    );
    assert_eq!(first.state, State::Subscribed);
    assert_eq!(first.cov, CovSupport::Accepted);
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
        assert_eq!(update.state, State::Subscribed);
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

/// Points that would take a hub past its cap take nothing and send
/// nothing: the client's own hub with one point more than it watches, and
/// a hub of two that already watches one, asked for two more.
#[tokio::test(start_paused = true)]
async fn a_watch_past_the_cap_takes_nothing() {
    let options = WatchOptions {
        max_points: 2,
        ..WatchOptions::default()
    };
    let lab = Lab::start(options).await;
    let device = lab_device(&lab).await;
    let points: Vec<Point> = (0..=MAX_WATCHED as u32)
        .map(|n| device.point((ai(n + 1), PROPERTY)).unwrap())
        .collect();
    let refused = lab.client.watch(points).unwrap_err();
    assert!(
        matches!(
            refused,
            crate::OpsError::Watch(WatchError::Full { max: MAX_WATCHED })
        ),
        "{refused:?}"
    );
    assert!(refused.to_string().contains("cancelling"), "{refused}");
    settle().await;
    assert!(lab.services().is_empty(), "{:?}", lab.services());
    assert!(!own_hub(&lab), "nothing holds the client's hub");
    let held = lab.hub.watch(device.point("ai:1").unwrap()).unwrap();
    let more = vec![device.point("ai:2").unwrap(), device.point("ai:3").unwrap()];
    assert_eq!(
        lab.hub.watch_all(more).unwrap_err(),
        WatchError::Full { max: 2 }
    );
    assert_eq!(lab.hub.stats().points, 1, "the one held, alone");
    drop((held, device));
    settle().await;
    lab.stop().await;
}

/// The property a Present_Value point names.
const PROPERTY: bacnet_types::enums::PropertyIdentifier =
    bacnet_types::enums::PropertyIdentifier::PRESENT_VALUE;

/// The client's own hub goes with its last watch and comes back with the
/// next, in the same range of subscriber process identifiers each time,
/// however often: a few hundred cycles never take a second range, which
/// could wrap onto another hub's (the lab's own hub took the first).
#[tokio::test(start_paused = true)]
async fn the_client_hub_keeps_one_range_however_often_it_is_made() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    for _ in 0..300 {
        let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
        until(&mut watch, |u| u.state == State::Subscribed).await;
        let table = lab.table().await;
        assert_eq!(table.len(), 1);
        let process = table[0].recipient.process_identifier;
        assert_eq!(process, PROCESS_BASE + 0x1_0000, "the client's range");
        drop(watch);
        settle().await;
        assert!(!own_hub(&lab), "the hub went with its last watch");
        assert!(lab.table().await.is_empty(), "cancelled");
    }
    assert_eq!(lab.client.next_hub(), 2, "two ranges taken in all");
    drop(device);
    lab.stop().await;
}

/// Outside a Tokio runtime a watch is refused, not a panic.
#[test]
fn a_watch_outside_a_runtime_is_refused() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    let (lab, point) = runtime.block_on(async {
        let lab = Lab::start(WatchOptions::default()).await;
        let point = lab_device(&lab).await.point("ai:1").unwrap();
        (lab, point)
    });
    let refused = lab.client.watch([point]).unwrap_err();
    assert!(
        matches!(refused, crate::OpsError::Watch(WatchError::NoRuntime)),
        "{refused:?}"
    );
    runtime.block_on(lab.stop());
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

/// The client's hub, made again in its one range, gives its first point
/// the identifier an earlier hub's subscription still holds on the device
/// (its cancel went unanswered). That old subscription's notifications,
/// about another object, never land on the new point.
#[tokio::test(start_paused = true)]
async fn a_reused_identifier_takes_no_late_notification() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    lab.quirks
        .muted
        .store(true, std::sync::atomic::Ordering::Relaxed);
    drop(watch);
    tokio::time::sleep(Duration::from_secs(40)).await;
    assert!(!own_hub(&lab), "the hub went");
    lab.quirks
        .muted
        .store(false, std::sync::atomic::Ordering::Relaxed);
    let old = lab.table().await;
    assert_eq!(old.len(), 1, "the old subscription stays: {old:?}");
    let mut again = lab.client.watch([device.point("ai:2").unwrap()]).unwrap();
    until(&mut again, |u| u.state == State::Subscribed).await;
    let table = lab.table().await;
    let processes: Vec<u32> = table
        .iter()
        .map(|s| s.recipient.process_identifier)
        .collect();
    assert_eq!(
        processes[0], processes[1],
        "one identifier, two objects: {table:?}"
    );
    latest(&mut again).await;
    lab.server
        .set_present_value_local(&ai(1), PropertyValue::Real(77.0))
        .await
        .unwrap();
    settle().await;
    while let Some(update) = ready(&mut again) {
        assert_ne!(
            number(&update),
            Some(77.0),
            "ai:1's value on ai:2: {update:?}"
        );
    }
    drop((again, device));
    settle().await;
    lab.stop().await;
}

/// Another hub made on the client never takes the client hub's range,
/// however many are made and however the ranges wrap.
#[tokio::test(start_paused = true)]
async fn other_hubs_skip_the_client_hubs_range() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    let ranges = super::hub::PROCESS_RANGES;
    let taken: Vec<u32> = (0..2 * ranges).map(|_| lab.client.hub_range()).collect();
    assert!(
        taken.iter().all(|range| range % ranges != 1),
        "the client hub took range 1"
    );
    drop((watch, device));
    settle().await;
    lab.stop().await;
}
