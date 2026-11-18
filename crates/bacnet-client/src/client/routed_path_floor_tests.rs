//! The floor of what a routed path learns from a reason 4 (#2665): a request
//! whose largest frame is 228 NPDU octets or fewer never takes one, since a
//! conforming router does not refuse a frame that small as too long. Every
//! test runs on paused time at the client's default timing.

use tokio::time::{Duration, sleep};

use super::ClientConfig;
use super::routed_path_answered_tests::{LARGE, Path, WINDOW, timed_out, too_long};

const ROUTER: [u8; 1] = [2];
const DNET: u16 = super::routed_path_limit_tests::DNET;

/// The probe of #2665: one small frame, a reason 4 for other traffic to
/// the network. The request is not ended by it, the drop is counted, the
/// path learns nothing, and a later larger request goes out at once.
/// Without the floor the path learned the 16-octet NPDU and every later
/// request failed as too long.
#[tokio::test(start_paused = true)]
async fn a_reject_for_a_tiny_request_is_dropped_and_the_path_learns_nothing() {
    let mut path = Path::new(ClientConfig::default(), None).await;
    let task = path.request(1, 0x41);
    let frame = path.next_frame().await;
    assert_eq!(frame.service_request.len(), 1, "an NPDU of 16 octets");
    path.reject().await;
    sleep(Duration::from_millis(10)).await;
    assert!(!task.is_finished(), "the reject ended the tiny request");
    assert_eq!(path.client.below_minimum_reject_drops(), 1);
    path.ack(frame.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());

    path.next_goes_out_after(LARGE, Duration::ZERO, 217).await;
    path.stop().await;
}

/// The floor's edge, on a wide path: a largest frame of 228 NPDU octets
/// (213 of service data) drops the reject, and 229 takes it, learns, and
/// leaves the path at the 228-octet floor.
#[tokio::test(start_paused = true)]
async fn the_floor_is_228_octets_and_a_reject_for_229_learns_it() {
    let mut path = Path::new(ClientConfig::default(), Some(1497)).await;
    let task = path.request(213, 0x41);
    let frame = path.next_frame().await;
    assert_eq!(frame.service_request.len(), 213);
    path.reject().await;
    sleep(Duration::from_millis(10)).await;
    assert!(!task.is_finished(), "a reject ended a 228-octet frame");
    assert_eq!(path.client.below_minimum_reject_drops(), 1);
    path.ack(frame.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());

    let task = path.request(214, 0x42);
    path.next_frame().await;
    path.reject().await;
    assert!(too_long(task.await.unwrap()));
    assert_eq!(path.client.below_minimum_reject_drops(), 1);

    // The 229-octet NPDU bounds the path to 228: 217 APDU octets.
    let first = path.next_goes_out_after(LARGE, Duration::ZERO, 217).await;
    assert!(first.segmented);
    path.stop().await;
}

/// A limit the user set is not held to the floor: a router that really
/// carries less is configured, and requests are cut to it.
#[tokio::test(start_paused = true)]
async fn a_configured_limit_below_the_floor_is_honoured() {
    let mut path = Path::new(ClientConfig::default(), Some(128)).await;
    // 128 - 11 octets of NPCI: segments of 117 APDU octets, 111 of data.
    let first = path.next_goes_out_after(LARGE, Duration::ZERO, 117).await;
    assert!(first.segmented);
    assert_eq!(first.service_request.len(), 111);
    assert_eq!(path.client.below_minimum_reject_drops(), 0);
    path.stop().await;
}

/// What a router that really limits below the floor looks like without
/// the configured limit: its rejects are dropped, so the request ends by
/// its own timeout, not as too long.
#[tokio::test(start_paused = true)]
async fn a_router_limiting_below_the_floor_shows_as_timeouts() {
    let mut path = Path::new(ClientConfig::default(), None).await;
    let task = path.request(LARGE, 0x41);
    for _ in 0..4 {
        path.next_frame().await;
        path.reject().await;
    }
    assert!(timed_out(task.await.unwrap()));
    assert_eq!(path.client.below_minimum_reject_drops(), 4);
    path.stop().await;
}

/// A small request inside an answered request's window is below the floor
/// and below the recorded size: one reject, dropped once and counted once.
#[tokio::test(start_paused = true)]
async fn a_small_request_in_the_answered_window_drops_a_reject_once() {
    let mut path = Path::new(ClientConfig::default(), Some(1497)).await;
    path.answered_on_retry(LARGE).await;
    let task = path.request(1, 0x42);
    let frame = path.next_frame().await;
    path.reject().await;
    sleep(Duration::from_millis(10)).await;
    assert!(!task.is_finished());
    assert_eq!(path.client.below_minimum_reject_drops(), 1);
    path.ack(frame.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());
    path.next_goes_out_after(LARGE, Duration::ZERO, 1476).await;
    path.stop().await;
}

/// The order inside `claim_active` matters: a dropped reject must leave a
/// one-frame generation open, so a request that then times out still holds
/// its path for the whole window.
#[tokio::test(start_paused = true)]
async fn a_dropped_reject_then_a_timeout_holds_the_path_for_the_window() {
    let mut path = Path::new(ClientConfig::default(), None).await;
    let task = path.request(1, 0x41);
    path.next_frame().await;
    path.reject().await;
    for _ in 0..3 {
        path.next_frame().await;
    }
    assert!(timed_out(task.await.unwrap()));
    assert_eq!(path.client.below_minimum_reject_drops(), 1);
    path.next_goes_out_after(1, WINDOW, 217).await;
    path.stop().await;
}

/// A reject dropped after a segmented request's first 228-octet segment
/// leaves nothing learned, and the device's answer frees the path at once.
#[tokio::test(start_paused = true)]
async fn an_answer_after_a_dropped_reject_frees_the_path_at_once() {
    let mut path = Path::new(ClientConfig::default(), None).await;
    let task = path.request(LARGE, 0x41);
    let first = path.next_frame().await;
    assert!(first.segmented && first.more_follows);
    path.reject().await;
    sleep(Duration::from_millis(10)).await;
    assert!(!task.is_finished());
    assert_eq!(path.client.below_minimum_reject_drops(), 1);
    path.finish_segmented(first, 217).await;
    assert!(task.await.unwrap().unwrap().is_empty());
    path.next_goes_out_after(LARGE, Duration::ZERO, 217).await;
    path.stop().await;
}

/// The warning fires on the first drop on a path only, and again after the
/// path's limit is configured or cleared; the counter counts every drop.
#[tokio::test(start_paused = true)]
async fn the_warning_fires_once_per_path_until_its_limit_changes() {
    let mut path = Path::new(ClientConfig::default(), None).await;
    let limits = &path.client.routed_path_limits;
    assert!(!limits.below_minimum_warned(&ROUTER, DNET));
    for round in 1..=2u64 {
        let task = path.request(1, 0x41);
        let frame = path.next_frame().await;
        path.reject().await;
        sleep(Duration::from_millis(10)).await;
        assert!(
            path.client
                .routed_path_limits
                .below_minimum_warned(&ROUTER, DNET)
        );
        assert_eq!(path.client.below_minimum_reject_drops(), round);
        path.ack(frame.invoke_id).await;
        assert!(task.await.unwrap().unwrap().is_empty());
    }
    path.client
        .configure_routed_path_max_npdu(&ROUTER, DNET, 128)
        .await
        .unwrap();
    assert!(
        !path
            .client
            .routed_path_limits
            .below_minimum_warned(&ROUTER, DNET)
    );
    path.stop().await;
}

/// A path configured below the floor keeps its limit: a reason 4 for a
/// frame within the floor is dropped and counted, and the limit stays.
#[tokio::test(start_paused = true)]
async fn a_reject_on_a_path_configured_below_the_floor_is_dropped_and_the_limit_stays() {
    let mut path = Path::new(ClientConfig::default(), Some(128)).await;
    let task = path.request(100, 0x41);
    let frame = path.next_frame().await;
    path.reject().await;
    sleep(Duration::from_millis(10)).await;
    assert!(!task.is_finished());
    assert_eq!(path.client.below_minimum_reject_drops(), 1);
    assert_eq!(path.client.too_long_reject_drops(), 0);
    path.ack(frame.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());
    // Still 128: segments of 117 APDU octets.
    let first = path.next_goes_out_after(LARGE, Duration::ZERO, 117).await;
    assert_eq!(first.service_request.len(), 111);
    path.stop().await;
}
