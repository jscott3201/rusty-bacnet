//! The fix round of #2115: the first notification after a subscription,
//! shutdown keeping to its wait, a scheduler's end racing an arrival, and
//! a one-row batch too large to answer; and from #2115's second PR, the
//! cancel of a point given up and a renewal behind a busy gate.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::Instant;

use super::test_lab::{LAB, Lab, RP, RPM, SUBSCRIBE_COV, ai, nc, object, settle};
use super::*;

/// The device notifies as soon as it takes a subscription, which can be
/// before the start job is back with its scheduler: the receiver must
/// already know where the device is. Real threads, so the race can land.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_first_notification_after_subscribing_is_kept() {
    let lab = Lab::start(WatchOptions::default()).await;
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let snapshot = lease.snapshot();
    assert_eq!(snapshot.source, Some(Source::Cov), "{snapshot:?}");
    assert!(snapshot.notifications >= 1, "{snapshot:?}");
    drop(lease);
    lab.stop().await;
}

/// A poll in flight doesn't hold shutdown past its wait: once every point
/// is done, what a scheduler still runs is aborted.
#[tokio::test(start_paused = true)]
async fn shutdown_keeps_to_its_wait_with_a_poll_in_flight() {
    let lab = Lab::start(WatchOptions::default()).await;
    let lease = lab.hub.watch(object(nc(1))).unwrap();
    settle().await;
    tokio::time::sleep(Duration::from_millis(9_800)).await;
    lab.quirks.muted.store(true, Ordering::Relaxed);
    let from = Instant::now();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(lab.sent(RPM, from), 1, "a poll is waiting for its answer");
    let start = Instant::now();
    assert!(lab.hub.shutdown(Duration::from_millis(200)).await);
    let took = start.elapsed();
    assert!(took <= Duration::from_millis(250), "shutdown took {took:?}");
    assert_eq!(lab.hub.stats().cancelling, 0);
    drop(lease);
    lab.quirks.muted.store(false, Ordering::Relaxed);
    lab.stop().await;
}

/// A point arriving just as its device's scheduler, with nothing left,
/// goes to end is still served: the end is decided under the table's lock.
#[tokio::test(start_paused = true)]
async fn an_arrival_as_the_scheduler_ends_is_served() {
    let lab = Lab::start(WatchOptions::default()).await;
    let first = lab.hub.watch(object(nc(1))).unwrap();
    settle().await;
    let late: Arc<Mutex<Option<Lease>>> = Arc::new(Mutex::new(None));
    let hub = lab.hub.clone();
    let slot = Arc::clone(&late);
    *lab.hub.inner.registry.before_end.lock().unwrap() = Some(Box::new(move || {
        *slot.lock().unwrap() = Some(hub.watch(object(ai(1))).unwrap());
    }));
    drop(first);
    settle().await;
    let late = late.lock().unwrap().take().expect("the hook ran");
    let snapshot = late.snapshot();
    assert_eq!(snapshot.source, Some(Source::Cov), "{snapshot:?}");
    assert_eq!(snapshot.state, State::Live);
    assert_eq!(lab.table().await.len(), 1);
    drop(late);
    lab.stop().await;
}

/// A one-row batch the device still can't answer isn't asked again: that
/// poll reads the row property by property.
#[tokio::test(start_paused = true)]
async fn a_row_too_large_alone_is_read_property_by_property() {
    let lab = Lab::start(WatchOptions::default()).await;
    let lease = lab.hub.watch(object(nc(1))).unwrap();
    settle().await;
    lab.quirks.abort_rpm.store(2, Ordering::Relaxed);
    let from = Instant::now();
    tokio::time::sleep(Duration::from_secs(11)).await;
    assert_eq!(lab.sent(RPM, from), 1);
    assert_eq!(lab.sent(RP, from), POINT_PROPERTIES.len());
    let snapshot = lease.snapshot();
    assert_eq!((snapshot.polls, snapshot.state), (1, State::Live));
    lab.stop().await;
}

/// Under [`MissingDevice::Fail`], a point whose device leaves the table
/// between attempts is given up, and the subscription an earlier attempt
/// may have made is cancelled. A shutdown inside that cancel's window
/// waits for it: the cancel is still sent.
#[tokio::test(start_paused = true)]
async fn shutdown_waits_for_the_cancel_of_a_point_given_up() {
    let options = WatchOptions {
        missing: MissingDevice::Fail,
        poll_floor: Duration::from_secs(1),
        retry_cov: Backoff::fixed(Duration::from_secs(2)),
        ..WatchOptions::default()
    };
    let lab = Lab::start(options).await;
    lab.quirks.swallow_cov.store(1, Ordering::Relaxed);
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    assert_eq!(lab.services(), [RPM, SUBSCRIBE_COV], "unanswered");
    lab.engine.client().clear_devices().await;
    // Another request holds the device's gate, so the cancel waits.
    let held = lab.engine.limits().turn(LAB).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(lease.snapshot().state, State::Gone);
    assert_eq!(lease.snapshot().why, Some(Why::Missing));
    assert_eq!(lab.hub.stats().cancelling, 1);
    let hub = lab.hub.clone();
    let shutdown = tokio::spawn(async move { hub.shutdown(Duration::from_secs(5)).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(held);
    assert!(shutdown.await.unwrap());
    assert_eq!(
        lab.services(),
        [RPM, SUBSCRIBE_COV, SUBSCRIBE_COV],
        "the cancel went"
    );
    assert_eq!(lab.hub.stats().cancelling, 0);
    drop(lease);
    lab.stop().await;
}

/// A renewal queued behind a request that holds the device's gate for a
/// whole budget still lands a budget before the lifetime ends (#1791; port
/// of the TUI's margin test).
#[tokio::test(start_paused = true)]
async fn a_renewal_behind_a_full_budget_request_lands_before_expiry() {
    let budget = Duration::from_secs(24);
    let lifetime = Duration::from_secs(120);
    let margin = renewal_margin(120, budget);
    let options = WatchOptions {
        lifetime: Lifetime::new(120).unwrap(),
        renewal_margin: margin,
        ..WatchOptions::default()
    };
    let lab = Lab::start(options).await;
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    let start = lab.sent_at(SUBSCRIBE_COV)[0];
    let due = start + lifetime - margin;
    tokio::time::sleep_until(due - Duration::from_secs(1)).await;
    let held = lab.engine.limits().turn(LAB).await;
    tokio::time::sleep(budget).await;
    drop(held);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let renewed = *lab.sent_at(SUBSCRIBE_COV).get(1).expect("renewed");
    assert!(
        renewed + budget <= start + lifetime,
        "renewal at {:?} leaves less than a budget before expiry",
        renewed - start
    );
    assert_eq!(lease.snapshot().renewals, 1);
    lab.stop().await;
}
