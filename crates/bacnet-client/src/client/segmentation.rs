use super::segmentation_context::{ConfirmedWait, SegmentedRequestLimits};
use super::segmented_request::{OutgoingSegmentContext, OutgoingSegmentSend};
use super::source_audit::Refused;
use super::*;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Send a confirmed request using segmented transfer with windowed flow control.
    pub(super) async fn segmented_confirmed_request(
        &self,
        target: ConfirmedTarget<'_>,
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
        limits: &mut SegmentedRequestLimits<'_>,
    ) -> Result<Bytes, Error> {
        let SegmentedRequestLimits {
            remote_max_apdu,
            remote_max_segments,
            routed_forwarded_npci_len,
            attributes,
            ..
        } = *limits;
        let transaction_peer = target.transaction_peer();
        let tsm_mac = transaction_peer.tsm_mac;
        let advertised_max_apdu = self.advertised_max_apdu_length_for_target(target)?;
        let max_seg_size = max_segment_payload(remote_max_apdu, SegmentedPduType::ConfirmedRequest);
        let segments = split_payload(service_data, max_seg_size)?;
        let total_segments = segments.len();

        if let Some(max_seg) = remote_max_segments
            && total_segments > max_seg as usize
        {
            return Err(bacnet_types::error::SegmentationError::PeerSegmentLimit {
                segments: total_segments,
                max: max_seg,
            }
            .into());
        }

        debug!(
            total_segments,
            max_seg_size,
            service_data_len = service_data.len(),
            "Starting segmented confirmed request"
        );

        let (seg_ack_tx, mut seg_ack_rx) = mpsc::channel(16);
        // A refusal is a closed seat, the node's DeviceCommunicationControl,
        // or a request the node audits (#2617).
        let (invoke_id, registration) = match self.register_transaction(
            tsm_mac.clone(),
            transaction_peer.canonical,
            service_choice,
            true,
        ) {
            Ok(registered) => registered,
            Err(refused) => match self.on_refused(refused, target, service_choice, true)? {
                Refused::Registered(registered) => registered,
                Refused::Audited(auditor) => {
                    return self
                        .audited_segmented(auditor, service_choice, service_data, limits)
                        .await;
                }
            },
        };

        let owner = registration.owner.clone();
        // The transaction keeps the request's data attributes, so each
        // segment and the wait's sends carry them from there (#2723).
        owner.set_data_attributes(attributes);
        if let Some(forwarded_npci_len) = routed_forwarded_npci_len {
            self.routed_path_limits.install_active(
                target,
                tsm_mac.clone(),
                invoke_id,
                owner.clone(),
                forwarded_npci_len,
                self.network.network_control_ingress_sequence(),
            );
        }
        let mut guard = TransactionGuard::new(
            Arc::clone(&self.tsm),
            self.cleanup_tx.clone(),
            tsm_mac.clone(),
            invoke_id,
            owner.clone(),
            Some(seg_ack_tx.clone()),
        );
        {
            let key = (tsm_mac.clone(), invoke_id);
            self.seg_ack_senders.lock().insert(
                key,
                SegmentAckRoute {
                    owner: owner.clone(),
                    sender: seg_ack_tx.clone(),
                },
            );
        }

        // Tseg: the segment timeout, which defaults to the APDU timeout.
        let timeout_duration = Duration::from_millis(self.config.segment_timeout_ms());
        let max_ack_retries = self.config.apdu_retries;
        // Segment zero goes alone, carrying the window this side proposes;
        // the receiver's answer to it grants the window every later group
        // uses (Clause 5.4.4.1), unless this peer once left it unanswered
        // (`first_window`, #2742).
        let (mut window_size, mut first) = self.first_window(&tsm_mac);
        let mut next_seq: usize = 0;
        let mut neg_ack_retries: u32 = 0;
        // A local livelock bound with no counterpart in Clause 5.4.4.2, which
        // bounds SEGMENTED_REQUEST only by SegmentTimer/Nretry and resets
        // SegmentRetryCount on every in-window ack. Ten in-window NAKs in a
        // row means a peer that keeps asking for the same window; cutting the
        // transfer off is a local matter, like every bounded resource here.
        const MAX_NEG_ACK_RETRIES: u32 = 10;
        let mut owns_tsm_cleanup = true;
        let mut response_rx = registration.response;
        let mut progress_rx = registration.progress;
        let send_context = OutgoingSegmentContext {
            target,
            service_choice,
            advertised_max_apdu,
            remote_max_apdu,
            invoke_id,
            total_segments,
            tsm_mac: &tsm_mac,
            owner: &owner,
        };

        let mut result = async {
            let mut segmented_response_started = false;
            'send: while next_seq < total_segments {
                let window_start = next_seq;
                let window_end = (window_start + window_size).min(total_segments);

                for (seq, segment_data) in segments[window_start..window_end]
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (window_start + i, s))
                {
                    let is_last = seq == total_segments - 1;
                    match self
                        .send_outgoing_segment(
                            send_context,
                            seq,
                            segment_data,
                            is_last,
                            &mut response_rx,
                            &mut progress_rx,
                        )
                        .await?
                    {
                        OutgoingSegmentSend::Sent => {
                            debug!(seq, is_last, "Sent segment");
                        }
                        OutgoingSegmentSend::Terminal(response) => {
                            owns_tsm_cleanup = false;
                            return Ok(response);
                        }
                        OutgoingSegmentSend::SegmentedResponse => {
                            segmented_response_started = true;
                            break 'send;
                        }
                    }
                }

                let ack = {
                    let mut ack_retries: u8 = 0;
                    loop {
                        let segment_timer = tokio::time::sleep(timeout_duration);
                        tokio::pin!(segment_timer);
                        tokio::select! {
                            biased;
                            response = &mut response_rx => {
                                owns_tsm_cleanup = false;
                                return response.map_err(|_| {
                                    Error::Encoding("TSM response channel closed".into())
                                });
                            }
                            // A transaction that left the TSM resolves the
                            // response arm above; progress stays silent.
                            () = progress_rx.changed() => {
                                if matches!(
                                    progress_rx.borrow_and_update(),
                                    TransactionProgress::SegmentedResponse { .. }
                                ) {
                                    segmented_response_started = true;
                                    break 'send;
                                }
                            }
                            received = seg_ack_rx.recv() => match received {
                                Some(ack) => {
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
                                // DuplicateACK_Received: SegmentTimer starts
                                // over and the transfer stays in
                                // SEGMENTED_REQUEST — discard and keep
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
                                None => {
                                return Err(Error::Encoding("SegmentAck channel closed".into()));
                                }
                            },
                            _ = &mut segment_timer => {
                                first = self.segment_zero_unanswered(first, window_start, &tsm_mac);
                                ack_retries += 1;
                                if ack_retries > max_ack_retries {
                                    return Err(Error::abort(bacnet_types::enums::AbortReason::TSM_TIMEOUT));
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
                                    match self
                                        .send_outgoing_segment(
                                            send_context,
                                            seq,
                                            segment_data,
                                            false,
                                            &mut response_rx,
                                            &mut progress_rx,
                                        )
                                        .await?
                                    {
                                        OutgoingSegmentSend::Sent => {
                                            debug!(seq, is_last, "Retransmitted segment");
                                        }
                                        OutgoingSegmentSend::Terminal(response) => {
                                            owns_tsm_cleanup = false;
                                            return Ok(response);
                                        }
                                        OutgoingSegmentSend::SegmentedResponse => {
                                                segmented_response_started = true;
                                                break 'send;
                                        }
                                    }
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

                window_size = self.granted_window(first, window_start, &ack, &tsm_mac);

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
                        return Err(
                            bacnet_types::error::SegmentationError::NegativeAckRetriesExhausted
                                .into(),
                        );
                    }
                } else {
                    neg_ack_retries = 0;
                }
                next_seq = ack_seq + 1;
            }

            if !segmented_response_started {
                self.tsm
                    .lock()
                    .finish_segmented_request(&tsm_mac, invoke_id, &owner);
            }
            // Once entered, the phase-aware waiter owns terminal TSM removal.
            // A later key-only cancellation could target a reused invoke ID.
            owns_tsm_cleanup = false;
            self.wait_for_confirmed_response(
                ConfirmedWait {
                    target,
                    tsm_mac: &tsm_mac,
                    invoke_id,
                    owner: &owner,
                    retry_apdu: None,
                },
                response_rx,
                progress_rx,
            )
            .await
        }
        .await;
        self.segment_zero_answered(first, &mut seg_ack_rx, &tsm_mac);
        self.note_answer(limits.sized_by, &result);
        if let Some(lease) = limits.routed_path_lease.as_ref() {
            self.settle_routed(lease, (&tsm_mac, invoke_id, &owner), &mut result);
        }

        #[cfg(test)]
        self.segmented_post_wait_cleanup.pause_if_enabled().await;

        {
            let key = (tsm_mac.clone(), invoke_id);
            let mut senders = self.seg_ack_senders.lock();
            if senders.get(&key).is_some_and(|route| {
                route.owner.same_as(&owner) && route.sender.same_channel(&seg_ack_tx)
            }) {
                senders.remove(&key);
            }
        }

        let response = match result {
            Ok(response) => response,
            Err(e) => {
                if owns_tsm_cleanup {
                    let mut tsm = self.tsm.lock();
                    tsm.cancel_transaction_for_owner(&tsm_mac, invoke_id, &owner);
                    drop(tsm);
                    self.enqueue_transaction_cleanup(
                        &tsm_mac,
                        invoke_id,
                        &owner,
                        false,
                        Some(seg_ack_tx.clone()),
                    );
                }
                guard.mark_completed();
                return Err(e);
            }
        };

        guard.mark_completed();
        Self::confirmed_response_result(response)
    }
}
