use super::*;
use bacnet_types::MacAddr;
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::Bytes;

fn request(invoke_id: u8, body: impl Into<Bytes>) -> ConfirmedRequest {
    ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: Some(4),
        max_apdu_length: 480,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: body.into(),
    }
}
fn new(admission: ConfirmedRequestAdmission) -> PendingConfirmedRequest {
    match admission {
        ConfirmedRequestAdmission::New(pending) => pending,
        _ => panic!("expected new"),
    }
}
fn begin(
    tracker: &Arc<ConfirmedRequestTracker>,
    request: ConfirmedRequest,
) -> ConfirmedRequestAdmission {
    tracker.begin(b"peer", None, TransportProvenance::unverified(), request)
}
#[test]
fn pending_duplicate_release_allows_immediate_identical_invoke_reuse() {
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let request = request(1, Bytes::from_static(b"same"));
    let pending = new(begin(&tracker, request.clone()));
    assert!(matches!(
        begin(&tracker, request.clone()),
        ConfirmedRequestAdmission::Duplicate
    ));
    drop(pending);
    let reused = new(begin(&tracker, request.clone()));
    assert!(matches!(
        begin(&tracker, request),
        ConfirmedRequestAdmission::Duplicate
    ));
    drop(reused);
    assert!(tracker.state.lock().unwrap().entries.is_empty());
}
#[test]
fn full_request_discrimination_preserves_pending_independent_operations() {
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let first = request(7, Bytes::from_static(b"one"));
    let pending = new(begin(&tracker, first.clone()));
    let different_body = new(begin(&tracker, request(7, Bytes::from_static(b"two"))));
    let different_invoke = new(begin(&tracker, request(8, Bytes::from_static(b"one"))));
    let mut service = first.clone();
    service.service_choice = ConfirmedServiceChoice::DELETE_OBJECT;
    let different_service = new(begin(&tracker, service));
    assert!(matches!(
        begin(&tracker, first.clone()),
        ConfirmedRequestAdmission::Duplicate
    ));
    drop((different_body, different_invoke, different_service));
    assert_eq!(tracker.state.lock().unwrap().entries.len(), 1);
    drop(pending);
    drop(new(begin(&tracker, first)));
}
#[test]
fn canonical_routed_origin_ignores_router_while_pending() {
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let origin = NpduAddress {
        network: 5,
        mac_address: MacAddr::from_slice(b"origin"),
    };
    let req = request(2, Bytes::from_static(b"same"));
    let pending = new(tracker.begin(
        b"router-a",
        Some(&origin),
        TransportProvenance::unverified(),
        req.clone(),
    ));
    assert!(matches!(
        tracker.begin(
            b"router-b",
            Some(&origin),
            TransportProvenance::unverified(),
            req.clone()
        ),
        ConfirmedRequestAdmission::Duplicate
    ));
    let other = NpduAddress {
        network: 6,
        ..origin.clone()
    };
    drop(new(tracker.begin(
        b"router-b",
        Some(&other),
        TransportProvenance::unverified(),
        req.clone(),
    )));
    drop(new(begin(&tracker, req.clone())));
    drop(pending);
    drop(new(tracker.begin(
        b"router-b",
        Some(&origin),
        TransportProvenance::unverified(),
        req,
    )));
}
#[test]
fn all_pending_capacity_keeps_detection_and_untracked_fallback() {
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let requests: Vec<_> = (0..MAX_ENTRIES)
        .map(|i| request(3, Bytes::from(i.to_be_bytes().to_vec())))
        .collect();
    let mut owners: Vec<_> = requests
        .iter()
        .cloned()
        .map(|r| new(begin(&tracker, r)))
        .collect();
    assert!(matches!(
        begin(&tracker, requests[0].clone()),
        ConfirmedRequestAdmission::Duplicate
    ));
    let overflow = request(4, Bytes::from_static(b"overflow"));
    let a = new(begin(&tracker, overflow.clone()));
    let b = new(begin(&tracker, overflow.clone()));
    assert!(a.id.is_none() && b.id.is_none());
    assert_eq!(tracker.state.lock().unwrap().entries.len(), 256);
    drop(owners.pop());
    let admitted = new(begin(&tracker, overflow));
    assert!(admitted.id.is_some());
    drop((a, b, owners, admitted));
    assert!(tracker.state.lock().unwrap().entries.is_empty());
}
#[test]
fn oversized_requests_are_untracked_without_consuming_capacity() {
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let req = request(1, vec![0; MAX_TRACKED_SERVICE_REQUEST_BYTES + 1]);
    let a = new(begin(&tracker, req.clone()));
    let b = new(begin(&tracker, req));
    assert!(a.id.is_none() && b.id.is_none());
    let boundary = new(begin(
        &tracker,
        request(1, vec![0; MAX_TRACKED_SERVICE_REQUEST_BYTES]),
    ));
    assert!(boundary.id.is_some());
}
#[test]
fn cancellation_panic_and_counter_exhaustion_leave_no_alias() {
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let req = request(1, Bytes::new());
    let panicking = tracker.clone();
    let input = req.clone();
    assert!(
        std::panic::catch_unwind(move || {
            let _owner = new(begin(&panicking, input));
            panic!("injected");
        })
        .is_err()
    );
    let owner = new(begin(&tracker, req.clone()));
    tracker.state.lock().unwrap().next_id = u64::MAX;
    let fallback = new(begin(&tracker, request(2, Bytes::new())));
    assert!(fallback.id.is_none());
    drop(fallback);
    assert!(matches!(
        begin(&tracker, req),
        ConfirmedRequestAdmission::Duplicate
    ));
    drop(owner);
    assert!(tracker.state.lock().unwrap().entries.is_empty());
}
#[test]
fn concurrent_exact_admission_has_one_owner_until_release() {
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let tasks: Vec<_> = (0..8)
        .map(|_| {
            let tracker = tracker.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                begin(&tracker, request(1, Bytes::new()))
            })
        })
        .collect();
    let results: Vec<_> = tasks.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, ConfirmedRequestAdmission::New(_)))
            .count(),
        1
    );
    drop(results);
    drop(new(begin(&tracker, request(1, Bytes::new()))));
}
