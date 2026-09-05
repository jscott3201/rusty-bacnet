//! Explicit-time tests of the production expiry boundary, without a test clock.

use super::*;
use crate::server::segmented_receive::{
    expire_segmented_requests, tests::observe_payload_drops, RequestPayload,
};
use std::sync::atomic::AtomicUsize;

fn saved_state(last_progress: Instant, last_activity: Instant) -> SegmentedRequestState {
    let first_req = ConfirmedRequestPdu {
        segmented: true,
        more_follows: true,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 0,
        sequence_number: Some(0),
        proposed_window_size: Some(3),
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: Bytes::from_static(b"first"),
    };
    let mut payload = RequestPayload::new(&first_req);
    payload
        .save_new(0, first_req.service_request.clone(), Some(0))
        .unwrap();
    SegmentedRequestState {
        payload,
        last_activity,
        last_progress,
        expected_seq: 1,
        initial_sequence_number: 0,
        duplicate_count: 0,
        last_acked_seq: 0,
        window_pos: 1,
        actual_window_size: 3,
        accepted_segments: 1,
    }
}

#[test]
fn request_progress_expiry_before_exact_and_after_16_seconds() {
    let start = Instant::now();
    let key = (test_mac(1), None, 0);
    for (elapsed, survives) in [
        (Duration::from_nanos(15_999_999_999), true),
        (Duration::from_secs(16), false),
        (Duration::from_nanos(16_000_000_001), false),
    ] {
        let now = start + elapsed;
        // Activity is fresh even at the exact progress expiry boundary.
        let mut receivers = HashMap::from([(key.clone(), saved_state(start, now))]);
        expire_segmented_requests(&mut receivers, now);
        assert_eq!(
            receivers.contains_key(&key),
            survives,
            "elapsed {elapsed:?}"
        );
    }
}

#[test]
fn request_progress_expiry_preserves_exact_4_second_inactivity_boundary() {
    let now = Instant::now();
    let key = (test_mac(1), None, 0);
    for (idle, survives) in [
        (Duration::from_nanos(3_999_999_999), true),
        (Duration::from_secs(4), false),
        (Duration::from_nanos(4_000_000_001), false),
    ] {
        let mut receivers = HashMap::from([(key.clone(), saved_state(now, now - idle))]);
        expire_segmented_requests(&mut receivers, now);
        assert_eq!(receivers.contains_key(&key), survives, "idle {idle:?}");
    }
}

#[test]
fn request_progress_expiry_keeps_mixed_fresh_survivors_and_releases_all_payload_owners() {
    let now = Instant::now();
    let drops = Arc::new(AtomicUsize::new(0));
    let probe = observe_payload_drops(&drops);
    let mut stale = saved_state(now - Duration::from_secs(16), now);
    // Observe actual detached first/later allocations through real saves, not
    // raw receiver replacement or retention of the original input owner.
    stale
        .payload
        .save_new(1, Bytes::from_static(&[2; 8]), Some(5))
        .unwrap();
    stale.accepted_segments = 2;
    stale.expected_seq = 2;
    drop(probe);
    let stale_key = (test_mac(1), None, 0);
    let fresh_key = (test_mac(2), None, 0);
    let idle_key = (test_mac(3), None, 0);
    let mut receivers = HashMap::from([
        (stale_key.clone(), stale),
        (
            fresh_key.clone(),
            saved_state(now - Duration::from_secs(15), now),
        ),
        (
            idle_key.clone(),
            saved_state(now, now - Duration::from_secs(4)),
        ),
    ]);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    expire_segmented_requests(&mut receivers, now);
    assert!(!receivers.contains_key(&stale_key));
    assert!(!receivers.contains_key(&idle_key));
    assert_eq!(receivers.len(), 1);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        2,
        "cleanup must drop ownership synchronously"
    );
    expire_segmented_requests(&mut receivers, now);
    assert_eq!(receivers.len(), 1);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        2,
        "repeated cleanup is harmless"
    );
    let fresh = receivers
        .remove(&fresh_key)
        .unwrap()
        .payload
        .complete(1)
        .unwrap();
    assert_eq!(fresh.service_request.as_ref(), b"first");
}
