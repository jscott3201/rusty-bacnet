use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{mpsc, watch};

use bacnet_encoding::apdu::{AbortPdu, SegmentAck as SegmentAckPdu};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::MacAddr;
use bacnet_types::sync::MutexExt;

use super::{DEFAULT_APDU_SEGMENT_RETRIES, DEFAULT_APDU_SEGMENT_TIMEOUT};

/// Key for tracking segmented transactions by peer and invoke ID.
pub(crate) type SegKey = (
    MacAddr,
    Option<NpduAddress>,
    u8,
    Option<bacnet_transport::port::DirectScIdentity>,
);

/// Short, synchronous registry sections permit cleanup during future drop.
#[derive(Default)]
pub(super) struct SegmentedSendRegistry {
    senders: Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>,
    /// The server's throttle on warnings about segmented sends a peer can
    /// set off: sender caps reached, and SegmentAcks ignored or dropped
    /// (#1950).
    pub(super) warning: bacnet_transport::warn_throttle::WarnThrottle,
}

impl SegmentedSendRegistry {
    pub(super) fn lock(&self) -> MutexGuard<'_, HashMap<SegKey, Arc<SegmentedSendHandle>>> {
        // Map operations preserve their invariants if a caller unwinds. Cleanup
        // must not panic again while dropping an already-panicking worker.
        self.senders.locked()
    }
}

pub(super) struct SegmentedSendRegistration<'a> {
    pub(super) registry: &'a SegmentedSendRegistry,
    pub(super) key: SegKey,
    pub(super) sender: mpsc::Sender<SegmentAckPdu>,
    // Fields drop after Drop runs: removal always precedes permit release.
    pub(super) _permit: tokio::sync::OwnedSemaphorePermit,
}

impl Drop for SegmentedSendRegistration<'_> {
    fn drop(&mut self) {
        let mut senders = self.registry.lock();
        if senders
            .get(&self.key)
            .is_some_and(|entry| entry.same_channel(&self.sender))
        {
            senders.remove(&self.key);
        }
    }
}

pub(crate) fn segmented_transaction_key(
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    invoke_id: u8,
    provenance: TransportProvenance,
) -> SegKey {
    match source_network {
        Some(address)
            if (1..=0xFFFE).contains(&address.network) && !address.mac_address.is_empty() =>
        {
            (
                MacAddr::new(),
                Some(address.clone()),
                invoke_id,
                provenance.direct_sc_identity(),
            )
        }
        _ => (
            MacAddr::from_slice(source_mac),
            source_network.cloned(),
            invoke_id,
            provenance.direct_sc_identity(),
        ),
    }
}

#[derive(Debug)]
pub(crate) enum SegmentedSendEvent {
    SegmentAck(SegmentAckPdu),
    Abort(AbortPdu),
}

#[derive(Debug, Clone)]
pub(crate) enum SegmentedSendControlEvent {
    Abort(AbortPdu),
    Cancel,
    /// A different request came under the answer's requester and invoke
    /// ID: stop, and send the Abort if one is owed ([`SegmentedSendHandle::supersede`]).
    Supersede,
}

/// How a different request under its requester and invoke ID ended an
/// answer in progress ([`SegmentedSendHandle::supersede`], #2770).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Superseded {
    /// Midway: the sending task sends the Abort, and the request is not
    /// executed.
    Aborted,
    /// Every segment had gone out, none was on its way, and only the final
    /// Segment-ACK was due: it is taken as lost, no Abort goes out, and the
    /// request is served as a new one.
    FinalAckTaken,
    /// An Abort under the invoke ID is still to be issued (handed to the
    /// transport): for a different request before, or the task's own. The
    /// request is dropped, so it can't run ahead of that Abort.
    AbortPending,
}

pub(crate) struct SegmentedSendHandle {
    pub(crate) segment_ack_tx: mpsc::Sender<SegmentAckPdu>,
    pub(crate) control_tx: watch::Sender<Option<SegmentedSendControlEvent>>,
    pub(crate) closed: AtomicBool,
    pub(crate) current_sequence: AtomicU16,
    pub(crate) total_segments: usize,
    /// The duplicate tracker's entry for the request this answers, if it
    /// has one: see [`Self::end_request`].
    request: Option<u64>,
    /// Where the sending task is, for a different request that comes
    /// meanwhile: see [`Self::supersede`].
    phase: AtomicU8,
}

/// The sending task has segments to send, has one on its way, or waits for
/// an acknowledgment of one that isn't the last.
const SENDING: u8 = 0;
/// A different request came: the task sends nothing more of the answer,
/// and the Abort once it stops.
const SUPERSEDED: u8 = 1;
/// The task sends nothing more, and owes nothing.
const FINISHED: u8 = 2;
/// Every segment has gone out, none is on its way, and only the final
/// Segment-ACK is due.
const LAST_SENT: u8 = 3;
/// The task has stopped and is issuing an Abort: the one a different
/// request is owed, or its own.
const ABORTING: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SegmentAckDisposition {
    Advance,
    Retransmit,
}

pub(crate) fn segment_ack_disposition(
    ack: &SegmentAckPdu,
    current: usize,
    total_segments: usize,
) -> Option<SegmentAckDisposition> {
    if current >= total_segments {
        return None;
    }

    let ack_seq = ack.sequence_number as usize;
    if ack_seq >= total_segments {
        return None;
    }

    // Clause 5.4.4.2 treats either ACK flavor's sequence number as the last
    // segment accepted. A NAK for the preceding segment asks for `current`
    // again; a NAK for `current` confirms it and advances the send window.
    if ack_seq == current {
        Some(SegmentAckDisposition::Advance)
    } else if ack.negative_ack && current.checked_sub(1) == Some(ack_seq) {
        Some(SegmentAckDisposition::Retransmit)
    } else {
        None
    }
}

impl SegmentedSendHandle {
    pub(crate) fn new(
        segment_ack_tx: mpsc::Sender<SegmentAckPdu>,
        control_tx: watch::Sender<Option<SegmentedSendControlEvent>>,
        total_segments: usize,
        request: Option<u64>,
    ) -> Self {
        Self {
            segment_ack_tx,
            control_tx,
            closed: AtomicBool::new(false),
            current_sequence: AtomicU16::new(u16::MAX),
            total_segments,
            request,
            phase: AtomicU8::new(SENDING),
        }
    }

    /// End the answered request's tracker entry, and with it the note a
    /// repeated segment of it is answered from, as the dispatch loop takes
    /// the requester's final Segment-ACK or Abort (Clause 5.4.5.4: either
    /// returns the transaction to IDLE). The sending task, which owns the
    /// entry, ends it too, but only once it runs, and the loop takes the
    /// next frame first: a request reusing the invoke ID right behind
    /// would otherwise be judged against a transaction already over.
    pub(super) fn end_request(&self, tracker: &super::ConfirmedRequestTracker) {
        // With an Abort to issue, the entry lasts until it is handed to the
        // transport, so nothing runs under the invoke ID ahead of it (#2770).
        if let Some(id) = self.request
            && !self.abort_pending()
        {
            tracker.end(id);
        }
    }

    /// The requester's final Segment-ACK, as the dispatch loop queues it:
    /// the task may send nothing again, then the entry ends
    /// ([`Self::end_request`]). A resend of the last segment its timer chose
    /// just before it reads that acknowledgment would otherwise follow a
    /// new request under the reused invoke ID (#2770).
    pub(super) fn final_ack(&self, tracker: &super::ConfirmedRequestTracker) {
        let _ = self
            .phase
            .try_update(Ordering::AcqRel, Ordering::Acquire, |phase| match phase {
                SENDING | LAST_SENT => Some(FINISHED),
                _ => None,
            });
        self.end_request(tracker);
    }

    /// End the answer for a different request that came under its
    /// requester and invoke ID (Clause 5.4.5.4, #2770); the task sends
    /// nothing more of it. Midway ([`Superseded::Aborted`]), once it stops,
    /// after any segment already on its way, the task sends the Abort
    /// (INVALID_APDU_IN_THIS_STATE) itself, so no segment can follow it.
    /// With every segment out and only the final Segment-ACK due
    /// ([`Superseded::FinalAckTaken`]), that acknowledgment is taken as
    /// lost and nothing is sent. Already ended midway, with its Abort not
    /// yet issued: [`Superseded::AbortPending`]. `None` when the answer is
    /// over: stopped by the task (given up), cancelled by a newer one or
    /// aborted by the requester; then nothing changes here.
    pub(super) fn supersede(&self) -> Option<Superseded> {
        let phase = self.phase.load(Ordering::Acquire);
        if phase == SUPERSEDED || phase == ABORTING {
            return Some(Superseded::AbortPending);
        }
        if self.closed.load(Ordering::Acquire) {
            return None;
        }
        let ended = self
            .phase
            .try_update(Ordering::AcqRel, Ordering::Acquire, |phase| match phase {
                SENDING => Some(SUPERSEDED),
                // The task owes nothing, and may not send again.
                LAST_SENT => Some(FINISHED),
                _ => None,
            })
            .ok()?;
        self.send_control(SegmentedSendControlEvent::Supersede);
        Some(if ended == LAST_SENT {
            Superseded::FinalAckTaken
        } else {
            Superseded::Aborted
        })
    }

    /// Before each send of a segment: whether the task may send it. A
    /// segment sent again after the last one went out puts the answer back
    /// on its way, so a different request meanwhile draws the Abort.
    pub(super) fn sending(&self) -> bool {
        match self
            .phase
            .compare_exchange(LAST_SENT, SENDING, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => true,
            Err(phase) => phase == SENDING,
        }
    }

    /// The last segment has gone out: from here until the task sends again,
    /// a different request takes the final Segment-ACK as lost.
    pub(super) fn last_sent(&self) {
        let _ =
            self.phase
                .compare_exchange(SENDING, LAST_SENT, Ordering::AcqRel, Ordering::Acquire);
    }

    /// The sending task sends nothing more of the answer; true when it owes
    /// the Abort a [`Self::supersede`] asked for, which it then issues
    /// (hands to the transport). True once only, so exactly one Abort goes
    /// out whichever of the task's ways out asks first.
    pub(super) fn finish(&self) -> bool {
        self.phase
            .try_update(Ordering::AcqRel, Ordering::Acquire, |phase| match phase {
                SUPERSEDED => Some(ABORTING),
                ABORTING => None,
                _ => Some(FINISHED),
            })
            == Ok(SUPERSEDED)
    }

    /// The task gives up with an Abort of its own; true when a different
    /// request had ended the answer first, whose Abort this one then is.
    /// Until it is issued, requests under the invoke ID are dropped, as for
    /// a superseded answer, so none is served ahead of it (#2770).
    pub(super) fn aborting(&self) -> bool {
        self.phase.swap(ABORTING, Ordering::AcqRel) == SUPERSEDED
    }

    /// Whether an Abort under the invoke ID is still to be issued (#2770):
    /// the requester's own Abort then leaves the entry to that issue.
    pub(super) fn abort_pending(&self) -> bool {
        let phase = self.phase.load(Ordering::Acquire);
        phase == SUPERSEDED || phase == ABORTING
    }

    pub(crate) fn accepts_segment_ack(&self, ack: &SegmentAckPdu) -> bool {
        if ack.sent_by_server || self.closed.load(Ordering::Acquire) {
            return false;
        }

        let current = self.current_sequence.load(Ordering::Acquire) as usize;
        if current >= self.total_segments {
            return false;
        }

        segment_ack_disposition(ack, current, self.total_segments).is_some()
    }

    pub(crate) fn send_control(&self, event: SegmentedSendControlEvent) {
        self.closed.store(true, Ordering::Release);
        self.control_tx.send_replace(Some(event));
    }

    pub(crate) fn same_channel(&self, sender: &mpsc::Sender<SegmentAckPdu>) -> bool {
        self.segment_ack_tx.same_channel(sender)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SegmentedSendOptions {
    pub(crate) segment_timeout: Duration,
    pub(crate) max_retries: u8,
}

impl Default for SegmentedSendOptions {
    fn default() -> Self {
        Self {
            segment_timeout: DEFAULT_APDU_SEGMENT_TIMEOUT,
            max_retries: DEFAULT_APDU_SEGMENT_RETRIES,
        }
    }
}
