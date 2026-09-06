use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};

use bacnet_encoding::apdu::{AbortPdu, SegmentAck as SegmentAckPdu};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::MacAddr;

use super::segmented_receive::RequestPayload;
use super::{DEFAULT_APDU_SEGMENT_RETRIES, DEFAULT_APDU_SEGMENT_TIMEOUT};

/// Key for tracking segmented transactions by peer and invoke ID.
pub(crate) type SegKey = (MacAddr, Option<NpduAddress>, u8);

pub(crate) fn segmented_transaction_key(
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    invoke_id: u8,
) -> SegKey {
    match source_network {
        Some(address)
            if (1..=0xFFFE).contains(&address.network) && !address.mac_address.is_empty() =>
        {
            (MacAddr::new(), Some(address.clone()), invoke_id)
        }
        _ => (
            MacAddr::from_slice(source_mac),
            source_network.cloned(),
            invoke_id,
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
}

pub(crate) struct SegmentedSendHandle {
    pub(crate) segment_ack_tx: mpsc::Sender<SegmentAckPdu>,
    pub(crate) control_tx: watch::Sender<Option<SegmentedSendControlEvent>>,
    pub(crate) closed: AtomicBool,
    pub(crate) current_sequence: AtomicU16,
    pub(crate) total_segments: usize,
}

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
    ) -> Self {
        Self {
            segment_ack_tx,
            control_tx,
            closed: AtomicBool::new(false),
            current_sequence: AtomicU16::new(u16::MAX),
            total_segments,
        }
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

pub(crate) struct SegmentedRequestState {
    pub(crate) payload: RequestPayload,
    pub(crate) last_activity: Instant,
    /// Last successfully saved new in-order segment, independent of SegmentTimer.
    pub(crate) last_progress: Instant,
    pub(crate) expected_seq: u8,
    /// Last sequence number in the previously completed receive window.
    pub(crate) initial_sequence_number: u8,
    /// Duplicates silently discarded in the current receive window.
    pub(crate) duplicate_count: u8,
    /// Last segment accepted in order (Clause 5.4.2 LastSequenceNumber).
    pub(crate) last_acked_seq: u8,
    pub(crate) window_pos: u8,
    pub(crate) actual_window_size: u8,
    /// Monotonic count of segments accepted in order (#364).
    ///
    /// The reassembly total. `expected_seq` cannot serve: Clause 20.1.2.7
    /// makes the sequence number modulo 256, so a 260-segment request ends at
    /// sequence 3 and `seq + 1` names a four-segment total. This counter also
    /// carries the overrun cap — acceptance is strictly in order, so it
    /// reaches [`MAX_REQUEST_SEGMENTS`] exactly when the sequence number is
    /// about to wrap onto stored segment 0.
    pub(crate) accepted_segments: usize,
}
