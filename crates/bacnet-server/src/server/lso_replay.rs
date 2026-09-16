//! Server-side confirmed-response replay for LifeSafetyOperation only.
//!
//! ASHRAE 135-2020 §13.13 (PDF pp. 703-704, printed 701-702) defines the
//! LifeSafetyOperation service. §5.3.5.3(a) (PDF p. 36, printed 34) requires a
//! server to discard a detectable duplicate, but it does NOT mandate retaining
//! or replaying the original response. This module is therefore a LOCAL,
//! service-specific idempotency extension confined to
//! `ConfirmedServiceChoice::LIFE_SAFETY_OPERATION` — not a Standard mandate —
//! and it deliberately diverges from the generic
//! [`super::confirmed_request_tracker`] policy (detection WITHOUT replay,
//! silent discard) for this one service.
//!
//! Wire-visible contract: a retransmitted already-executed confirmed LSO
//! within the window receives a byte-identical SimpleACK/Error resend with a
//! single execution. Pending (in-flight, no response yet) concurrent exact
//! duplicates preserve the generic DISCARD (no replay available).
//!
//! Safety invariants (life-safety-adjacent, all held):
//! 1. Any doubt (oversize, full-pending, empty/restarted store) executes
//!    normally; first execution is never suppressed.
//! 2. Full key match required: canonical requester + invoke ID + full
//!    ConfirmedRequest equality (service choice LSO + exact service-request
//!    bytes). The Requesting Source is fingerprint bytes inside the request,
//!    NEVER an authenticated identity.
//! 3. Post-TTL expiry executes fresh (it does not mask a new operation).
//! 4. Replay path has zero side effects: no mutation, no authorizer
//!    re-invocation, no COV/event/reset re-fire — byte resend only.
//! 5. Fail-closed authorization is preserved: the authorizer still runs
//!    outside the database lock on first execution.
//! 6. No physical-idempotency claim: reset executors own application
//!    idempotency; replay only shrinks the retransmission window.
//!
//! Resource policy is SEPARATE from the generic tracker (not shared):
//! 60 s completed-only TTL measured from response time, at most 256 entries,
//! service requests larger than 64 KiB are served untracked (execute, never
//! store), oldest-completed-first eviction, all-pending-full serves untracked.
//! In-memory only; a restart clears the store.
//!
//! Locking: `std::sync::Mutex`, lock → clone → unlock → send, never held
//! across `.await` (same convention as the generic tracker).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;

use super::request_peer::{canonical_requester, CanonicalRequester};
use bacnet_encoding::apdu::ConfirmedRequest;
use bacnet_encoding::npdu::NpduAddress;

/// Completed LSO responses are retained for this long after the response time.
const COMPLETED_RETENTION: Duration = Duration::from_secs(60);
/// Independent LSO-only entry budget (not shared with the generic tracker).
const MAX_ENTRIES: usize = 256;
/// Service requests larger than this are executed untracked (never stored,
/// never replayed) so an oversized retransmission can never suppress work.
const MAX_TRACKED_SERVICE_REQUEST_BYTES: usize = 64 * 1024;

struct Entry {
    id: u64,
    requester: CanonicalRequester,
    invoke_id: u8,
    request: ConfirmedRequest,
    completed_at: Option<Instant>,
    response: Option<Bytes>,
}

#[derive(Default)]
struct LsoState {
    next_id: u64,
    entries: VecDeque<Entry>,
}

/// Server-lifetime, bounded, LSO-only confirmed-response replay store.
#[derive(Default)]
pub(super) struct LsoReplayCache {
    state: Mutex<LsoState>,
}

pub(super) enum LsoAdmission {
    /// Already executed within the window: resend these exact APDU bytes.
    Replay(Bytes),
    /// In-flight with no response yet: preserve the generic DISCARD.
    DuplicatePending,
    /// Execute (tracked) or execute untracked (`id` is `None`); completing an
    /// untracked guard is a no-op.
    New(PendingLsoReplay),
}

/// RAII admission for one LSO request.
///
/// A handler that produces a response calls [`Self::complete_with_response`].
/// Cancellation, panic, DCC drops, or overload drops an incomplete admission
/// and removes its pending entry so a retry executes normally.
pub(super) struct PendingLsoReplay {
    cache: Arc<LsoReplayCache>,
    id: Option<u64>,
    completed: bool,
}

impl LsoReplayCache {
    pub(super) fn begin(
        self: &Arc<Self>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        request: ConfirmedRequest,
    ) -> LsoAdmission {
        self.begin_at(source_mac, source_network, request, Instant::now())
    }

    fn begin_at(
        self: &Arc<Self>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        request: ConfirmedRequest,
        now: Instant,
    ) -> LsoAdmission {
        if request.service_request.len() > MAX_TRACKED_SERVICE_REQUEST_BYTES {
            return LsoAdmission::New(PendingLsoReplay::untracked(self));
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
        if let Some(hit) = state.entries.iter().find(|entry| {
            entry.requester == requester && entry.invoke_id == invoke_id && entry.request == request
        }) {
            match (hit.completed_at, hit.response.clone()) {
                (Some(_), Some(response)) => return LsoAdmission::Replay(response),
                // Completed without bytes must never suppress work (invariant
                // 1); this state is unreachable because completion always
                // stores bytes, but a defensive fresh execution is safer than
                // a silent drop. Fall through to insert a new pending entry?
                // No: inserting would create a duplicate key. Instead treat as
                // pending-discard would also suppress. The only safe choice
                // given the unreachable state is to serve untracked.
                (Some(_), None) => {
                    return LsoAdmission::New(PendingLsoReplay::untracked(self));
                }
                (None, _) => return LsoAdmission::DuplicatePending,
            }
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
                // Every bounded slot is still executing. Execute untracked so
                // the first execution is never suppressed (invariant 1).
                return LsoAdmission::New(PendingLsoReplay::untracked(self));
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
            response: None,
        });
        LsoAdmission::New(PendingLsoReplay {
            cache: Arc::clone(self),
            id: Some(id),
            completed: false,
        })
    }
}

impl PendingLsoReplay {
    fn untracked(cache: &Arc<LsoReplayCache>) -> Self {
        Self {
            cache: Arc::clone(cache),
            id: None,
            completed: false,
        }
    }

    /// True when this admission is tracked and completion will store bytes.
    #[allow(dead_code)]
    pub(super) fn is_tracked(&self) -> bool {
        self.id.is_some()
    }

    pub(super) fn complete_with_response(self, response: Bytes) {
        self.complete_with_response_at(response, Instant::now());
    }

    fn complete_with_response_at(mut self, response: Bytes, now: Instant) {
        if let Some(id) = self.id {
            let mut state = self
                .cache
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(entry) = state.entries.iter_mut().find(|entry| entry.id == id) {
                entry.completed_at = Some(now);
                entry.response = Some(response);
            }
        }
        self.completed = true;
    }
}

impl Drop for PendingLsoReplay {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        if let Some(id) = self.id {
            let mut state = self
                .cache
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.entries.retain(|entry| entry.id != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;

    use bacnet_types::enums::ConfirmedServiceChoice;
    use bacnet_types::MacAddr;
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
            service_choice: ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
            service_request: body.into(),
        }
    }

    fn routed(network: u16, mac: &[u8]) -> NpduAddress {
        NpduAddress {
            network,
            mac_address: MacAddr::from_slice(mac),
        }
    }

    fn expect_new(admission: LsoAdmission) -> PendingLsoReplay {
        match admission {
            LsoAdmission::New(pending) => pending,
            LsoAdmission::Replay(_) => panic!("unexpected replay"),
            LsoAdmission::DuplicatePending => panic!("unexpected pending duplicate"),
        }
    }

    fn expect_replay(admission: LsoAdmission) -> Bytes {
        match admission {
            LsoAdmission::Replay(bytes) => bytes,
            LsoAdmission::New(_) => panic!("expected replay, got new"),
            LsoAdmission::DuplicatePending => panic!("expected replay, got pending"),
        }
    }

    #[test]
    fn completed_response_replays_until_window_boundary_then_reexecutes() {
        let cache = Arc::new(LsoReplayCache::default());
        let started_at = Instant::now();
        let req = request(1, Bytes::from_static(b"lso"));
        let pending = expect_new(cache.begin_at(b"peer", None, req.clone(), started_at));
        assert!(matches!(
            cache.begin_at(b"peer", None, req.clone(), started_at),
            LsoAdmission::DuplicatePending
        ));

        let completed_at = started_at + Duration::from_secs(30);
        pending.complete_with_response_at(Bytes::from_static(b"ack"), completed_at);
        assert_eq!(
            expect_replay(cache.begin_at(b"peer", None, req.clone(), completed_at)),
            Bytes::from_static(b"ack")
        );
        assert_eq!(
            expect_replay(cache.begin_at(
                b"peer",
                None,
                req.clone(),
                completed_at + COMPLETED_RETENTION - Duration::from_millis(1)
            )),
            Bytes::from_static(b"ack")
        );
        assert!(matches!(
            cache.begin_at(b"peer", None, req, completed_at + COMPLETED_RETENTION),
            LsoAdmission::New(_)
        ));
    }

    #[test]
    fn error_bytes_replay_identically() {
        let cache = Arc::new(LsoReplayCache::default());
        let now = Instant::now();
        let req = request(9, Bytes::from_static(b"denied"));
        expect_new(cache.begin_at(b"peer", None, req.clone(), now))
            .complete_with_response_at(Bytes::from_static(b"error"), now);
        assert_eq!(
            expect_replay(cache.begin_at(b"peer", None, req, now)),
            Bytes::from_static(b"error")
        );
    }

    #[test]
    fn full_key_match_required_for_replay() {
        let cache = Arc::new(LsoReplayCache::default());
        let now = Instant::now();
        let first = request(7, Bytes::from_static(b"one"));
        expect_new(cache.begin_at(b"peer", None, first.clone(), now))
            .complete_with_response_at(Bytes::from_static(b"r1"), now);

        assert!(matches!(
            cache.begin_at(b"peer", None, first, now),
            LsoAdmission::Replay(_)
        ));
        // Changed invoke ID re-executes.
        let changed_invoke =
            expect_new(cache.begin_at(b"peer", None, request(8, Bytes::from_static(b"one")), now));
        drop(changed_invoke);
        // Changed bytes re-execute.
        let changed_body =
            expect_new(cache.begin_at(b"peer", None, request(7, Bytes::from_static(b"two")), now));
        drop(changed_body);
        // Changed service choice re-executes.
        let mut changed_service = request(7, Bytes::from_static(b"one"));
        changed_service.service_choice = ConfirmedServiceChoice::ACKNOWLEDGE_ALARM;
        assert!(matches!(
            cache.begin_at(b"peer", None, changed_service, now),
            LsoAdmission::New(_)
        ));
    }

    #[test]
    fn canonical_routed_origin_replays_while_peers_stay_independent() {
        let cache = Arc::new(LsoReplayCache::default());
        let now = Instant::now();
        let req = request(2, Bytes::from_static(b"same"));
        let origin = routed(5, b"origin");
        expect_new(cache.begin_at(b"router-a", Some(&origin), req.clone(), now))
            .complete_with_response_at(Bytes::from_static(b"ack"), now);
        assert!(matches!(
            cache.begin_at(b"router-b", Some(&origin), req.clone(), now),
            LsoAdmission::Replay(_)
        ));
        assert!(matches!(
            cache.begin_at(b"router-b", Some(&routed(6, b"origin")), req.clone(), now),
            LsoAdmission::New(_)
        ));
        assert!(matches!(
            cache.begin_at(b"direct-a", None, req.clone(), now),
            LsoAdmission::New(_)
        ));
        let invalid = routed(0, b"claimed-origin");
        let invalid_pending =
            expect_new(cache.begin_at(b"router-c", Some(&invalid), req.clone(), now));
        assert!(invalid_pending.is_tracked());
        invalid_pending.complete_with_response_at(Bytes::from_static(b"ack"), now);
        // Network 0 is not a valid routed origin, so it collapses to direct
        // peers keyed by immediate MAC and stays independent.
        assert!(matches!(
            cache.begin_at(b"router-d", Some(&invalid), req, now),
            LsoAdmission::New(_)
        ));
    }

    #[test]
    fn completed_capacity_evicts_oldest_completion() {
        let cache = Arc::new(LsoReplayCache::default());
        let now = Instant::now();
        for index in 0..MAX_ENTRIES {
            let req = request(3, Bytes::from(vec![index as u8, (index >> 8) as u8]));
            expect_new(cache.begin_at(b"peer", None, req, now)).complete_with_response_at(
                Bytes::from_static(b"r"),
                now + Duration::from_millis(index as u64),
            );
        }

        expect_new(cache.begin_at(
            b"peer",
            None,
            request(3, Bytes::from_static(b"newest")),
            now + Duration::from_millis(MAX_ENTRIES as u64),
        ))
        .complete_with_response_at(
            Bytes::from_static(b"new"),
            now + Duration::from_millis(MAX_ENTRIES as u64),
        );
        assert_eq!(cache.state.lock().unwrap().entries.len(), MAX_ENTRIES);
        // Newest still replays; oldest was evicted so it re-executes.
        assert!(matches!(
            cache.begin_at(
                b"peer",
                None,
                request(3, Bytes::from_static(b"newest")),
                now
            ),
            LsoAdmission::Replay(_)
        ));
        assert!(matches!(
            cache.begin_at(b"peer", None, request(3, Bytes::from_static(&[0, 0])), now),
            LsoAdmission::New(_)
        ));
    }

    #[test]
    fn all_pending_capacity_and_oversize_serve_untracked_without_suppression() {
        let cache = Arc::new(LsoReplayCache::default());
        let now = Instant::now();
        let mut pending = Vec::new();
        for index in 0..MAX_ENTRIES {
            pending.push(expect_new(cache.begin_at(
                b"peer",
                None,
                request(4, Bytes::from(vec![index as u8, (index >> 8) as u8])),
                now,
            )));
        }
        // Exact pending duplicate still discards.
        assert!(matches!(
            cache.begin_at(b"peer", None, request(4, Bytes::from_static(&[0, 0])), now),
            LsoAdmission::DuplicatePending
        ));
        let fallback = expect_new(cache.begin_at(
            b"peer",
            None,
            request(4, Bytes::from_static(b"fallback")),
            now,
        ));
        assert!(!fallback.is_tracked());
        assert_eq!(cache.state.lock().unwrap().entries.len(), MAX_ENTRIES);
        drop(fallback);
        // Untracked fallback never suppresses a later identical request.
        assert!(matches!(
            cache.begin_at(
                b"peer",
                None,
                request(4, Bytes::from_static(b"fallback")),
                now
            ),
            LsoAdmission::New(_)
        ));
        drop(pending);

        let oversized = expect_new(cache.begin_at(
            b"peer",
            None,
            request(
                5,
                Bytes::from(vec![0; MAX_TRACKED_SERVICE_REQUEST_BYTES + 1]),
            ),
            now,
        ));
        assert!(!oversized.is_tracked());
        drop(oversized);
        // Oversize never suppresses: the same oversized request executes again.
        let again = expect_new(cache.begin_at(
            b"peer",
            None,
            request(
                5,
                Bytes::from(vec![0; MAX_TRACKED_SERVICE_REQUEST_BYTES + 1]),
            ),
            now,
        ));
        assert!(!again.is_tracked());
    }

    #[test]
    fn raii_drop_reclaims_cancelled_pending_and_restart_clears() {
        let cache = Arc::new(LsoReplayCache::default());
        let now = Instant::now();
        let req = request(6, Bytes::from_static(b"cancelled"));
        let pending = expect_new(cache.begin_at(b"peer", None, req.clone(), now));
        drop(pending);
        assert!(matches!(
            cache.begin_at(b"peer", None, req.clone(), now),
            LsoAdmission::New(_)
        ));

        let restarted = Arc::new(LsoReplayCache::default());
        assert!(matches!(
            restarted.begin_at(b"peer", None, req, now),
            LsoAdmission::New(_)
        ));
    }

    #[test]
    fn concurrent_exact_admission_is_atomic_single_execution() {
        const WORKERS: usize = 16;
        let cache = Arc::new(LsoReplayCache::default());
        let start = Arc::new(Barrier::new(WORKERS + 1));
        let admitted = Arc::new(AtomicUsize::new(0));
        let finish = Arc::new(Barrier::new(WORKERS + 1));
        let mut workers = Vec::new();

        for _ in 0..WORKERS {
            let cache = Arc::clone(&cache);
            let start = Arc::clone(&start);
            let admitted = Arc::clone(&admitted);
            let finish = Arc::clone(&finish);
            workers.push(std::thread::spawn(move || {
                start.wait();
                let pending =
                    match cache.begin(b"peer", None, request(8, Bytes::from_static(b"concurrent")))
                    {
                        LsoAdmission::DuplicatePending => None,
                        LsoAdmission::Replay(_) => panic!("no response yet"),
                        LsoAdmission::New(pending) => {
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
