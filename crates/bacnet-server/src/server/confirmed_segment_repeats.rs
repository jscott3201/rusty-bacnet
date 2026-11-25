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
//! So the segment branch notes, beside the pending entry `begin` made for
//! the request, the acknowledgment that completed it, and answers a repeat
//! from the same peer (same requester, trust context and route) and invoke
//! ID with it. The note lives as long as that entry, which ends when the
//! answer is handed to the link (for a segmented answer, as the dispatch
//! loop takes its final acknowledgment, before the next frame); a
//! requester's Abort for the invoke ID ends the note sooner. Once the answer
//! goes out in segments, a segment zero opens a transfer instead, so that a
//! different request can be told from the same one sent again
//! (`confirmed_answers.rs`, #2770). After that a
//! repeat finds no transfer and draws the Abort it always did. A request
//! the tracker doesn't follow (past its entry cap, or a LifeSafetyOperation,
//! which has its own replay) gets no note. Only the segment branch and a
//! requester's Abort touch the notes, so a request that isn't segmented
//! pays nothing for them.

use super::*;
use bacnet_network::response_route::ResponseRoute;
use bacnet_types::sync::MutexExt;

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
    /// The pending entries of this requester and invoke ID.
    fn ids_of<'a>(
        &'a self,
        requester: &'a CanonicalRequester,
        direct_identity: &'a Option<DirectScIdentity>,
        invoke_id: u8,
    ) -> impl DoubleEndedIterator<Item = u64> + 'a {
        self.entries
            .iter()
            .filter(move |entry| entry.is_under(requester, direct_identity, invoke_id))
            .map(|entry| entry.id)
    }
}

impl ConfirmedRequestTracker {
    /// The id `begin` gives the next request it admits. The dispatch loop
    /// alone calls `begin`, so the request it dispatches next gets this one,
    /// if any (#2751).
    pub(in crate::server) fn next_admission(&self) -> u64 {
        self.lock().next_id
    }

    /// Note that the request admitted as `id` from this peer, reassembled
    /// from segments, ended with a Segment-ACK naming `last_sequence` and
    /// `window`. Nothing is noted unless that entry is pending (it may have
    /// been answered already, or never admitted). Notes of ended entries go
    /// here, so there are never more than the entries.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::server) fn note_segmented(
        &self,
        id: u64,
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
        let mut state = self.lock();
        let TrackerState {
            entries, segmented, ..
        } = &mut *state;
        segmented.retain(|record| entries.iter().any(|entry| entry.id == record.id));
        if !state
            .ids_of(&requester, &direct_identity, invoke_id)
            .any(|pending| pending == id)
        {
            return;
        }
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
    /// same origin, repeats a request still executing. A segment zero
    /// (`opening`) while that request's answer is sent in segments draws
    /// none: it opens a transfer, whose request, once whole, is dropped if
    /// it is the same one and draws the Abort if not (#2770).
    pub(in crate::server) fn repeated_segment(
        &self,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        provenance: &TransportProvenance,
        route: &ResponseRoute,
        invoke_id: u8,
        opening: bool,
    ) -> Option<(u8, u8)> {
        let state = self.lock();
        if state.segmented.is_empty() {
            return None;
        }
        let requester = canonical_requester(source_mac, source_network);
        let direct_identity = provenance.direct_sc_identity();
        // The pending entries first (few), newest first, then their notes
        // by id: a newer request under the invoke ID is the one repeated.
        state
            .ids_of(&requester, &direct_identity, invoke_id)
            .rev()
            .find_map(|id| {
                state.segmented.iter().find(|record| {
                    record.id == id
                        && record.provenance == *provenance
                        && record.route.same_origin(route)
                })
            })
            .filter(|record| !(opening && state.answering(record.id)))
            .map(|record| (record.last_sequence, record.window))
    }

    /// A requester's Abort for `invoke_id` ends the notes of its requests:
    /// a new transfer under that invoke ID is not answered with an old
    /// acknowledgment.
    #[cold]
    pub(in crate::server) fn requester_aborted(
        &self,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        provenance: &TransportProvenance,
        invoke_id: u8,
    ) {
        let mut state = self.lock();
        if state.segmented.is_empty() {
            return;
        }
        let requester = canonical_requester(source_mac, source_network);
        let direct_identity = provenance.direct_sc_identity();
        let ended: Vec<u64> = state
            .ids_of(&requester, &direct_identity, invoke_id)
            .collect();
        state.segmented.retain(|record| !ended.contains(&record.id));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TrackerState> {
        self.state.locked()
    }

    /// Notes of pending requests, for tests.
    #[cfg(test)]
    pub(in crate::server) fn segmented_notes(&self) -> usize {
        let state = self.lock();
        state
            .segmented
            .iter()
            .filter(|record| state.entries.iter().any(|entry| entry.id == record.id))
            .count()
    }
}
