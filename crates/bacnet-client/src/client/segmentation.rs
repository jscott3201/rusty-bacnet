use super::*;
use crate::tsm::CompletionOutcome;
use bacnet_encoding::apdu::advertised_max_segments;

/// Size of the sequence-number space, and so the hard reassembly ceiling.
///
/// A local storage bound, not a protocol one. Clause 20.1.5.4 makes the
/// sequence number modulo 256, so a longer response is entirely representable —
/// this client simply keys its segment store by that `u8` and cannot tell
/// segment 257 from segment 1. Clause 5.4.4.4 names this exact situation
/// (`NewSegmentReceived_NoSpace`, "the segment cannot be saved due to local
/// conditions") and prescribes the Abort below.
///
/// Exactly 256 segments reassemble correctly and must keep working; 257 is the
/// first that would corrupt the payload.
const SEQUENCE_NUMBER_SPACE: usize = 256;

impl ResponseLimits {
    /// The receive-side limits `config` puts on the wire.
    pub(super) fn from_config(config: &ClientConfig) -> Self {
        // Clause 20.1.2.4 defines max-segments-accepted as "the maximum number
        // of segments that the device will accept"; Clause 5.2.1.3 makes it
        // binding, requiring the segment count to be the smallest of the
        // sender's own limit and "(b) the maximum number of segments accepted
        // by the remote peer device" — which, for a ComplexACK, is "the 'Max
        // Segments Accepted' parameter of the BACnet-Confirmed-Request-PDU for
        // which this is a response". A peer that overruns it is therefore
        // non-conformant, not merely unusual.
        //
        // Only the rungs B'001'..B'110' name a number; B'000' and B'111'
        // promise nothing, and `advertised_max_segments` reports those as
        // `None`.
        let advertised =
            advertised_max_segments(config.max_segments).map_or(usize::MAX, usize::from);
        Self {
            segmented_response_accepted: config.segmented_response_accepted,
            max_reassembly_segments: advertised.min(SEQUENCE_NUMBER_SPACE),
        }
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Transmit an Abort this client originates.
    ///
    /// Every Abort a requesting BACnet-user sends carries `'server' = FALSE` —
    /// Clauses 5.4.4.1, 5.4.4.3 and 5.4.4.4 each spell it out — because the
    /// flag names the sender's role, not the error.
    pub(super) async fn send_client_abort(
        network: &Arc<NetworkLayer<T>>,
        reply_mac: &[u8],
        reply_network: &Option<NpduAddress>,
        invoke_id: u8,
        abort_reason: bacnet_types::enums::AbortReason,
    ) {
        let abort = Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id,
            abort_reason,
        });
        let mut buf = BytesMut::with_capacity(4);
        if let Err(e) = encode_apdu(&mut buf, &abort) {
            warn!(error = %e, reason = abort_reason.to_raw(), "Failed to encode Abort");
            return;
        }
        if let Err(e) = Self::send_reply_apdu(network, &buf, reply_mac, reply_network).await {
            warn!(error = %e, reason = abort_reason.to_raw(), "Failed to send Abort");
        }
    }

    /// Abort a reassembly in progress, telling both the peer and the caller.
    ///
    /// Clause 5.4.4.4 gives this same shape to every way SEGMENTED_CONF can
    /// end badly — `NewSegmentReceived_NoSpace` for a segment that "cannot be
    /// saved due to local conditions" and `UnexpectedPDU_Received` for a PDU
    /// that does not belong in the state. Both "transmit a BACnet-Abort-PDU
    /// with 'server' = FALSE", "send ABORT.indication ... to the local
    /// application program", and "enter the IDLE state"; only `abort-reason`
    /// differs. The local ABORT.indication is the waiting caller, so the
    /// transaction is completed rather than left to time out.
    ///
    /// The caller is responsible for having removed the `seg_state` entry —
    /// that is the "enter the IDLE state" half.
    pub(super) async fn abort_reassembly(
        tsm: &Arc<Mutex<Tsm>>,
        network: &Arc<NetworkLayer<T>>,
        tsm_mac: &MacAddr,
        reply_mac: &MacAddr,
        reply_network: &Option<NpduAddress>,
        invoke_id: u8,
        reason: bacnet_types::enums::AbortReason,
    ) {
        Self::send_client_abort(network, reply_mac, reply_network, invoke_id, reason).await;
        tsm.lock().await.complete_transaction(
            tsm_mac,
            invoke_id,
            None,
            TsmResponse::Abort {
                reason: reason.to_raw(),
            },
        );
    }

    /// Handle a segmented ComplexAck: accumulate segments, send SegmentAcks,
    /// and reassemble when all segments are received.
    pub(super) async fn handle_segmented_complex_ack(
        tsm: &Arc<Mutex<Tsm>>,
        network: &Arc<NetworkLayer<T>>,
        seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        ack: bacnet_encoding::apdu::ComplexAck,
        limits: ResponseLimits,
    ) {
        let seq = ack.sequence_number.unwrap_or(0);
        let tsm_mac = response_tsm_mac(source_mac, source_network);
        let key = (tsm_mac.clone(), ack.invoke_id);

        debug!(
            invoke_id = ack.invoke_id,
            seq = seq,
            more = ack.more_follows,
            "Received segmented ComplexAck"
        );

        // A segmented ComplexACK when this device does not support
        // segmentation is Clause 5.4.4.3's UnexpectedPDU_Received, which
        // requires an Abort with 'server' = FALSE. The transmitted reason
        // follows Clause 18.10, which names SEGMENTATION_NOT_SUPPORTED for
        // exactly this case.
        if !limits.segmented_response_accepted {
            let abort = Apdu::Abort(AbortPdu {
                sent_by_server: false,
                invoke_id: ack.invoke_id,
                abort_reason: bacnet_types::enums::AbortReason::SEGMENTATION_NOT_SUPPORTED,
            });
            let mut buf = BytesMut::with_capacity(4);
            if let Err(e) = encode_apdu(&mut buf, &abort) {
                warn!(error = %e, "Failed to encode segmentation-not-supported Abort");
                return;
            }
            let _ = Self::send_reply_apdu(network, &buf, source_mac, source_network).await;
            return;
        }

        // A reassembly session lives exactly as long as its transaction, so
        // the TSM is consulted for EVERY segment, not only at session open.
        // Clause 5.4's SEGMENTED_CONF is a state of the transaction itself:
        // once the transaction ends — peer Abort/Error/Reject, or the caller
        // cancelling on timeout — this device is in IDLE for that invoke ID,
        // and 5.4.4.1 UnexpectedSegmentInfoReceived names this exact PDU as
        // "an unexpected PDU indicating the existence of an active server
        // TSM", prescribing an answer, not silence: "transmit a
        // BACnet-Abort-PDU with 'server' = FALSE and 'abort-reason' =
        // INVALID_APDU_IN_THIS_STATE and enter the IDLE state." A session
        // whose transaction is gone is removed here rather than lingering to
        // ack segments nobody is waiting on (#367). The per-open version of
        // this gate also kept an unsolicited peer from occupying all 64
        // session slots; requiring it per-segment keeps that property.
        //
        // Residual, deliberate: `release_invoke_id` drops a peer's allocator
        // once every ID is free, so the caller's next request to this peer
        // reuses the same invoke ID and a stale segment stream can alias
        // onto the new pending transaction. This gate removes the orphaned
        // session; it cannot tell that aliased stream from a genuine one.
        //
        // Bind the guard so its scope is obvious: it must not be held across
        // the send below, and an `if let` here would extend it silently.
        let transaction_pending = {
            tsm.lock()
                .await
                .expected_service_choice(&tsm_mac, ack.invoke_id)
                .is_some()
        };
        if !transaction_pending {
            seg_state.remove(&key);
            debug!(
                invoke_id = ack.invoke_id,
                "Aborting segmented ComplexAck for a transaction that is not pending"
            );
            Self::send_client_abort(
                network,
                source_mac,
                source_network,
                ack.invoke_id,
                bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
            )
            .await;
            return;
        }

        // Below the pending gate: a non-pending segment must draw the 5.4.4.1
        // Abort above even when every session slot is occupied.
        const MAX_CONCURRENT_SEG_SESSIONS: usize = 64;
        if !seg_state.contains_key(&key) && seg_state.len() >= MAX_CONCURRENT_SEG_SESSIONS {
            warn!(
                invoke_id = ack.invoke_id,
                sessions = seg_state.len(),
                "Max concurrent segmented sessions reached, dropping segment"
            );
            return;
        }

        let proposed_ws = ack.proposed_window_size.unwrap_or(1);
        let state = seg_state
            .entry(key.clone())
            .or_insert_with(|| SegmentedReceiveState {
                receiver: SegmentReceiver::new(),
                reply_mac: MacAddr::from_slice(source_mac),
                reply_network: source_network.clone(),
                expected_next_seq: 0,
                initial_sequence_number: 0,
                last_sequence_number: 0,
                duplicate_count: 0,
                last_activity: Instant::now(),
                window_position: 0,
                actual_window_size: proposed_ws,
                accepted_segments: 0,
            });

        state.last_activity = Instant::now();
        if state.accepted_segments > 0
            && tsm
                .lock()
                .await
                .record_segmented_response_activity(&tsm_mac, ack.invoke_id)
                .is_none()
        {
            seg_state.remove(&key);
            Self::send_client_abort(
                network,
                source_mac,
                source_network,
                ack.invoke_id,
                bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
            )
            .await;
            return;
        }

        if seq != state.expected_next_seq {
            let is_duplicate = duplicate_in_window(
                seq,
                state.initial_sequence_number,
                state.last_sequence_number,
            );
            if is_duplicate && state.duplicate_count < state.actual_window_size {
                state.duplicate_count += 1;
                debug!(
                    invoke_id = ack.invoke_id,
                    seq,
                    duplicate_count = state.duplicate_count,
                    "Silently discarding duplicate segment"
                );
                return;
            }

            if is_duplicate {
                state.duplicate_count = 0;
                warn!(
                    invoke_id = ack.invoke_id,
                    seq, "Duplicate allowance exhausted, sending negative SegmentAck"
                );
            } else {
                state.initial_sequence_number = state.last_sequence_number;
                state.duplicate_count = 0;
                warn!(
                    invoke_id = ack.invoke_id,
                    expected = state.expected_next_seq,
                    received = seq,
                    "Segment gap detected, sending negative SegmentAck"
                );
            }
            let neg_ack = Apdu::SegmentAck(SegmentAckPdu {
                negative_ack: true,
                sent_by_server: false,
                invoke_id: ack.invoke_id,
                sequence_number: state.last_sequence_number,
                actual_window_size: state.actual_window_size,
            });
            let mut buf = BytesMut::with_capacity(4);
            if let Err(e) = encode_apdu(&mut buf, &neg_ack) {
                warn!(error = %e, "Failed to encode negative SegmentAck");
                return;
            }
            if let Err(e) = Self::send_reply_apdu(network, &buf, source_mac, source_network).await {
                warn!(error = %e, "Failed to send SegmentAck");
            }
            return;
        }

        // Reached only by an in-order NEW segment: every duplicate and every
        // out-of-order segment returned above. That placement is the whole
        // point — Clause 5.4.4.4 requires a duplicate to be discarded, not
        // treated as an error, so a cap that duplicates could trip would abort
        // healthy transfers on retransmission.
        if state.accepted_segments >= limits.max_reassembly_segments {
            warn!(
                invoke_id = ack.invoke_id,
                accepted = state.accepted_segments,
                limit = limits.max_reassembly_segments,
                "Segmented response exceeds reassembly capacity, aborting"
            );
            let reply_mac = state.reply_mac.clone();
            let reply_network = state.reply_network.clone();
            seg_state.remove(&key);
            Self::abort_reassembly(
                tsm,
                network,
                &tsm_mac,
                &reply_mac,
                &reply_network,
                ack.invoke_id,
                bacnet_types::enums::AbortReason::BUFFER_OVERFLOW,
            )
            .await;
            return;
        }

        let first_accepted_segment = state.accepted_segments == 0;
        // Save segment zero and enter SEGMENTED_CONF while holding the same
        // TSM lock used by RequestTimer expiry. This makes acceptance itself
        // the serialized transition: timeout cannot slip between the save and
        // the phase change to retransmit or cancel the request.
        let mut admission_tsm = if first_accepted_segment {
            Some(tsm.lock().await)
        } else {
            None
        };
        if admission_tsm.as_ref().is_some_and(|tsm| {
            tsm.expected_service_choice(&tsm_mac, ack.invoke_id)
                .is_none()
        }) {
            drop(admission_tsm);
            seg_state.remove(&key);
            Self::send_client_abort(
                network,
                source_mac,
                source_network,
                ack.invoke_id,
                bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
            )
            .await;
            return;
        }
        if let Err(e) = state.receiver.receive(seq, ack.service_ack) {
            // Also "the segment cannot be saved due to local conditions", so
            // Clause 5.4.4.4 wants the same Abort rather than leaving the
            // caller to time out on a session that can no longer complete.
            warn!(error = %e, "Rejecting oversized segment");
            let reply_mac = state.reply_mac.clone();
            let reply_network = state.reply_network.clone();
            drop(admission_tsm);
            seg_state.remove(&key);
            Self::abort_reassembly(
                tsm,
                network,
                &tsm_mac,
                &reply_mac,
                &reply_network,
                ack.invoke_id,
                bacnet_types::enums::AbortReason::BUFFER_OVERFLOW,
            )
            .await;
            return;
        }
        if let Some(tsm) = admission_tsm.as_mut() {
            let generation = tsm.begin_segmented_response(&tsm_mac, ack.invoke_id);
            debug_assert!(
                generation.is_some(),
                "pending transaction disappeared while its TSM lock was held"
            );
        }
        drop(admission_tsm);
        state.accepted_segments += 1;
        state.expected_next_seq = seq.wrapping_add(1);
        state.last_sequence_number = seq;
        state.window_position += 1;

        // Per-window SegmentAck: only ack at window boundary or final segment (Clause 5.2.2)
        let should_ack = !ack.more_follows || state.window_position >= state.actual_window_size;

        if should_ack {
            state.window_position = 0;
            state.initial_sequence_number = state.last_sequence_number;
            state.duplicate_count = 0;
            let seg_ack = Apdu::SegmentAck(SegmentAckPdu {
                negative_ack: false,
                sent_by_server: false,
                invoke_id: ack.invoke_id,
                sequence_number: seq,
                actual_window_size: state.actual_window_size,
            });
            let mut buf = BytesMut::with_capacity(4);
            if let Err(e) = encode_apdu(&mut buf, &seg_ack) {
                warn!(error = %e, "Failed to encode SegmentAck");
                return;
            }
            if let Err(e) = Self::send_reply_apdu(network, &buf, source_mac, source_network).await {
                warn!(error = %e, "Failed to send SegmentAck");
            }
        }

        if !ack.more_follows {
            let state = seg_state.remove(&key).unwrap();
            let total = state.receiver.received_count();
            match state.receiver.reassemble(total) {
                Ok(service_data) => {
                    debug!(
                        invoke_id = ack.invoke_id,
                        segments = total,
                        bytes = service_data.len(),
                        "Reassembled segmented ComplexAck"
                    );
                    let mut tsm = tsm.lock().await;
                    let outcome = tsm.complete_transaction(
                        &tsm_mac,
                        ack.invoke_id,
                        Some(ack.service_choice),
                        TsmResponse::ComplexAck {
                            service_data: Bytes::from(service_data),
                        },
                    );
                    if let CompletionOutcome::ServiceChoiceMismatch { expected, observed } = outcome
                    {
                        tsm.reset_segmented_response(&tsm_mac, ack.invoke_id);
                        warn!(
                            invoke_id = ack.invoke_id,
                            expected = expected.to_raw(),
                            observed = observed.to_raw(),
                            "Discarding reassembled ComplexAck labelled for a different service"
                        );
                    }
                }
                Err(e) => {
                    tsm.lock()
                        .await
                        .reset_segmented_response(&tsm_mac, ack.invoke_id);
                    warn!(error = %e, "Failed to reassemble segmented ComplexAck");
                }
            }
        }
    }
    /// Send a confirmed request using segmented transfer with windowed flow control.
    pub(super) async fn segmented_confirmed_request(
        &self,
        target: ConfirmedTarget<'_>,
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
        remote_max_apdu: u16,
        remote_max_segments: Option<u32>,
    ) -> Result<Bytes, Error> {
        let tsm_mac = target.tsm_mac();
        let advertised_max_apdu = self.advertised_max_apdu_length_for_target(target)?;
        let max_seg_size = max_segment_payload(remote_max_apdu, SegmentedPduType::ConfirmedRequest);
        let segments = split_payload(service_data, max_seg_size)?;
        let total_segments = segments.len();

        if let Some(max_seg) = remote_max_segments {
            if total_segments > max_seg as usize {
                return Err(Error::Segmentation(format!(
                    "request requires {} segments but remote accepts at most {}",
                    total_segments, max_seg
                )));
            }
        }

        debug!(
            total_segments,
            max_seg_size,
            service_data_len = service_data.len(),
            "Starting segmented confirmed request"
        );

        let (invoke_id, registration) = {
            let mut tsm = self.tsm.lock().await;
            let invoke_id = tsm.allocate_invoke_id(&tsm_mac).ok_or_else(|| {
                Error::Encoding("all invoke IDs exhausted for destination".into())
            })?;
            let registration =
                tsm.register_transaction_with_progress(tsm_mac.clone(), invoke_id, service_choice);
            (invoke_id, registration)
        };

        let (seg_ack_tx, mut seg_ack_rx) = mpsc::channel(16);
        {
            let key = (tsm_mac.clone(), invoke_id);
            self.seg_ack_senders.lock().await.insert(key, seg_ack_tx);
        }

        // Tseg: use APDU timeout for now (configurable via apdu_timeout_ms)
        let timeout_duration = Duration::from_millis(self.config.apdu_timeout_ms);
        let max_ack_retries = self.config.apdu_retries;
        let mut window_size = self.config.proposed_window_size.max(1) as usize;
        let mut next_seq: usize = 0;
        let mut neg_ack_retries: u32 = 0;
        // A local livelock bound with no counterpart in Clause 5.4.4.2, which
        // bounds SEGMENTED_REQUEST only by SegmentTimer/Nretry and resets
        // SegmentRetryCount on every in-window ack. Ten in-window NAKs in a
        // row means a peer that keeps asking for the same window; cutting the
        // transfer off is a local matter, like every bounded resource here.
        const MAX_NEG_ACK_RETRIES: u32 = 10;

        let result = async {
            while next_seq < total_segments {
                let window_start = next_seq;
                let window_end = (window_start + window_size).min(total_segments);

                for (seq, segment_data) in segments[window_start..window_end]
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (window_start + i, s))
                {
                    let is_last = seq == total_segments - 1;
                    let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                        segmented: true,
                        more_follows: !is_last,
                        segmented_response_accepted: self.config.segmented_response_accepted,
                        max_segments: self.config.max_segments,
                        max_apdu_length: advertised_max_apdu,
                        invoke_id,
                        sequence_number: Some(seq as u8),
                        proposed_window_size: Some(self.config.proposed_window_size.max(1)),
                        service_choice,
                        service_request: segment_data.clone(),
                    });

                    let mut buf = BytesMut::with_capacity(remote_max_apdu as usize);
                    encode_apdu(&mut buf, &pdu)?;

                    self.send_confirmed_target_apdu(target, &buf).await?;

                    debug!(seq, is_last, "Sent segment");
                }

                let ack = {
                    let mut ack_retries: u8 = 0;
                    loop {
                        match timeout(timeout_duration, seg_ack_rx.recv()).await {
                            Ok(Some(ack)) => {
                                let ack_seq = ack.sequence_number as usize;

                                // The sole gate on an inbound SegmentACK,
                                // standing in for Clause 5.4.2.1's
                                // InWindow(seqA, seqB). The 5.4.4.2 ack
                                // transitions apply it without regard to the
                                // 'negative-ack' flag: either flavor names
                                // the last segment the peer accepted, and
                                // either advances this side past it — a NAK
                                // differs only in asking again for what
                                // follows. (A NAK spelled as "resend from
                                // here" has no source in 5.4.4.2; treating
                                // sequence 0 that way made a lost first ack
                                // unrecoverable: the retransmitted segment 0
                                // draws NAK(0) from a live session, and
                                // "resend segment 0" loops it forever.)
                                // A sequence number outside the window —
                                // including one at or past this request's
                                // segment count, e.g. a duplicated ack from
                                // an earlier transfer aliased onto a reused
                                // invoke ID — is 5.4.4.2
                                // DuplicateACK_Received: "restart
                                // SegmentTimer and enter the
                                // SEGMENTED_REQUEST state to await an
                                // acknowledgment" — discard and keep
                                // waiting, never a failure (#368). The
                                // `continue` below re-enters the timeout
                                // call, which is the SegmentTimer restart.
                                let ack_in_current_window =
                                    ack_seq >= window_start && ack_seq < window_end;

                                if !ack_in_current_window {
                                    debug!(
                                        seq = ack.sequence_number,
                                        negative = ack.negative_ack,
                                        window_start,
                                        window_end,
                                        "Ignoring SegmentAck outside current send window"
                                    );
                                    continue;
                                }

                                break ack;
                            }
                            Ok(None) => {
                                return Err(Error::Encoding("SegmentAck channel closed".into()));
                            }
                            Err(_timeout) => {
                                ack_retries += 1;
                                if ack_retries > max_ack_retries {
                                    return Err(Error::Timeout(timeout_duration));
                                }
                                warn!(
                                    attempt = ack_retries,
                                    "Retransmitting segmented request window"
                                );
                                for (seq, segment_data) in segments[window_start..window_end]
                                    .iter()
                                    .enumerate()
                                    .map(|(i, s)| (window_start + i, s))
                                {
                                    let is_last = seq == total_segments - 1;
                                    let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                                        segmented: true,
                                        more_follows: !is_last,
                                        segmented_response_accepted: self
                                            .config
                                            .segmented_response_accepted,
                                        max_segments: self.config.max_segments,
                                        max_apdu_length: advertised_max_apdu,
                                        invoke_id,
                                        sequence_number: Some(seq as u8),
                                        proposed_window_size: Some(
                                            self.config.proposed_window_size.max(1),
                                        ),
                                        service_choice,
                                        service_request: segment_data.clone(),
                                    });

                                    let mut buf = BytesMut::with_capacity(remote_max_apdu as usize);
                                    encode_apdu(&mut buf, &pdu)?;

                                    self.send_confirmed_target_apdu(target, &buf).await?;
                                }
                            }
                        }
                    }
                };

                debug!(
                    seq = ack.sequence_number,
                    negative = ack.negative_ack,
                    window = ack.actual_window_size,
                    "Received SegmentAck"
                );

                window_size = ack.actual_window_size.max(1) as usize;

                // Clause 5.4.4.2 gives positive and negative acks the same
                // effect on the window: 'sequence-number' is the last segment
                // the peer accepted, and this side continues from the one
                // after it. The NAK counter is the only difference — a local
                // livelock bound on a peer that keeps re-asking for the same
                // window.
                let ack_seq = ack.sequence_number as usize;
                if ack.negative_ack {
                    neg_ack_retries += 1;
                    if neg_ack_retries > MAX_NEG_ACK_RETRIES {
                        return Err(Error::Segmentation(
                            "too many negative SegmentAck retransmissions".into(),
                        ));
                    }
                } else {
                    neg_ack_retries = 0;
                }
                next_seq = ack_seq + 1;
            }

            self.wait_for_confirmed_response(
                target,
                &tsm_mac,
                invoke_id,
                registration.response,
                registration.progress,
                None,
            )
            .await
        }
        .await;

        {
            let key = (tsm_mac.clone(), invoke_id);
            self.seg_ack_senders.lock().await.remove(&key);
        }

        let response = match result {
            Ok(response) => response,
            Err(e) => {
                let mut tsm = self.tsm.lock().await;
                tsm.cancel_transaction(&tsm_mac, invoke_id);
                return Err(e);
            }
        };

        Self::confirmed_response_result(response)
    }
}
