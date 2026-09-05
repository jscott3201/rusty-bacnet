//! Explicit-time tests of the production expiry boundary, without a test clock.

use super::*;
use crate::server::segmented_receive::expire_segmented_requests;
use std::sync::atomic::AtomicUsize;

struct RetainedPayload {
    data: [u8; 8],
    drops: Arc<AtomicUsize>,
}

impl AsRef<[u8]> for RetainedPayload {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl Drop for RetainedPayload {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

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
    let mut receiver = SegmentReceiver::new();
    receiver
        .receive(0, first_req.service_request.clone())
        .unwrap();
    SegmentedRequestState {
        receiver,
        first_req,
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
    let mut stale = saved_state(now - Duration::from_secs(16), now);
    // Two owners: the initial payload shared by first_req and the receiver,
    // and a later segment owned only by the receiver. Both must drop now.
    stale.first_req.service_request = Bytes::from_owner(RetainedPayload {
        data: [1; 8],
        drops: Arc::clone(&drops),
    });
    stale
        .receiver
        .receive(0, stale.first_req.service_request.clone())
        .unwrap();
    stale
        .receiver
        .receive(
            1,
            Bytes::from_owner(RetainedPayload {
                data: [2; 8],
                drops: Arc::clone(&drops),
            }),
        )
        .unwrap();
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
        receivers[&fresh_key].receiver.reassemble(1).unwrap(),
        b"first"
    );
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
}
