use super::*;

/// Private defensive limit, not a normative SegmentTimer or total-request age.
const SEG_RECEIVER_PROGRESS_TIMEOUT: Duration = Duration::from_secs(16);

/// Private resource partition, not an authenticated-peer fairness guarantee.
const MAX_SEG_RECEIVERS_PER_PEER: usize = 16;

/// Owner policy: logical saved service payload per server instance, not a
/// normative cap, heap/RSS bound, or application memory budget. Current input,
/// channels, metadata, allocation overhead/capacity and completed output are
/// excluded. Each accepted segment is copied once to detach its backing owner.
const MAX_SAVED_REQUEST_BYTES: usize = 4 * 1024 * 1024;

pub(super) fn saved_request_payload_bytes(
    receivers: &HashMap<SegKey, SegmentedRequestState>,
) -> Option<usize> {
    receivers.values().try_fold(0usize, |sum, state| {
        sum.checked_add(state.payload.saved_payload_bytes())
    })
}

fn payload_fits(saved: Option<usize>, additional: usize) -> bool {
    saved
        .and_then(|bytes| bytes.checked_add(additional))
        .is_some_and(|bytes| bytes <= MAX_SAVED_REQUEST_BYTES)
}

/// Owns all active payload storage and its accounting. The mutable encoding
/// receiver never escapes: server saves are append-only, ordered, and charged
/// exactly once, while the first request template contains metadata only.
pub(super) struct RequestPayload {
    receiver: SegmentReceiver,
    first: ConfirmedRequestPdu,
    saved_payload_bytes: usize,
}

impl RequestPayload {
    pub(super) fn new(first: &ConfirmedRequestPdu) -> Self {
        Self {
            receiver: SegmentReceiver::new(),
            first: reassembled_confirmed_request(first, Bytes::new()),
            saved_payload_bytes: 0,
        }
    }

    /// The aggregate snapshot includes this owner. Call only at the new-save
    /// point, with no await between snapshot, validation, save and charge.
    pub(super) fn save_new(
        &mut self,
        seq: u8,
        data: Bytes,
        aggregate: Option<usize>,
    ) -> Result<(), Error> {
        if usize::from(seq) != self.receiver.received_count()
            || self.receiver.received_count() >= MAX_REQUEST_SEGMENTS
            || !payload_fits(aggregate, data.len())
        {
            return Err(Error::Segmentation(
                "request payload capacity exceeded".into(),
            ));
        }
        let charged = self
            .saved_payload_bytes
            .checked_add(data.len())
            .ok_or_else(|| Error::Segmentation("request payload accounting overflow".into()))?;
        // Authoritative per-segment validation happens BEFORE copying. Insert
        // the original transiently, then replace the SAME key synchronously
        // with detached bytes of the validated length. No extra charge/save.
        self.receiver.receive(seq, data.clone())?;
        let detached = Bytes::copy_from_slice(&data);
        #[cfg(test)]
        let detached = tests::observe_saved_payload(detached);
        self.receiver
            .receive(seq, detached)
            .expect("same validated segment length");
        self.saved_payload_bytes = charged;
        Ok(())
    }

    pub(super) fn saved_payload_bytes(&self) -> usize {
        self.saved_payload_bytes
    }

    /// Consuming completion releases all saved owners before returning to async
    /// dispatch, even on error. Reassembly temporarily overlaps the output Vec
    /// with saved segments; output storage is outside the active payload budget.
    pub(super) fn complete(self, total: usize) -> Result<ConfirmedRequestPdu, Error> {
        let data = self.receiver.reassemble(total)?;
        Ok(reassembled_confirmed_request(
            &self.first,
            Bytes::from(data),
        ))
    }
}

#[cfg(test)]
#[path = "segmentation_tests/request_payload_owner.rs"]
pub(super) mod tests;

/// Only for a new, supported sequence-zero request with a valid window.
/// Global capacity takes precedence; the bounded key scan ignores invoke ID.
pub(super) fn segmented_request_admission_error(
    receivers: &HashMap<SegKey, SegmentedRequestState>,
    key: &SegKey,
) -> Option<AbortReason> {
    if receivers.len() >= MAX_SEG_RECEIVERS {
        return Some(AbortReason::BUFFER_OVERFLOW);
    }
    if receivers
        .keys()
        .filter(|existing| existing.0 == key.0 && existing.1 == key.1)
        .count()
        >= MAX_SEG_RECEIVERS_PER_PEER
    {
        return Some(AbortReason::OUT_OF_RESOURCES);
    }
    None
}

/// Drop stale incarnations and all their retained payload ownership before input.
/// Cleanup is silent and synchronous; idle or blocked dispatch is not reclaimed.
pub(super) fn expire_segmented_requests(
    receivers: &mut HashMap<SegKey, SegmentedRequestState>,
    now: Instant,
) {
    receivers.retain(|_key, state| {
        now.duration_since(state.last_activity) < SEG_RECEIVER_TIMEOUT
            && now.duration_since(state.last_progress) < SEG_RECEIVER_PROGRESS_TIMEOUT
    });
}

pub(super) fn reassembled_confirmed_request(
    first: &ConfirmedRequestPdu,
    service_request: Bytes,
) -> ConfirmedRequestPdu {
    ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        sequence_number: None,
        proposed_window_size: None,
        service_request,
        invoke_id: first.invoke_id,
        service_choice: first.service_choice,
        max_apdu_length: first.max_apdu_length,
        segmented_response_accepted: first.segmented_response_accepted,
        max_segments: first.max_segments,
    }
}

/// Apply the Clause 5.4.5.2 duplicate or out-of-order receive transition.
pub(super) fn classify_non_next_segment(
    state: &mut SegmentedRequestState,
    invoke_id: u8,
    sequence_number: u8,
) -> Option<SegmentAckPdu> {
    let is_duplicate = duplicate_in_window(
        sequence_number,
        state.initial_sequence_number,
        state.last_acked_seq,
    );
    if is_duplicate && state.duplicate_count < state.actual_window_size {
        state.duplicate_count += 1;
        debug!(
            invoke_id,
            seq = sequence_number,
            duplicate_count = state.duplicate_count,
            "Silently discarding duplicate segment"
        );
        return None;
    }

    if is_duplicate {
        warn!(
            invoke_id,
            seq = sequence_number,
            "Duplicate allowance exhausted, sending negative SegmentAck"
        );
    } else {
        warn!(
            invoke_id,
            expected = state.expected_seq,
            received = sequence_number,
            "Segment gap detected, sending negative SegmentAck"
        );
    }
    state.initial_sequence_number = state.last_acked_seq;
    state.duplicate_count = 0;
    Some(SegmentAckPdu {
        negative_ack: true,
        sent_by_server: true,
        invoke_id,
        sequence_number: state.last_acked_seq,
        actual_window_size: state.actual_window_size,
    })
}
