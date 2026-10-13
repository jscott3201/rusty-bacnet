use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::request_peer::{canonical_requester, CanonicalRequester};
use bacnet_encoding::apdu::ConfirmedRequest;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_transport::port::{DirectScIdentity, TransportProvenance};

// Local detection bounds, not transaction lifetimes. Above either bound,
// Clause 5.3.5.3 permits normal service when exact duplicate detection is unavailable.
const MAX_ENTRIES: usize = 256;
const MAX_TRACKED_SERVICE_REQUEST_BYTES: usize = 64 * 1024;

struct Entry {
    id: u64,
    requester: CanonicalRequester,
    direct_identity: Option<DirectScIdentity>,
    invoke_id: u8,
    request: ConfirmedRequest,
}

#[derive(Default)]
struct TrackerState {
    next_id: u64,
    entries: Vec<Entry>,
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
}

pub(super) enum ConfirmedRequestAdmission {
    Duplicate,
    New(PendingConfirmedRequest),
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
        request: ConfirmedRequest,
    ) -> ConfirmedRequestAdmission {
        if request.service_request.len() > MAX_TRACKED_SERVICE_REQUEST_BYTES {
            return ConfirmedRequestAdmission::New(PendingConfirmedRequest::untracked(self));
        }
        let requester = canonical_requester(source_mac, source_network);
        let direct_identity = provenance.direct_sc_identity();
        let invoke_id = request.invoke_id;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.entries.iter().any(|entry| {
            entry.requester == requester
                && entry.direct_identity == direct_identity
                && entry.invoke_id == invoke_id
                && entry.request == request
        }) {
            return ConfirmedRequestAdmission::Duplicate;
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
}
impl Drop for PendingConfirmedRequest {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.tracker
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entries
                .retain(|entry| entry.id != id);
        }
    }
}

#[cfg(test)]
#[path = "confirmed_tracker_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "group_source_request_tests.rs"]
mod group_source_tests;
