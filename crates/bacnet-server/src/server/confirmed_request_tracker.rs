use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::request_peer::{CanonicalRequester, canonical_requester};
use bacnet_encoding::apdu::ConfirmedRequest;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_transport::port::{DirectScIdentity, TransportProvenance};
use bacnet_types::sync::MutexExt;

// Local detection bounds, not transaction lifetimes. Above the entry bound,
// Clause 5.3.5.3 permits normal service when exact duplicate detection is unavailable.
const MAX_ENTRIES: usize = 256;
// Above this an entry keeps the request's digest, not its data (#2751).
const MAX_TRACKED_SERVICE_REQUEST_BYTES: usize = 64 * 1024;

struct Entry {
    id: u64,
    requester: CanonicalRequester,
    direct_identity: Option<DirectScIdentity>,
    invoke_id: u8,
    request: ConfirmedRequest,
}

impl Entry {
    /// Whether this entry is under that requester, direct identity and
    /// invoke ID: the key duplicates are judged under, and an answer in
    /// progress is ended under (#2770).
    #[inline]
    fn is_under(
        &self,
        requester: &CanonicalRequester,
        direct_identity: &Option<DirectScIdentity>,
        invoke_id: u8,
    ) -> bool {
        self.requester == *requester
            && self.direct_identity == *direct_identity
            && self.invoke_id == invoke_id
    }
}

#[derive(Default)]
struct TrackerState {
    next_id: u64,
    entries: Vec<Entry>,
    /// What a repeated segment of a reassembled request still executing is
    /// answered with (#2751). Only the segmented branch reads or writes it;
    /// `begin` and the drop of a pending request never touch it.
    segmented: Vec<segment_repeats::Record>,
    /// The answers sent in segments, oldest first (#2770). `begin` reads
    /// them only once an entry under its request's key is pending, and the
    /// drop of a pending request never touches them.
    answers: Vec<answers::Answer>,
}

/// Exact duplicate detection only while the ordinary server transaction is live.
/// No completed response or TTL is retained. Accepted direct SC keys also carry
/// the original verified leaf/incarnation; non-direct canonicalization is unchanged.
/// LSO's completed replay is an independent service policy and budget.
#[derive(Default)]
pub(super) struct ConfirmedRequestTracker {
    state: Mutex<TrackerState>,
    pub(super) lso: Arc<super::lso_replay::LsoReplayCache>,
    /// See [`super::BACnetServer::group_source_request_drops`].
    group_source_drops: AtomicU64,
    /// Keys for the digest of an oversized request: this tracker's own, so
    /// a peer can't craft two requests with one digest ahead of time
    /// (#2751). Last, after every field the plain path reads.
    digest_keys: std::collections::hash_map::RandomState,
}

pub(super) enum ConfirmedRequestAdmission {
    Duplicate,
    New(PendingConfirmedRequest),
    /// Dropped, not executed (#2770): another request than the one
    /// answered in segments under its requester and invoke ID, midway
    /// through that answer, which it ends (the answer's task sends the
    /// Abort); or any request under them while an Abort there is still to
    /// be handed to the transport. The answer's entry lasts until then.
    AbortsAnswer,
}

/// Non-cloneable ownership of one pending transaction. Move into its response
/// owner: unsegmented local encoded issuance/handoff, or the segmented child
/// through final ACK/terminal outcome. Every cancellation/drop removes exactly
/// this entry; request-task and segmented-send permits have separate lifetimes.
pub(super) struct PendingConfirmedRequest {
    tracker: Arc<ConfirmedRequestTracker>,
    id: Option<u64>,
}

impl ConfirmedRequestTracker {
    pub(super) fn begin(
        self: &Arc<Self>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        provenance: TransportProvenance,
        mut request: ConfirmedRequest,
    ) -> ConfirmedRequestAdmission {
        if request.service_request.len() > MAX_TRACKED_SERVICE_REQUEST_BYTES {
            digest(&self.digest_keys, &mut request);
        }
        let requester = canonical_requester(source_mac, source_network);
        let direct_identity = provenance.direct_sc_identity();
        let invoke_id = request.invoke_id;
        let mut state = self.state.locked();
        // Nothing more for a request with no entry under its key pending.
        if let Some(first) = state
            .entries
            .iter()
            .position(|entry| entry.is_under(&requester, &direct_identity, invoke_id))
            && let Some(admission) =
                state.admit_same_key(first, &requester, &direct_identity, invoke_id, &request)
        {
            return admission;
        }
        if state.entries.len() >= MAX_ENTRIES {
            return ConfirmedRequestAdmission::New(PendingConfirmedRequest::untracked(self));
        }
        let id = state.next_id;
        let Some(next_id) = id.checked_add(1) else {
            // Never alias a surviving guard after counter exhaustion.
            return ConfirmedRequestAdmission::New(PendingConfirmedRequest::untracked(self));
        };
        state.next_id = next_id;
        state.entries.push(Entry {
            id,
            requester,
            direct_identity,
            invoke_id,
            request,
        });
        ConfirmedRequestAdmission::New(PendingConfirmedRequest {
            tracker: Arc::clone(self),
            id: Some(id),
        })
    }
}
/// Keep an oversized request by its header and a digest of its service data
/// (its length and a keyed SipHash of it), not the data: only a reassembled
/// request can be this long, so a request that came whole never gets here.
/// Two such requests with equal headers and digests count as the same
/// request. The `segmented` flag marks the digest form: no request `begin`
/// is given has it set, so the form never equals one kept whole (#2751).
#[cold]
#[inline(never)]
fn digest(keys: &std::collections::hash_map::RandomState, request: &mut ConfirmedRequest) {
    use std::hash::BuildHasher;
    let hash = keys.hash_one(&request.service_request[..]);
    let mut kept = Vec::with_capacity(16);
    kept.extend_from_slice(&(request.service_request.len() as u64).to_be_bytes());
    kept.extend_from_slice(&hash.to_be_bytes());
    request.service_request = bytes::Bytes::from(kept);
    request.segmented = true;
}

impl ConfirmedRequestTracker {
    /// Whether to ignore a confirmed request from link-layer `source_mac`
    /// because that address reaches a group of nodes (`is_group`, the
    /// transport's `is_group_destination`). Its answer, any segment ACK, and
    /// the confirmed COV notifications of a subscription it makes would all
    /// go back there, to every node in the group (#1504). Each one is
    /// counted.
    pub(super) fn refuse_group_source(
        &self,
        source_mac: &[u8],
        is_group: impl FnOnce(&[u8]) -> bool,
    ) -> bool {
        if !is_group(source_mac) {
            return false;
        }
        self.group_source_drops.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(
            ?source_mac,
            "Ignoring a ConfirmedRequest from a group address"
        );
        true
    }
}

impl<T: bacnet_transport::port::TransportPort + 'static> super::BACnetServer<T> {
    /// Confirmed requests ignored since start because the link-layer address
    /// they came from is a group destination of the transport
    /// ([`TransportPort::is_group_destination`](bacnet_transport::port::TransportPort::is_group_destination)),
    /// such as a B/IP broadcast or multicast address (#1504). The answer, and
    /// the confirmed COV notifications of a subscription such a request
    /// makes, would go back to that address, so the request is neither
    /// executed nor answered. No built-in transport hands one up; a custom
    /// one can.
    pub fn group_source_request_drops(&self) -> u64 {
        let tracker = &self.confirmed_request_tracker;
        tracker.group_source_drops.load(Ordering::Relaxed)
    }
}

impl PendingConfirmedRequest {
    fn untracked(tracker: &Arc<ConfirmedRequestTracker>) -> Self {
        Self {
            tracker: Arc::clone(tracker),
            id: None,
        }
    }

    /// The entry this owns, if the tracker follows the request.
    pub(super) fn id(&self) -> Option<u64> {
        self.id
    }

    /// Note that `handle` answers this request in segments (#2770).
    pub(super) fn answering(&self, handle: &Arc<super::segmented_send::SegmentedSendHandle>) {
        if let Some(id) = self.id {
            self.tracker.note_answer(id, handle);
        }
    }
}

impl ConfirmedRequestTracker {
    /// End pending entry `id` now, ahead of its owner. The owner's drop
    /// then finds nothing to end: `begin` never gives an id twice.
    /// `#[inline]`: every confirmed request's guard ends its entry here.
    #[inline]
    pub(super) fn end(&self, id: u64) {
        self.state.locked().entries.retain(|entry| entry.id != id);
    }
}

impl Drop for PendingConfirmedRequest {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.tracker.end(id);
        }
    }
}

#[path = "confirmed_segment_repeats.rs"]
mod segment_repeats;

#[path = "confirmed_answers.rs"]
mod answers;

#[cfg(test)]
#[path = "confirmed_tracker_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "group_source_request_tests.rs"]
mod group_source_tests;
