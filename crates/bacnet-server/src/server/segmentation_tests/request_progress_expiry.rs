//! Explicit-time tests of the production expiry boundary, without a test clock.

use super::*;
use crate::server::segmented_receive::{
    expire_segmented_requests, tests::observe_payload_drops, RequestPayload,
};
use bacnet_transport::port::TransportProvenance;
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
        source_mac: MacAddr::new(),
        source_network: None,
        direct_response: None,
        payload,
        provenance: TransportProvenance::unverified(),
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
    let key = (test_mac(1), None, 0, TransportProvenance::unverified());
    for (elapsed, survives) in [
        (Duration::from_nanos(15_999_999_999), true),
        (Duration::from_secs(16), false),
        (Duration::from_nanos(16_000_000_001), false),
    ] {
        let now = start + elapsed;
        // Activity is fresh even at the exact progress expiry boundary.
        let mut receivers = HashMap::from([(key.clone(), saved_state(start, now))]);
        expire_segmented_requests(&mut receivers, now, Duration::from_secs(4));
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
    let key = (test_mac(1), None, 0, TransportProvenance::unverified());
    for (idle, survives) in [
        (Duration::from_nanos(3_999_999_999), true),
        (Duration::from_secs(4), true),
        (Duration::from_nanos(4_000_000_001), false),
    ] {
        let mut receivers = HashMap::from([(key.clone(), saved_state(now, now - idle))]);
        expire_segmented_requests(&mut receivers, now, Duration::from_secs(4));
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
    let stale_key = (test_mac(1), None, 0, TransportProvenance::unverified());
    let fresh_key = (test_mac(2), None, 0, TransportProvenance::unverified());
    let idle_key = (test_mac(3), None, 0, TransportProvenance::unverified());
    let mut receivers = HashMap::from([
        (stale_key.clone(), stale),
        (
            fresh_key.clone(),
            saved_state(now - Duration::from_secs(15), now),
        ),
        (
            idle_key.clone(),
            saved_state(now, now - Duration::from_secs(4) - Duration::from_nanos(1)),
        ),
    ]);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    expire_segmented_requests(&mut receivers, now, Duration::from_secs(4));
    assert!(!receivers.contains_key(&stale_key));
    assert!(!receivers.contains_key(&idle_key));
    assert_eq!(receivers.len(), 1);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        2,
        "cleanup must drop ownership synchronously"
    );
    expire_segmented_requests(&mut receivers, now, Duration::from_secs(4));
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

#[test]
fn receive_and_progress_expiry_precedence_and_next_wakeup() {
    use crate::server::segmented_receive::next_receive_deadline;
    let now = Instant::now();
    let key = (test_mac(1), None, 0, TransportProvenance::unverified());
    // If both timers are overdue, protocol expiry is silent. At their exact
    // common boundary only the inclusive local cap is due, hence OTHER.
    for (elapsed, expected_aborts) in [
        (Duration::from_secs(16), 1),
        (Duration::from_secs(16) + Duration::from_nanos(1), 0),
    ] {
        let mut receivers = HashMap::from([(key.clone(), saved_state(now, now))]);
        assert_eq!(
            next_receive_deadline(&receivers, Duration::from_secs(16)),
            Some(now + Duration::from_secs(16))
        );
        let aborts =
            expire_segmented_requests(&mut receivers, now + elapsed, Duration::from_secs(16));
        assert_eq!(aborts.len(), expected_aborts);
        assert!(receivers.is_empty());
        assert_eq!(
            next_receive_deadline(&receivers, Duration::from_secs(16)),
            None
        );
    }
    let receivers = HashMap::from([(key, saved_state(now, now))]);
    assert_eq!(
        next_receive_deadline(&receivers, Duration::from_secs(4)),
        Some(now + Duration::from_secs(4) + Duration::from_nanos(1))
    );
}

#[tokio::test]
async fn progress_expiry_drops_all_payload_before_blocked_abort_and_survives_send_failure() {
    let now = Instant::now();
    let drops = Arc::new(AtomicUsize::new(0));
    let probe = observe_payload_drops(&drops);
    let mut receivers = HashMap::new();
    for invoke in 0..3 {
        let key = (test_mac(1), None, invoke, TransportProvenance::unverified());
        let mut state = saved_state(now - Duration::from_secs(16), now);
        state.source_mac = test_mac(1);
        receivers.insert(key, state);
    }
    drop(probe);
    let (network, sent, control) = blocking_first_send_network();
    control.fail_next_send();
    let reaping =
        BACnetServer::reap_expired_requests(&network, &mut receivers, now, Duration::from_secs(20));
    tokio::pin!(reaping);
    tokio::select! {
        () = &mut reaping => panic!("first Abort should block"),
        () = control.wait_blocked() => {}
    }
    assert_eq!(
        drops.load(Ordering::SeqCst),
        3,
        "all payload must drop before first send awaits"
    );
    control.release_sends(1);
    reaping.await;
    assert_eq!(
        sent_count(&sent),
        3,
        "failed first Abort does not prevent the others"
    );
    for index in 0..3 {
        assert_eq!(abort_reason(&sent, index), AbortReason::OTHER);
    }
}
