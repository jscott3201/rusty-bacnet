//! A segment repeated while its request executes (Clause 5.4.5.3, #2751).
//!
//! Once a segmented request is whole the shared receiver forgets it, and the
//! request runs as a pending transaction of the tracker until it is
//! answered. A requester whose last Segment-ACK was lost, or one that sent a
//! window ahead of an acknowledgement, can send a segment of it again in
//! that time. The standard has the server acknowledge such a segment again,
//! with the last sequence number and the window, and go on; an Abort there
//! could reach the requester before the answer and report a failure for a
//! request that took effect.
//!
//! So the segment branch notes, beside the request's pending entry, the
//! acknowledgment that completed it, and answers a repeat from the same
//! peer (same requester, trust context and route) and invoke ID with it.
//! The note lives exactly as long as the pending entry: one whose entry has
//! ended is dropped when the next note is made or looked up, and a repeat
//! after the answer finds no transfer and draws the Abort it always did. A
//! request the tracker doesn't follow (over its size bound, past its entry
//! cap, or a LifeSafetyOperation, which has its own replay) gets no note.
//! Only the segment branch touches the notes, so a request that isn't
//! segmented pays nothing for them.

use super::*;
use bacnet_network::response_route::ResponseRoute;

/// The acknowledgment a completed segmented request ended with, kept while
/// its pending entry `id` lives.
pub(super) struct Record {
    id: u64,
    provenance: TransportProvenance,
    route: ResponseRoute,
    last_sequence: u8,
    window: u8,
}

impl TrackerState {
    /// Drop the notes whose request has been answered.
    fn prune(&mut self) {
        let Self {
            entries, segmented, ..
        } = self;
        segmented.retain(|record| entries.iter().any(|entry| entry.id == record.id));
    }
}

impl ConfirmedRequestTracker {
    /// Note that the request `invoke_id` from this peer, reassembled from
    /// segments, ended with a Segment-ACK naming `last_sequence` and
    /// `window`. Nothing is noted when the request isn't pending (answered
    /// already, or not followed).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::server) fn note_segmented(
        &self,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        provenance: TransportProvenance,
        route: ResponseRoute,
        invoke_id: u8,
        last_sequence: u8,
        window: u8,
    ) {
        let requester = canonical_requester(source_mac, source_network);
        let direct_identity = provenance.direct_sc_identity();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.prune();
        let Some(id) = state
            .entries
            .iter()
            .filter(|entry| {
                entry.requester == requester
                    && entry.direct_identity == direct_identity
                    && entry.invoke_id == invoke_id
            })
            .map(|entry| entry.id)
            .max()
        else {
            return;
        };
        state.segmented.retain(|record| record.id != id);
        state.segmented.push(Record {
            id,
            provenance,
            route,
            last_sequence,
            window,
        });
    }

    /// The last sequence number and window to acknowledge again, when a
    /// segment from this peer, in this trust context and by a route of the
    /// same origin, repeats a request still executing.
    pub(in crate::server) fn repeated_segment(
        &self,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        provenance: &TransportProvenance,
        route: &ResponseRoute,
        invoke_id: u8,
    ) -> Option<(u8, u8)> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.segmented.is_empty() {
            return None;
        }
        state.prune();
        let requester = canonical_requester(source_mac, source_network);
        let direct_identity = provenance.direct_sc_identity();
        let TrackerState {
            entries, segmented, ..
        } = &*state;
        segmented
            .iter()
            .find(|record| {
                record.provenance == *provenance
                    && record.route.same_origin(route)
                    && entries.iter().any(|entry| {
                        entry.id == record.id
                            && entry.requester == requester
                            && entry.direct_identity == direct_identity
                            && entry.invoke_id == invoke_id
                    })
            })
            .map(|record| (record.last_sequence, record.window))
    }

    /// Notes kept, for tests.
    #[cfg(test)]
    pub(in crate::server) fn segmented_notes(&self) -> usize {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.prune();
        state.segmented.len()
    }
}
