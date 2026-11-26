//! A client's watch settings and the hubs they pool on (#2603): equal
//! settings share a hub and others get their own, each freed with its last
//! watch; polling only; settings refused before anything is sent; a
//! lifetime the device refuses told from a refused service; and a watch
//! ending with its hub.

use std::time::Duration;

use super::control_tests::{hubs_gone, sent};
use super::hub::{Backoff, Lifetime, WatchOptions};
use super::stream_tests::{lab_device, next, pooled, until};
use super::test_lab::{Lab, SUBSCRIBE_COV, ai, settle};
use super::*;
use crate::OpsError;

/// Why `update`'s point is polled, if it is.
fn polled_for(update: &Update) -> Option<&Why> {
    match &update.state {
        State::Polled { why } => Some(why),
        _ => None,
    }
}

/// How many ranges of subscriber process identifiers `lab`'s client holds.
fn ranges(lab: &Lab) -> u32 {
    lab.client.shared.ranges.lock().unwrap().held()
}

/// Watches of equal settings share one hub, and one subscription a point;
/// other settings, from `watch_with` or a clone's defaults, run on a hub
/// of their own, in a range of their own. Each hub goes with its last
/// watch and gives its range back.
#[tokio::test(start_paused = true)]
async fn equal_settings_share_a_hub_and_others_get_their_own() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let (one, two) = (device.point("ai:1").unwrap(), device.point("ai:2").unwrap());
    let mut first = lab.client.watch([one.clone()]).unwrap();
    let mut second = device.watch(["ai:1"]).unwrap();
    until(&mut first, |u| u.state == State::Subscribed).await;
    until(&mut second, |u| u.state == State::Subscribed).await;
    assert_eq!(lab.table().await.len(), 1, "one subscription");
    assert_eq!((pooled(&lab.client), ranges(&lab)), (1, 2));
    let polled = WatchSettings::default().with_subscribe(false);
    let before = sent(&lab, SUBSCRIBE_COV);
    let mut third = lab.client.watch_with([one.clone()], &polled).unwrap();
    let update = until(&mut third, |u| polled_for(u).is_some()).await;
    assert_eq!(polled_for(&update), Some(&Why::PollOnly));
    assert_eq!(sent(&lab, SUBSCRIBE_COV), before, "nothing subscribed");
    assert_eq!((pooled(&lab.client), ranges(&lab)), (2, 3));
    // A clone whose defaults poll only joins that hub.
    let clone = lab.client.with_watch_settings(polled.clone());
    assert_eq!(clone.watch_settings(), &polled);
    let mut fourth = clone.watch([two]).unwrap();
    next(&mut fourth).await;
    assert_eq!((pooled(&lab.client), ranges(&lab)), (2, 3));
    drop((first, second));
    settle().await;
    assert_eq!((pooled(&lab.client), ranges(&lab)), (1, 2));
    assert!(lab.table().await.is_empty(), "cancelled with its hub");
    drop((third, fourth));
    settle().await;
    assert_eq!((pooled(&lab.client), ranges(&lab)), (0, 1));
    drop((device, clone));
    lab.stop().await;
}

/// A pooled hub whose last watch went stays while its cancel is out, so
/// its range isn't taken again under a cancel that may still go, then
/// goes.
#[tokio::test(start_paused = true)]
async fn a_pooled_hub_stays_until_its_cancels_are_done() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut watch = device.watch(["ai:1"]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    lab.quirks
        .muted
        .store(true, std::sync::atomic::Ordering::Relaxed);
    drop(watch);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!((pooled(&lab.client), ranges(&lab)), (1, 2), "cancelling");
    tokio::time::sleep(Duration::from_secs(40)).await;
    assert_eq!((pooled(&lab.client), ranges(&lab)), (0, 1), "given up");
    lab.quirks
        .muted
        .store(false, std::sync::atomic::Ordering::Relaxed);
    drop(device);
    lab.stop().await;
}

/// Settings no hub takes are refused before anything is made or sent:
/// polls under a second or over an hour, from `watch_with` or a clone's
/// defaults.
#[tokio::test(start_paused = true)]
async fn settings_a_hub_refuses_take_nothing() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let one = device.point("ai:1").unwrap();
    for every in [Duration::from_millis(500), Duration::from_secs(3601)] {
        let settings = WatchSettings::default().with_poll_every(every);
        assert!(settings.validate().is_err(), "{every:?}");
        let refused = lab.client.watch_with([one.clone()], &settings).unwrap_err();
        assert!(
            matches!(refused, OpsError::Watch(WatchError::Settings(_))),
            "{refused:?}"
        );
        let clone = lab.client.with_watch_settings(settings);
        assert!(clone.watch([one.clone()]).is_err());
    }
    settle().await;
    assert!(lab.services().is_empty(), "{:?}", lab.services());
    assert_eq!((pooled(&lab.client), ranges(&lab)), (0, 1));
    // The longest lifetime and poll interval are taken.
    let longest = WatchSettings::default()
        .with_lifetime(Lifetime::MAX)
        .with_poll_every(Duration::from_secs(3600));
    assert!(longest.validate().is_ok());
    drop(device);
    lab.stop().await;
}

/// A device that refuses a subscription for its lifetime (a services-class
/// value-out-of-range error) refuses that point, as any refusal but the
/// service's own: polled and not asked again, and nothing is remembered of
/// the device's COV service, so its next point is asked.
#[tokio::test(start_paused = true)]
async fn a_refused_lifetime_refuses_the_point_not_the_service() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks
        .short_lifetimes
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let device = lab_device(&lab).await;
    let mut watch = device.watch(["ai:1"]).unwrap();
    let update = until(&mut watch, |u| polled_for(u).is_some()).await;
    assert!(
        matches!(polled_for(&update), Some(Why::Refused(_))),
        "{update:?}"
    );
    assert_eq!(update.cov, CovSupport::Refused, "this point's subscription");
    assert_eq!(device.cov_support(), CovSupport::Unknown, "not the service");
    let before = sent(&lab, SUBSCRIBE_COV);
    let two = device.point("ai:2").unwrap();
    watch.control().add([two.clone()]).unwrap();
    let update = until(&mut watch, |u| u.point == two && polled_for(u).is_some()).await;
    assert!(
        matches!(polled_for(&update), Some(Why::Refused(_))),
        "{update:?}"
    );
    assert_eq!(sent(&lab, SUBSCRIBE_COV) - before, 1, "asked again");
    drop((watch, device));
    settle().await;
    lab.stop().await;
}

/// A watch whose hub shuts down ends at once: `next` gives `None`, a change
/// waiting included, and stays so; its controls are refused; the hub's
/// cancels went in the wait. A watch made after runs on a new hub.
#[tokio::test(start_paused = true)]
async fn a_watch_ends_when_its_hub_shuts_down() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut watch = device.watch(["ai:1"]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    let control = watch.control();
    lab.server
        .set_present_value_local(&ai(1), bacnet_types::primitives::PropertyValue::Real(12.0))
        .await
        .unwrap();
    settle().await;
    assert!(lab.client.shutdown_watches(Duration::from_secs(5)).await);
    assert!(watch.next().await.is_none(), "ended at once");
    assert!(watch.next().await.is_none(), "and stays ended");
    assert_eq!(
        control.add([device.point("ai:2").unwrap()]),
        Err(WatchError::Closed)
    );
    assert!(lab.table().await.is_empty(), "cancelled");
    drop(control);
    settle().await;
    assert_eq!((pooled(&lab.client), ranges(&lab)), (0, 1));
    let mut again = device.watch(["ai:1"]).unwrap();
    until(&mut again, |u| u.state == State::Subscribed).await;
    drop((watch, again, device));
    settle().await;
    lab.stop().await;
}

/// Concurrency: a task awaiting `next` when the hub shuts down gets `None`,
/// never a point given up for the stop. Real threads.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_waiting_next_ends_when_the_hub_shuts_down() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut watch = device.watch(["ai:1", "ai:2"]).unwrap();
    let wait = Duration::from_secs(10);
    tokio::time::timeout(wait, until(&mut watch, |u| u.state == State::Subscribed))
        .await
        .unwrap();
    let reader = tokio::spawn(async move {
        let mut seen = Vec::new();
        while let Some(update) = watch.next().await {
            seen.push(update.state);
        }
        seen
    });
    tokio::task::yield_now().await;
    lab.client.shutdown_watches(Duration::from_secs(2)).await;
    let seen = tokio::time::timeout(wait, reader)
        .await
        .expect("the waiting next ended")
        .unwrap();
    assert!(
        !seen.iter().any(|state| matches!(state, State::Gone { .. })),
        "{seen:?}"
    );
    drop(device);
    hubs_gone(&lab).await;
    lab.stop().await;
}

/// The hub settings a watch's settings make: the request budget sets the
/// renewal margin, polls go no faster than a second, and polling only
/// sends no subscription.
#[test]
fn the_settings_make_the_hubs_options() {
    let settings = WatchSettings::default();
    assert_eq!(settings.lifetime, Lifetime::DEFAULT);
    assert_eq!(settings.poll_every, Duration::from_secs(10));
    assert!(settings.subscribe);
    // At the default timers (6 s, three retries) a request takes up to 24 s.
    let options = settings.options(Duration::from_secs(24));
    assert_eq!(options.renewal_margin, Duration::from_secs(48));
    // A COV retry waits 5 minutes, as the client hub's always did.
    assert_eq!(options.retry_cov, Backoff::fixed(Duration::from_secs(300)));
    assert_eq!(options.max_points, MAX_WATCHED);
    assert!(options.subscribe);
    assert!(options.validate().is_ok());
    let short = settings
        .clone()
        .with_lifetime(Lifetime::new(60).unwrap())
        .with_subscribe(false)
        .options(Duration::from_secs(2));
    assert_eq!(short.renewal_margin, Duration::from_secs(6));
    assert!(!short.subscribe);
    assert_ne!(settings, settings.clone().with_subscribe(false));
}
