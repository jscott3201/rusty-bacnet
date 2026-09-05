//! Encapsulated saved-allocation observation: wraps the actual detached Bytes,
//! not incoming storage or an unrelated lifetime sentinel. Only test builds.

use super::*;
use std::{cell::RefCell, sync::atomic::AtomicUsize};

thread_local! {
    static SAVED_DROPS: RefCell<Option<Arc<AtomicUsize>>> = const { RefCell::new(None) };
}

pub(in crate::server) struct SavedPayloadProbe;

impl Drop for SavedPayloadProbe {
    fn drop(&mut self) {
        SAVED_DROPS.with(|probe| *probe.borrow_mut() = None);
    }
}

// Tokio wire tests using this seam run on current_thread; dispatch inherits
// the probe without sharing it with concurrently running test threads.
pub(in crate::server) fn observe_payload_drops(drops: &Arc<AtomicUsize>) -> SavedPayloadProbe {
    SAVED_DROPS.with(|probe| {
        assert!(probe.borrow().is_none(), "nested saved-payload probe");
        *probe.borrow_mut() = Some(Arc::clone(drops));
    });
    SavedPayloadProbe
}

struct ObservedPayload {
    bytes: Bytes,
    drops: Arc<AtomicUsize>,
}

impl AsRef<[u8]> for ObservedPayload {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl Drop for ObservedPayload {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

pub(super) fn observe_saved_payload(bytes: Bytes) -> Bytes {
    SAVED_DROPS.with(|probe| match probe.borrow().as_ref() {
        Some(drops) => Bytes::from_owner(ObservedPayload {
            bytes,
            drops: Arc::clone(drops),
        }),
        None => bytes,
    })
}

fn request(data: Bytes) -> ConfirmedRequestPdu {
    ConfirmedRequestPdu {
        segmented: true,
        more_follows: true,
        segmented_response_accepted: false,
        max_segments: Some(16),
        max_apdu_length: 480,
        invoke_id: 23,
        sequence_number: Some(0),
        proposed_window_size: Some(3),
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: data,
    }
}

fn state(now: Instant, first: &[u8]) -> SegmentedRequestState {
    let req = request(Bytes::copy_from_slice(first));
    let mut payload = RequestPayload::new(&req);
    payload.save_new(0, req.service_request, Some(0)).unwrap();
    SegmentedRequestState {
        payload,
        last_activity: now,
        last_progress: now,
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
fn request_payload_owner_counts_first_once_and_consumes_byte_exact_metadata() {
    let drops = Arc::new(AtomicUsize::new(0));
    let _probe = observe_payload_drops(&drops);
    let req = request(Bytes::from_static(b"first"));
    let mut payload = RequestPayload::new(&req);
    assert!(
        payload.first.service_request.is_empty(),
        "template is metadata only"
    );
    assert_eq!(payload.saved_payload_bytes(), 0);
    payload
        .save_new(0, req.service_request.clone(), Some(0))
        .unwrap();
    assert_eq!(payload.saved_payload_bytes(), 5);
    payload
        .save_new(1, Bytes::from_static(b"last"), Some(5))
        .unwrap();
    assert_eq!(payload.saved_payload_bytes(), 9);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let complete = payload.complete(2).unwrap();
    assert_eq!(
        drops.load(Ordering::SeqCst),
        2,
        "actual saved owners released before returning output"
    );
    assert_eq!(complete.service_request.as_ref(), b"firstlast");
    assert_eq!(complete.invoke_id, req.invoke_id);
    assert_eq!(complete.service_choice, req.service_choice);
    assert_eq!(complete.max_apdu_length, req.max_apdu_length);
    assert_eq!(complete.max_segments, req.max_segments);
    assert_eq!(
        complete.segmented_response_accepted,
        req.segmented_response_accepted
    );
    assert!(!complete.segmented && !complete.more_follows);
    assert_eq!(complete.sequence_number, None);
    assert_eq!(complete.proposed_window_size, None);
}

#[test]
fn request_payload_owner_failed_validation_and_append_only_invariant_are_uncharged() {
    let saved_drops = Arc::new(AtomicUsize::new(0));
    let _probe = observe_payload_drops(&saved_drops);
    let req = request(Bytes::new());
    let mut payload = RequestPayload::new(&req);
    let input_drops = Arc::new(AtomicUsize::new(0));
    let oversized = Bytes::from_owner(ObservedPayload {
        bytes: Bytes::from(vec![1; 1024 * 1024]),
        drops: Arc::clone(&input_drops),
    });
    assert!(payload.save_new(0, oversized, Some(0)).is_err());
    assert_eq!(input_drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        saved_drops.load(Ordering::SeqCst),
        0,
        "invalid payload never detached/stored"
    );
    assert_eq!(payload.receiver.received_count(), 0);
    assert_eq!(payload.saved_payload_bytes(), 0);
    payload
        .save_new(0, Bytes::from_static(b"first"), Some(0))
        .unwrap();
    for (seq, data, aggregate) in [
        (0, Bytes::from_static(b"overwrite"), Some(5)),
        (2, Bytes::from_static(b"gap"), Some(5)),
        (1, Bytes::from(vec![2; 1477]), Some(5)),
        (1, Bytes::from_static(b"x"), Some(4 * 1024 * 1024)),
        (1, Bytes::new(), None),
    ] {
        assert!(payload.save_new(seq, data, aggregate).is_err());
        assert_eq!(payload.saved_payload_bytes(), 5);
        assert_eq!(payload.receiver.received_count(), 1);
        assert_eq!(saved_drops.load(Ordering::SeqCst), 0);
    }
    payload
        .save_new(1, Bytes::new(), Some(4 * 1024 * 1024))
        .unwrap();
    assert_eq!(payload.saved_payload_bytes(), 5);
    assert_eq!(payload.receiver.received_count(), 2);
    assert_eq!(
        payload.complete(2).unwrap().service_request.as_ref(),
        b"first"
    );
}

#[test]
fn request_payload_owner_checked_aggregate_and_charge_fail_closed() {
    assert!(payload_fits(Some(4 * 1024 * 1024 - 1), 1));
    assert!(!payload_fits(Some(4 * 1024 * 1024 - 1), 2));
    assert!(payload_fits(Some(4 * 1024 * 1024), 0));
    assert!(!payload_fits(Some(4 * 1024 * 1024 + 1), 0));
    assert!(!payload_fits(Some(usize::MAX), 1));
    assert!(!payload_fits(None, 0));
    let now = Instant::now();
    let mut receivers = HashMap::from([
        ((MacAddr::new(), None, 0), state(now, b"abc")),
        ((MacAddr::new(), None, 1), state(now, b"defgh")),
    ]);
    assert_eq!(saved_request_payload_bytes(&receivers), Some(8));
    // Deliberately corrupt only private counts to reach arithmetic errors that
    // the production 128 x 256 x 1476 bounds otherwise make unreachable.
    let payload = &mut receivers
        .get_mut(&(MacAddr::new(), None, 0))
        .unwrap()
        .payload;
    payload.saved_payload_bytes = usize::MAX;
    assert!(payload
        .save_new(1, Bytes::from_static(b"x"), Some(0))
        .is_err());
    assert_eq!(payload.receiver.received_count(), 1);
    assert_eq!(payload.saved_payload_bytes(), usize::MAX);
    assert_eq!(saved_request_payload_bytes(&receivers), None);
}

#[test]
fn request_payload_owner_missing_reassembly_and_owner_destruction_release_storage() {
    let drops = Arc::new(AtomicUsize::new(0));
    let _probe = observe_payload_drops(&drops);
    let now = Instant::now();
    let missing = state(now, b"first");
    assert!(missing.payload.complete(2).is_err());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let saved = state(now, b"next");
    drop(saved);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn request_payload_owner_expiry_returns_capacity_without_refund_bookkeeping() {
    for progress in [false, true] {
        let drops = Arc::new(AtomicUsize::new(0));
        let _probe = observe_payload_drops(&drops);
        let now = Instant::now();
        let old = if progress {
            Duration::from_secs(16)
        } else {
            Duration::from_secs(4)
        };
        let mut stale = state(now, b"stale");
        if progress {
            stale.last_progress = now - old;
        } else {
            stale.last_activity = now - old;
        }
        let fresh_key = (MacAddr::new(), None, 1);
        let mut receivers = HashMap::from([
            ((MacAddr::new(), None, 0), stale),
            (fresh_key.clone(), state(now, b"fresh")),
        ]);
        assert_eq!(saved_request_payload_bytes(&receivers), Some(10));
        expire_segmented_requests(&mut receivers, now);
        assert_eq!(saved_request_payload_bytes(&receivers), Some(5));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(receivers.contains_key(&fresh_key));
        expire_segmented_requests(&mut receivers, now);
        assert_eq!(saved_request_payload_bytes(&receivers), Some(5));
        receivers.clear();
        assert_eq!(saved_request_payload_bytes(&receivers), Some(0));
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }
}

#[test]
fn request_payload_owner_discard_classification_preserves_payload_and_progress() {
    let now = Instant::now();
    let mut state = state(now, b"first");
    state
        .payload
        .save_new(1, Bytes::from_static(b"second"), Some(5))
        .unwrap();
    state.accepted_segments = 2;
    state.expected_seq = 2;
    state.last_acked_seq = 1;
    state.window_pos = 2;
    // Simulate activity refresh at the existing owner of that timer; discard
    // classification must not touch progress, accepted count or saved bytes.
    state.last_activity = now + Duration::from_secs(1);
    for seq in [1, 1, 1, 1, 5] {
        classify_non_next_segment(&mut state, 23, seq);
        assert_eq!(state.last_progress, now);
        assert_eq!(state.last_activity, now + Duration::from_secs(1));
        assert_eq!(state.accepted_segments, 2);
        assert_eq!(state.expected_seq, 2);
        assert_eq!(state.payload.saved_payload_bytes(), 11);
    }
    assert_eq!(
        state.payload.complete(2).unwrap().service_request.as_ref(),
        b"firstsecond"
    );
}
