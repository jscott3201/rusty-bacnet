//! A change of a routed path's limit while the path is in use (#2683): it
//! applies at once, ahead of a request waiting out the hold a request with
//! no answer left (#2651), and of one already in flight. The hold is
//! neither shortened nor restarted, a reason 4 in it is still discarded,
//! and requests still go through the path one at a time, in the order they
//! came. Every test runs on paused time at the client's default timing,
//! where the hold is 24 s.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use bacnet_types::error::Error;
use tokio::time::{Duration, Instant, sleep};

use super::ClientConfig;
use super::routed_path_answered_tests::{LARGE, Path, WINDOW, timed_out, too_long};
use super::routed_path_limit_tests::DNET;
use super::routed_path_limits::RoutedPathLimits;
use super::routed_path_listing_tests::{ROUTER, listed};

/// A request with no answer leaves its path held for the window (the APDU
/// timeout times the attempts): returns the client when it has ended, which
/// is when the hold starts.
async fn timed_out_path() -> Path {
    let mut path = Path::new(ClientConfig::default(), None).await;
    let task = path.request(1, 0x41);
    for _ in 0..4 {
        path.next_frame().await;
    }
    assert!(timed_out(task.await.unwrap()));
    path
}

/// A change of the limit after a timeout applies at once instead of waiting
/// out the hold, and leaves the hold as it was: made 10 s into it, the next
/// request still goes out when the hold ends, 10 s short of a full window,
/// sized to the new limit (480 octets: 469 of APDU, so 304 go out whole,
/// where the default would segment them).
#[tokio::test(start_paused = true)]
async fn a_change_after_a_timeout_applies_at_once_and_the_hold_stays() {
    let mut path = timed_out_path().await;
    let into = Duration::from_secs(10);
    sleep(into).await;
    path.client
        .configure_routed_path_max_npdu(&ROUTER, DNET, 300)
        .unwrap();
    path.client.clear_routed_path_limit(&ROUTER, DNET).unwrap();
    path.client
        .configure_routed_path_max_npdu(&ROUTER, DNET, 480)
        .unwrap();
    let first = path.next_goes_out_after(LARGE, WINDOW - into, 469).await;
    assert!(!first.segmented);
    path.stop().await;
}

/// The issue's case: a request already waiting out the hold holds the path,
/// and a limit set then applies ahead of it. The request goes out when the
/// hold ends, not sooner, sized to the new limit: 304 octets whole, where
/// the old 228 would have sent segments of 217.
#[tokio::test(start_paused = true)]
async fn a_change_applies_ahead_of_a_request_waiting_out_the_hold() {
    let mut path = timed_out_path().await;
    let held = Instant::now();
    let queued = path.request(LARGE, 0x42);
    sleep(Duration::from_millis(1)).await;
    path.client
        .configure_routed_path_max_npdu(&ROUTER, DNET, 480)
        .unwrap();
    assert_eq!(
        path.client.routed_path_limits(),
        [listed(Some(480), None, false)]
    );
    let first = path.next_frame().await;
    assert_eq!(
        held.elapsed(),
        WINDOW,
        "the hold was shortened or restarted"
    );
    assert!(!first.segmented, "sized by the old limit");
    assert_eq!(first.service_request[0], 0x42);
    path.ack(first.invoke_id).await;
    assert!(queued.await.unwrap().unwrap().is_empty());
    path.stop().await;
}

/// Two requests waiting out a hold go in the order they came. The first
/// goes unanswered, so the hold starts again while the second waits; the
/// second goes a full window after that, sized to a limit set in the second
/// hold.
#[tokio::test(start_paused = true)]
async fn waiters_go_in_order_and_one_waits_out_a_hold_that_starts_again() {
    let mut path = timed_out_path().await;
    let held = Instant::now();
    let first = path.request(1, 0x41);
    sleep(Duration::from_millis(1)).await;
    let second = path.request(LARGE, 0x42);
    sleep(Duration::from_millis(1)).await;

    let frame = path.next_frame().await;
    assert_eq!(held.elapsed(), WINDOW);
    assert_eq!(
        frame.service_request[0], 0x41,
        "the first request goes first"
    );
    for _ in 0..3 {
        assert_eq!(path.next_frame().await.invoke_id, frame.invoke_id);
    }
    assert!(timed_out(first.await.unwrap()));

    let held_again = Instant::now();
    sleep(Duration::from_secs(1)).await;
    path.client
        .configure_routed_path_max_npdu(&ROUTER, DNET, 480)
        .unwrap();
    let frame = path.next_frame().await;
    assert_eq!(
        held_again.elapsed(),
        WINDOW,
        "the second waited the new hold"
    );
    assert_eq!(frame.service_request[0], 0x42);
    assert!(
        !frame.segmented,
        "sized to the limit set in the second hold"
    );
    path.ack(frame.invoke_id).await;
    assert!(second.await.unwrap().unwrap().is_empty());
    path.stop().await;
}

/// A request cancelled while it waits out the hold leaves the hold as it
/// was: the next request goes out when it ends.
#[tokio::test(start_paused = true)]
async fn a_request_cancelled_in_the_hold_leaves_the_hold_alone() {
    let mut path = timed_out_path().await;
    let into = Duration::from_secs(5);
    let cancelled = path.request(LARGE, 0x42);
    sleep(into).await;
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    path.client
        .configure_routed_path_max_npdu(&ROUTER, DNET, 480)
        .unwrap();
    let first = path.next_goes_out_after(LARGE, WINDOW - into, 469).await;
    assert!(!first.segmented);
    path.stop().await;
}

/// A change while a request is in flight applies at once; the request
/// keeps the size it went out at (304 octets whole, under 1497), and a
/// reason 4 for it fails it but teaches nothing: before #2683 it would
/// have learned 314 against the new limit.
#[tokio::test(start_paused = true)]
async fn a_change_while_a_request_is_in_flight_learns_nothing_from_its_reject() {
    let mut path = Path::new(ClientConfig::default(), Some(1497)).await;
    let task = path.request(LARGE, 0x41);
    let frame = path.next_frame().await;
    assert!(!frame.segmented);
    path.client
        .configure_routed_path_max_npdu(&ROUTER, DNET, 480)
        .unwrap();
    path.reject().await;
    assert!(too_long(task.await.unwrap()));
    assert_eq!(
        path.client.routed_path_limits(),
        [listed(Some(480), None, false)]
    );
    path.stop().await;
}

/// A below-minimum reject for a request sized before a change is counted,
/// but doesn't mark the path whose limit was just set.
#[tokio::test(start_paused = true)]
async fn a_below_minimum_reject_sized_before_a_change_leaves_no_mark() {
    let mut path = Path::new(ClientConfig::default(), None).await;
    let task = path.request(1, 0x41);
    let frame = path.next_frame().await;
    path.client
        .configure_routed_path_max_npdu(&ROUTER, DNET, 128)
        .unwrap();
    path.reject().await;
    sleep(Duration::from_millis(10)).await;
    assert_eq!(path.client.below_minimum_reject_drops(), 1);
    assert_eq!(
        path.client.routed_path_limits(),
        [listed(Some(128), None, false)]
    );
    path.ack(frame.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());
    path.stop().await;
}

/// A clear while a request is in flight works as a configure does: the
/// request keeps its size, and the reject for it fails it but teaches the
/// cleared path nothing (it would have learned 314).
#[tokio::test(start_paused = true)]
async fn a_clear_while_a_request_is_in_flight_learns_nothing_from_its_reject() {
    let mut path = Path::new(ClientConfig::default(), Some(1497)).await;
    let task = path.request(LARGE, 0x41);
    let frame = path.next_frame().await;
    assert!(!frame.segmented);
    path.client.clear_routed_path_limit(&ROUTER, DNET).unwrap();
    path.reject().await;
    assert!(too_long(task.await.unwrap()));
    assert!(path.client.routed_path_limits().is_empty());
    path.stop().await;
}

/// A below-minimum reject for a request sized before a clear is counted,
/// but doesn't mark the path just put back on the default.
#[tokio::test(start_paused = true)]
async fn a_below_minimum_reject_sized_before_a_clear_leaves_no_mark() {
    let mut path = Path::new(ClientConfig::default(), Some(1497)).await;
    let task = path.request(1, 0x41);
    let frame = path.next_frame().await;
    path.client.clear_routed_path_limit(&ROUTER, DNET).unwrap();
    path.reject().await;
    sleep(Duration::from_millis(10)).await;
    assert_eq!(path.client.below_minimum_reject_drops(), 1);
    assert!(path.client.routed_path_limits().is_empty());
    path.ack(frame.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());
    path.stop().await;
}

/// A reason 4 that lands in the hold after a change finds no request
/// active on the path: it is discarded, uncounted, and neither the new
/// limit nor a learned one moves, as #2651's hold has it for any reject
/// there. The next request goes out after the rest of the hold at the new
/// limit.
#[tokio::test(start_paused = true)]
async fn a_reject_in_the_hold_after_a_change_is_discarded() {
    let mut path = timed_out_path().await;
    path.client
        .configure_routed_path_max_npdu(&ROUTER, DNET, 480)
        .unwrap();
    path.reject().await;
    sleep(Duration::from_millis(10)).await;
    assert_eq!(
        (
            path.client.below_minimum_reject_drops(),
            path.client.too_long_reject_drops()
        ),
        (0, 0)
    );
    assert_eq!(
        path.client.routed_path_limits(),
        [listed(Some(480), None, false)]
    );
    let first = path
        .next_goes_out_after(LARGE, WINDOW - Duration::from_millis(10), 469)
        .await;
    assert!(!first.segmented);
    path.stop().await;
}

/// A clear checks for the path's entry and clears it under one lock, so a
/// request to a new path reclaiming that entry on a full table never makes
/// the clear fail for capacity (#2667). One task keeps taking new paths
/// through a one-entry table while another recreates the path and clears
/// it. The one-lock shape is what makes the race impossible; this test only
/// guards against a return to a check and a reservation in two steps, and
/// catches that in some runs, not every one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_clear_racing_a_reclaim_never_reports_capacity() {
    let limits = Arc::new(RoutedPathLimits::with_capacity(
        1,
        StdDuration::from_millis(1),
    ));
    let churn = {
        let limits = Arc::clone(&limits);
        tokio::spawn(async move {
            for round in 0..2_000u16 {
                // A path the clear never names; busy only for a moment.
                if let Ok(lease) = limits.acquire(&[9], 1 + round % 100).await {
                    drop(lease);
                }
                tokio::task::yield_now().await;
            }
        })
    };
    for _ in 0..2_000 {
        if let Ok(lease) = limits.acquire(&ROUTER, DNET).await {
            drop(lease);
        }
        match limits.clear(&ROUTER, DNET) {
            Ok(()) => {}
            Err(Error::RoutedPathCapacityExceeded { .. }) => {
                panic!("a clear reported capacity for a path it had nothing to clear on")
            }
            Err(other) => panic!("{other:?}"),
        }
        tokio::task::yield_now().await;
    }
    churn.await.unwrap();
}
