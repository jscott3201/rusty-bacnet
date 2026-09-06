use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bacnet_encoding::apdu::ConfirmedRequest;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::MacAddr;

/// Local retention and resource policy for exact confirmed-request detection.
///
/// Clause 5.3.5.3 requires a server to discard a duplicate when it can detect
/// one, but does not mandate these bounds or exact-request discrimination. An
/// identical legal Invoke ID reuse inside this window is necessarily
/// indistinguishable and may therefore be discarded. No response is retained
/// or replayed; expiry or server restart clears this guard, though a service-
/// specific durable idempotency policy may still apply.
const COMPLETED_RETENTION: Duration = Duration::from_secs(60);
const MAX_ENTRIES: usize = 256;
const MAX_TRACKED_SERVICE_REQUEST_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
enum CanonicalRequester {
    Direct(MacAddr),
    Routed(NpduAddress),
}

struct Entry {
    id: u64,
    requester: CanonicalRequester,
    invoke_id: u8,
    request: ConfirmedRequest,
    completed_at: Option<Instant>,
}

#[derive(Default)]
struct TrackerState {
    next_id: u64,
    entries: VecDeque<Entry>,
}

/// Server-lifetime, bounded, exact confirmed-request duplicate tracker.
#[derive(Default)]
pub(super) struct ConfirmedRequestTracker {
    state: Mutex<TrackerState>,
}

pub(super) enum ConfirmedRequestAdmission {
    Duplicate,
    New(PendingConfirmedRequest),
}

/// RAII admission for one request that is pending handler completion.
///
/// Normal handler completion calls [`Self::complete`]. Cancellation or panic
/// drops an incomplete admission and removes its pending entry so a retry can
/// be serviced.
pub(super) struct PendingConfirmedRequest {
    tracker: Arc<ConfirmedRequestTracker>,
    id: Option<u64>,
    completed: bool,
}

impl ConfirmedRequestTracker {
    pub(super) fn begin(
        self: &Arc<Self>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        request: ConfirmedRequest,
    ) -> ConfirmedRequestAdmission {
        self.begin_at(source_mac, source_network, request, Instant::now())
    }

    fn begin_at(
        self: &Arc<Self>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        request: ConfirmedRequest,
        now: Instant,
    ) -> ConfirmedRequestAdmission {
        if request.service_request.len() > MAX_TRACKED_SERVICE_REQUEST_BYTES {
            return ConfirmedRequestAdmission::New(PendingConfirmedRequest::untracked(self));
        }

        let requester = canonical_requester(source_mac, source_network);
        let invoke_id = request.invoke_id;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.entries.retain(|entry| match entry.completed_at {
            None => true,
            Some(completed_at) => completed_at + COMPLETED_RETENTION > now,
        });
        if state.entries.iter().any(|entry| {
            entry.requester == requester && entry.invoke_id == invoke_id && entry.request == request
        }) {
            return ConfirmedRequestAdmission::Duplicate;
        }

        if state.entries.len() >= MAX_ENTRIES {
            let oldest_completed = state
                .entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    entry.completed_at.map(|completed_at| (index, completed_at))
                })
                .min_by_key(|(_, completed_at)| *completed_at)
                .map(|(index, _)| index);
            if let Some(index) = oldest_completed {
                state.entries.remove(index);
            } else {
                // Every bounded slot is still executing. Detection is not safe
                // here, so Clause 5.3.5.3 permits normal untracked service.
                return ConfirmedRequestAdmission::New(PendingConfirmedRequest::untracked(self));
            }
        }

        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        state.entries.push_back(Entry {
            id,
            requester,
            invoke_id,
            request,
            completed_at: None,
        });
        ConfirmedRequestAdmission::New(PendingConfirmedRequest {
            tracker: Arc::clone(self),
            id: Some(id),
            completed: false,
        })
    }
}

impl PendingConfirmedRequest {
    fn untracked(tracker: &Arc<ConfirmedRequestTracker>) -> Self {
        Self {
            tracker: Arc::clone(tracker),
            id: None,
            completed: false,
        }
    }

    pub(super) fn complete(self) {
        self.complete_at(Instant::now());
    }

    fn complete_at(mut self, now: Instant) {
        if let Some(id) = self.id {
            let mut state = self
                .tracker
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(entry) = state.entries.iter_mut().find(|entry| entry.id == id) {
                entry.completed_at = Some(now);
            }
        }
        self.completed = true;
    }
}

impl Drop for PendingConfirmedRequest {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        if let Some(id) = self.id {
            let mut state = self
                .tracker
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.entries.retain(|entry| entry.id != id);
        }
    }
}

fn canonical_requester(
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
) -> CanonicalRequester {
    source_network
        .filter(|source| (1..=0xfffe).contains(&source.network) && !source.mac_address.is_empty())
        .cloned()
        .map(CanonicalRequester::Routed)
        .unwrap_or_else(|| CanonicalRequester::Direct(MacAddr::from_slice(source_mac)))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;

    use bacnet_types::enums::ConfirmedServiceChoice;
    use bytes::Bytes;

    use super::*;

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

    fn routed(network: u16, mac: &[u8]) -> NpduAddress {
        NpduAddress {
            network,
            mac_address: MacAddr::from_slice(mac),
        }
    }

    fn expect_new(admission: ConfirmedRequestAdmission) -> PendingConfirmedRequest {
        match admission {
            ConfirmedRequestAdmission::New(pending) => pending,
            ConfirmedRequestAdmission::Duplicate => panic!("first request was a duplicate"),
        }
    }

    #[test]
    fn exact_request_moves_from_pending_to_completed_until_window_boundary() {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let started_at = Instant::now();
        let req = request(1, Bytes::from_static(b"request"));
        let pending = expect_new(tracker.begin_at(b"peer", None, req.clone(), started_at));
        assert!(matches!(
            tracker.begin_at(b"peer", None, req.clone(), started_at),
            ConfirmedRequestAdmission::Duplicate
        ));

        let completed_at = started_at + COMPLETED_RETENTION + Duration::from_secs(30);
        assert!(matches!(
            tracker.begin_at(b"peer", None, req.clone(), completed_at),
            ConfirmedRequestAdmission::Duplicate
        ));
        pending.complete_at(completed_at);
        assert!(matches!(
            tracker.begin_at(
                b"peer",
                None,
                req.clone(),
                completed_at + COMPLETED_RETENTION - Duration::from_millis(1)
            ),
            ConfirmedRequestAdmission::Duplicate
        ));
        assert!(matches!(
            tracker.begin_at(b"peer", None, req, completed_at + COMPLETED_RETENTION),
            ConfirmedRequestAdmission::New(_)
        ));
    }

    #[test]
    fn complete_request_discrimination_allows_changed_invoke_reuse() {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let now = Instant::now();
        let first = request(7, Bytes::from_static(b"one"));
        expect_new(tracker.begin_at(b"peer", None, first.clone(), now)).complete_at(now);

        assert!(matches!(
            tracker.begin_at(b"peer", None, first, now),
            ConfirmedRequestAdmission::Duplicate
        ));
        let changed_body = expect_new(tracker.begin_at(
            b"peer",
            None,
            request(7, Bytes::from_static(b"two")),
            now,
        ));
        drop(changed_body);
        let mut changed_service = request(7, Bytes::from_static(b"one"));
        changed_service.service_choice = ConfirmedServiceChoice::DELETE_OBJECT;
        assert!(matches!(
            tracker.begin_at(b"peer", None, changed_service, now),
            ConfirmedRequestAdmission::New(_)
        ));
    }

    #[test]
    fn canonical_routed_origin_ignores_router_and_peers_remain_independent() {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let now = Instant::now();
        let req = request(2, Bytes::from_static(b"same"));
        let origin = routed(5, b"origin");
        expect_new(tracker.begin_at(b"router-a", Some(&origin), req.clone(), now)).complete_at(now);
        assert!(matches!(
            tracker.begin_at(b"router-b", Some(&origin), req.clone(), now),
            ConfirmedRequestAdmission::Duplicate
        ));
        assert!(matches!(
            tracker.begin_at(b"router-b", Some(&routed(6, b"origin")), req.clone(), now),
            ConfirmedRequestAdmission::New(_)
        ));
        assert!(matches!(
            tracker.begin_at(b"direct-a", None, req.clone(), now),
            ConfirmedRequestAdmission::New(_)
        ));
        assert!(matches!(
            tracker.begin_at(b"direct-b", None, req.clone(), now),
            ConfirmedRequestAdmission::New(_)
        ));

        let invalid = routed(0, b"claimed-origin");
        let invalid_pending =
            expect_new(tracker.begin_at(b"router-c", Some(&invalid), req.clone(), now));
        invalid_pending.complete_at(now);
        assert!(matches!(
            tracker.begin_at(b"router-d", Some(&invalid), req, now),
            ConfirmedRequestAdmission::New(_)
        ));
    }

    #[test]
    fn completed_capacity_evicts_oldest_completion() {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let now = Instant::now();
        for index in 0..MAX_ENTRIES {
            let req = request(3, Bytes::from(vec![index as u8, (index >> 8) as u8]));
            expect_new(tracker.begin_at(b"peer", None, req, now))
                .complete_at(now + Duration::from_millis(index as u64));
        }

        expect_new(tracker.begin_at(
            b"peer",
            None,
            request(3, Bytes::from_static(b"newest")),
            now + Duration::from_millis(MAX_ENTRIES as u64),
        ))
        .complete_at(now + Duration::from_millis(MAX_ENTRIES as u64));
        assert_eq!(tracker.state.lock().unwrap().entries.len(), MAX_ENTRIES);
        assert!(matches!(
            tracker.begin_at(b"peer", None, request(3, Bytes::from_static(&[1, 0])), now),
            ConfirmedRequestAdmission::Duplicate
        ));
        assert!(matches!(
            tracker.begin_at(b"peer", None, request(3, Bytes::from_static(&[0, 0])), now),
            ConfirmedRequestAdmission::New(_)
        ));
    }

    #[test]
    fn all_pending_capacity_and_oversize_requests_fall_back_untracked() {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let now = Instant::now();
        let mut pending = Vec::new();
        for index in 0..MAX_ENTRIES {
            pending.push(expect_new(tracker.begin_at(
                b"peer",
                None,
                request(4, Bytes::from(vec![index as u8, (index >> 8) as u8])),
                now,
            )));
        }
        assert!(matches!(
            tracker.begin_at(b"peer", None, request(4, Bytes::from_static(&[0, 0])), now),
            ConfirmedRequestAdmission::Duplicate
        ));
        let fallback = expect_new(tracker.begin_at(
            b"peer",
            None,
            request(4, Bytes::from_static(b"fallback")),
            now,
        ));
        assert!(fallback.id.is_none());
        assert_eq!(tracker.state.lock().unwrap().entries.len(), MAX_ENTRIES);
        drop(fallback);
        assert!(matches!(
            tracker.begin_at(
                b"peer",
                None,
                request(4, Bytes::from_static(b"fallback")),
                now
            ),
            ConfirmedRequestAdmission::New(_)
        ));
        drop(pending);

        let oversized = expect_new(tracker.begin_at(
            b"peer",
            None,
            request(
                5,
                Bytes::from(vec![0; MAX_TRACKED_SERVICE_REQUEST_BYTES + 1]),
            ),
            now,
        ));
        assert!(oversized.id.is_none());
    }

    #[test]
    fn raii_drop_reclaims_cancelled_pending_and_restart_allows_service_again() {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let now = Instant::now();
        let req = request(6, Bytes::from_static(b"cancelled"));
        let pending = expect_new(tracker.begin_at(b"peer", None, req.clone(), now));
        drop(pending);
        assert!(matches!(
            tracker.begin_at(b"peer", None, req.clone(), now),
            ConfirmedRequestAdmission::New(_)
        ));

        let restarted = Arc::new(ConfirmedRequestTracker::default());
        assert!(matches!(
            restarted.begin_at(b"peer", None, req, now),
            ConfirmedRequestAdmission::New(_)
        ));
    }

    #[test]
    fn concurrent_exact_admission_is_atomic() {
        const WORKERS: usize = 16;
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let start = Arc::new(Barrier::new(WORKERS + 1));
        let admitted = Arc::new(AtomicUsize::new(0));
        let finish = Arc::new(Barrier::new(WORKERS + 1));
        let mut workers = Vec::new();

        for _ in 0..WORKERS {
            let tracker = Arc::clone(&tracker);
            let start = Arc::clone(&start);
            let admitted = Arc::clone(&admitted);
            let finish = Arc::clone(&finish);
            workers.push(std::thread::spawn(move || {
                start.wait();
                let pending = match tracker.begin(
                    b"peer",
                    None,
                    request(8, Bytes::from_static(b"concurrent")),
                ) {
                    ConfirmedRequestAdmission::Duplicate => None,
                    ConfirmedRequestAdmission::New(pending) => {
                        admitted.fetch_add(1, Ordering::AcqRel);
                        Some(pending)
                    }
                };
                finish.wait();
                drop(pending);
            }));
        }

        start.wait();
        finish.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(admitted.load(Ordering::Acquire), 1);
    }
}
