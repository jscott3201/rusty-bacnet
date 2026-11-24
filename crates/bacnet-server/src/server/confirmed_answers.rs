//! A request that comes while a segmented answer is sent (Clause 5.4.5.4,
//! #2770).
//!
//! Once a request's answer goes out in segments, its pending entry lasts
//! until the requester's final Segment-ACK. In that time the same requester
//! (same trust context) can send another request under the invoke ID. The
//! server's state machine answers a request there with an Abort and ends the
//! answer; this server does that only for a request that differs from the
//! one being answered. The same request again keeps the duplicate handling
//! it always had (dropped whole; a segment other than zero of a request that
//! came in segments acknowledged again with its last acknowledgment), so a
//! requester that lost the answer's first segment and sent its request again
//! still gets the answer from the server's own resend.
//!
//! The sending task notes its answer here as it starts, beside the pending
//! entry. `begin` looks at the notes only once an entry under the request's
//! requester, identity and invoke ID is pending, so a request with none pays
//! nothing for them. A different request then ends that entry, so the invoke
//! ID is free, and the answer: its task sends the Abort once it stops, after
//! any segment already on its way ([`SegmentedSendHandle::supersede`]). The
//! request itself is not executed. An answer whose task has stopped sending
//! is over, and the request is admitted as new.
//!
//! A request that comes in segments is known whole only once its last
//! segment is in; its segments are acknowledged first. A segment zero during
//! the answer therefore opens a transfer of its own rather than drawing the
//! note of a request that came in segments
//! ([`ConfirmedRequestTracker::repeated_segment`]), and the whole request is
//! judged here: dropped when it is the same request, an Abort when not.

use super::*;
use crate::server::segmented_send::SegmentedSendHandle;
use std::sync::Weak;

/// An answer the task `handle` sends in segments for pending entry `id`.
/// Weak: a note that outlives its task holds none of the task's channels.
pub(super) struct Answer {
    id: u64,
    handle: Weak<SegmentedSendHandle>,
}

impl ConfirmedRequestTracker {
    /// Note that pending entry `id` is answered in segments by `handle`.
    /// Nothing is noted unless the entry is pending. Notes of ended entries
    /// go here, so there are never more than the entries.
    pub(in crate::server) fn note_answer(&self, id: u64, handle: &Arc<SegmentedSendHandle>) {
        let mut state = self.lock_state();
        let TrackerState {
            entries, answers, ..
        } = &mut *state;
        answers.retain(|answer| entries.iter().any(|entry| entry.id == answer.id));
        if entries.iter().any(|entry| entry.id == id) {
            answers.push(Answer {
                id,
                handle: Arc::downgrade(handle),
            });
        }
    }

    /// End the segmented answer in progress under this requester, trust
    /// context and invoke ID for a request that differs from the one it
    /// answers; see [`TrackerState::supersede_answer`]. A LifeSafetyOperation,
    /// which the tracker's entries don't follow, asks this first.
    pub(in crate::server) fn supersede_answer(
        &self,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        provenance: &TransportProvenance,
        invoke_id: u8,
    ) -> bool {
        let mut state = self.lock_state();
        if state.answers.is_empty() {
            return false;
        }
        let requester = canonical_requester(source_mac, source_network);
        let direct_identity = provenance.direct_sc_identity();
        state.supersede_answer(&requester, &direct_identity, invoke_id)
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, TrackerState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl TrackerState {
    /// The rest of `begin`'s decision once entry `first` is pending under the
    /// request's requester, identity and invoke ID: a duplicate when one such
    /// entry holds this very request; else the Abort when one such entry's
    /// answer is in progress; else nothing (admit it as new).
    #[cold]
    pub(super) fn admit_same_key(
        &mut self,
        first: usize,
        requester: &CanonicalRequester,
        direct_identity: &Option<DirectScIdentity>,
        invoke_id: u8,
        request: &ConfirmedRequest,
    ) -> Option<ConfirmedRequestAdmission> {
        if self.entries[first..].iter().any(|entry| {
            entry.is_under(requester, direct_identity, invoke_id) && entry.request == *request
        }) {
            return Some(ConfirmedRequestAdmission::Duplicate);
        }
        self.supersede_answer(requester, direct_identity, invoke_id)
            .then_some(ConfirmedRequestAdmission::AbortsAnswer)
    }

    /// Supersede the answer under this key that is still sending, and end
    /// its entry. Newest first: a newer answer under a key cancels the
    /// older one, so at most one is sending once that lands. False, with
    /// nothing changed, when none is.
    fn supersede_answer(
        &mut self,
        requester: &CanonicalRequester,
        direct_identity: &Option<DirectScIdentity>,
        invoke_id: u8,
    ) -> bool {
        let Self {
            entries, answers, ..
        } = self;
        for at in (0..answers.len()).rev() {
            let answer = &answers[at];
            let under_key = entries.iter().any(|entry| {
                entry.id == answer.id && entry.is_under(requester, direct_identity, invoke_id)
            });
            if under_key
                && answer
                    .handle
                    .upgrade()
                    .is_some_and(|handle| handle.supersede())
            {
                let id = answers.remove(at).id;
                entries.retain(|entry| entry.id != id);
                return true;
            }
        }
        false
    }

    /// Whether pending entry `id` is answered in segments.
    pub(super) fn answering(&self, id: u64) -> bool {
        self.answers.iter().any(|answer| answer.id == id)
    }
}

#[cfg(test)]
#[path = "confirmed_answers_tests.rs"]
mod tests;
