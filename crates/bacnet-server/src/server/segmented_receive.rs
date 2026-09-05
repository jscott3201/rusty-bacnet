use super::*;

/// Private defensive limit, not a normative SegmentTimer or total-request age.
const SEG_RECEIVER_PROGRESS_TIMEOUT: Duration = Duration::from_secs(16);

/// Private resource partition, not an authenticated-peer fairness guarantee.
const MAX_SEG_RECEIVERS_PER_PEER: usize = 16;

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
