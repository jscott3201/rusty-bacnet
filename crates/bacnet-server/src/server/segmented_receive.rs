use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub(super) async fn reap_expired_requests(
        network: &Arc<NetworkLayer<T>>,
        receivers: &mut HashMap<SegRecvKey, SegmentedRequestState>,
        now: Instant,
        receive_timeout: Duration,
    ) {
        // All selected states and payload charges are gone before any I/O.
        // This future belongs to dispatch, including cancellation on shutdown.
        let expired = expire_segmented_requests(receivers, now, receive_timeout);
        for expired in expired {
            Self::send_server_abort(
                network,
                &expired.source_mac,
                expired.source_network.as_ref(),
                &expired.route,
                expired.invoke_id,
                AbortReason::OTHER,
            )
            .await;
        }
    }

    /// Send a `'server' = TRUE` Abort back along the request's path.
    ///
    /// Split from `lifecycle.rs` to keep the 700-LOC file cap; no behavior
    /// change. Every Abort this dispatch loop originates answers a client's
    /// request, so the flag is always TRUE (Clause 20.1.9.1).
    pub(super) async fn send_server_abort(
        network: &Arc<NetworkLayer<T>>,
        source_mac: &MacAddr,
        source_network: Option<&NpduAddress>,
        route: &bacnet_network::response_route::ResponseRoute,
        invoke_id: u8,
        abort_reason: AbortReason,
    ) {
        let abort_pdu = Apdu::Abort(AbortPdu {
            sent_by_server: true,
            invoke_id,
            abort_reason,
        });
        let mut abort_buf = BytesMut::new();
        encode_apdu(&mut abort_buf, &abort_pdu).expect("valid APDU encoding");
        if let Err(e) = Self::send_confirmed_response_apdu(
            network,
            &abort_buf,
            source_mac,
            source_network,
            route,
        )
        .await
        {
            warn!(error = %e, reason = abort_reason.to_raw(), "Failed to send Abort");
        }
    }
}

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
    receivers: &HashMap<SegRecvKey, SegmentedRequestState>,
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
pub(crate) struct RequestPayload {
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
    receivers: &HashMap<SegRecvKey, SegmentedRequestState>,
    key: &SegRecvKey,
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

/// Fail-closed conflict for segmented request reassembly (RB-07).
/// Returns the conflicting key when the same (peer, invoke) exists under a
/// different non-direct scope; the caller aborts that session rather than
/// merging. Direct identities always own independent exact keys, including
/// when a non-direct frame claims their address.
pub(super) fn find_receive_provenance_conflict(
    receivers: &HashMap<SegRecvKey, SegmentedRequestState>,
    key: &SegRecvKey,
) -> Option<SegRecvKey> {
    if key.3.is_direct_peer() {
        return None;
    }
    receivers
        .keys()
        .find(|existing| {
            !existing.3.is_direct_peer()
                && existing.0 == key.0
                && existing.1 == key.1
                && existing.2 == key.2
                && existing.3 != key.3
        })
        .cloned()
}

/// A direct Abort cancels only its exact principal/incarnation key. Legacy
/// address-scoped cancellation may sweep non-direct contexts only. Whole-server
/// shutdown and timeouts retain their independent global cleanup semantics.
pub(super) fn remove_matching_reassemblies(
    receivers: &mut HashMap<SegRecvKey, SegmentedRequestState>,
    key: &SegRecvKey,
) {
    if key.3.is_direct_peer() {
        receivers.remove(key);
        return;
    }
    let doomed: Vec<SegRecvKey> = receivers
        .keys()
        .filter(|existing| {
            !existing.3.is_direct_peer()
                && existing.0 == key.0
                && existing.1 == key.1
                && existing.2 == key.2
        })
        .cloned()
        .collect();
    for doomed in doomed {
        receivers.remove(&doomed);
    }
}

/// Validate the active timing/declaration contract before starting network I/O.
pub(super) fn validate_segment_timeout(
    config: &ServerConfig,
    db: &ObjectDatabase,
) -> Result<Duration, Error> {
    if config.segmentation_supported.to_raw() > Segmentation::NONE.to_raw() {
        return Err(Error::Encoding(
            "invalid server segmentation support".into(),
        ));
    }
    let active = config.segmentation_supported != Segmentation::NONE;
    if active && config.apdu_segment_timeout_ms == 0 {
        return Err(Error::Encoding(
            "APDU segment timeout must be positive".into(),
        ));
    }
    let milliseconds = config
        .apdu_segment_timeout_ms
        .checked_mul(4)
        .ok_or_else(|| Error::Encoding("receive segment timeout overflows milliseconds".into()))?;
    let receive_timeout = Duration::from_millis(milliseconds);
    runtime_clock::now()
        .checked_add(receive_timeout)
        .and_then(|deadline| deadline.checked_add(Duration::from_nanos(1)))
        .ok_or_else(|| Error::Encoding("receive segment deadline is not representable".into()))?;
    if let Some(oid) = db.selected_device() {
        let device = db.get(&oid).expect("selected under same database guard");
        let declared = device.read_property(PropertyIdentifier::SEGMENTATION_SUPPORTED, None)?;
        if declared != PropertyValue::Enumerated(u32::from(config.segmentation_supported.to_raw()))
        {
            return Err(Error::Encoding(
                "selected Device Segmentation_Supported must match server configuration".into(),
            ));
        }
        if active
            && device.read_property(PropertyIdentifier::APDU_SEGMENT_TIMEOUT, None)?
                != PropertyValue::Unsigned(config.apdu_segment_timeout_ms)
        {
            return Err(Error::Encoding(
                "selected Device APDU_Segment_Timeout must match server configuration".into(),
            ));
        }
    }
    Ok(receive_timeout)
}

/// An expired local-policy response retains no reassembly payload or quota charge.
pub(super) struct ExpiredRequest {
    pub(super) source_mac: MacAddr,
    pub(super) source_network: Option<NpduAddress>,
    pub(super) route: bacnet_network::response_route::ResponseRoute,
    pub(super) invoke_id: u8,
}

/// Remove every due state before any response can await. Protocol expiry is
/// silent and wins if both conditions are already due when dispatch observes
/// them. At exact 4*Tseg equality the protocol timer is live, so a coincident
/// inclusive local progress deadline still produces OTHER.
pub(super) fn expire_segmented_requests(
    receivers: &mut HashMap<SegRecvKey, SegmentedRequestState>,
    now: Instant,
    receive_timeout: Duration,
) -> Vec<ExpiredRequest> {
    let mut expired = Vec::new();
    receivers.retain(|key, state| {
        if now.duration_since(state.last_activity) > receive_timeout {
            return false;
        }
        if now.duration_since(state.last_progress) >= SEG_RECEIVER_PROGRESS_TIMEOUT {
            expired.push(ExpiredRequest {
                source_mac: state.source_mac.clone(),
                source_network: state.source_network.clone(),
                route: bacnet_network::response_route::ResponseRoute::new(
                    state.provenance,
                    state.direct_response.clone(),
                ),
                invoke_id: key.2,
            });
            return false;
        }
        true
    });
    expired
}

/// A fresh scan of at most 128 entries avoids stale deadlines after activity,
/// completion or removal. Strict protocol expiry wakes one representable tick
/// beyond equality; tokio may round this up to its timer resolution.
pub(super) fn next_receive_deadline(
    receivers: &HashMap<SegRecvKey, SegmentedRequestState>,
    receive_timeout: Duration,
) -> Option<Instant> {
    receivers
        .values()
        .flat_map(|state| {
            let protocol = state
                .last_activity
                .checked_add(receive_timeout)
                .and_then(|deadline| deadline.checked_add(Duration::from_nanos(1)));
            let progress = state
                .last_progress
                .checked_add(SEG_RECEIVER_PROGRESS_TIMEOUT);
            [protocol, progress].into_iter().flatten()
        })
        .min()
}

pub(super) async fn wait_receive_deadline(
    receivers: &HashMap<SegRecvKey, SegmentedRequestState>,
    receive_timeout: Duration,
) {
    match next_receive_deadline(receivers, receive_timeout) {
        Some(deadline) => tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await,
        None => std::future::pending().await,
    }
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

/// Segment zero owns the immutable authorization and response snapshots.
pub(super) fn initial_state(
    payload: RequestPayload,
    provenance: bacnet_transport::port::TransportProvenance,
    direct_response: Option<bacnet_transport::port::DirectResponse>,
    source_mac: MacAddr,
    source_network: Option<NpduAddress>,
    request: &ConfirmedRequestPdu,
) -> (SegmentedRequestState, Option<SegmentAckPdu>) {
    let actual_window_size = request.proposed_window_size.unwrap_or(0);
    let should_ack = !request.more_follows || actual_window_size <= 1;
    let now = runtime_clock::now();
    let state = SegmentedRequestState {
        source_mac,
        source_network,
        payload,
        provenance,
        direct_response,
        last_activity: now,
        last_progress: now,
        expected_seq: 1,
        initial_sequence_number: 0,
        duplicate_count: 0,
        last_acked_seq: 0,
        window_pos: if should_ack { 0 } else { 1 },
        actual_window_size,
        accepted_segments: 1,
    };
    let ack = should_ack.then_some(SegmentAckPdu {
        negative_ack: false,
        sent_by_server: true,
        invoke_id: request.invoke_id,
        sequence_number: request.sequence_number.unwrap_or(0),
        actual_window_size,
    });
    (state, ack)
}

/// A client Abort retires only its matching active reassembly.
pub(super) fn remove_peer_aborted_request(
    receivers: &mut HashMap<SegRecvKey, SegmentedRequestState>,
    received: &bacnet_network::layer::ReceivedApdu,
    decoded: &Apdu,
) {
    if let Apdu::Abort(abort) = decoded {
        if !abort.sent_by_server {
            let key = segmented_receive_key(
                &received.source_mac,
                received.source_network.as_ref(),
                abort.invoke_id,
                received.provenance,
            );
            remove_matching_reassemblies(receivers, &key);
        }
    }
}
