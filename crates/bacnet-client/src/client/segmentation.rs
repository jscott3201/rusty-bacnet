//! Sending a segmented confirmed request (#2558), by phase.
//! `segmented_confirmed_request` opens the transaction and closes it; in
//! between, `send_segments` runs the transfer: `send_window` sends a
//! window's segments in order, `await_window_ack` waits for the window's
//! acknowledgement and sends the window again on each segment timeout, and
//! `take_window_ack` takes the acknowledgement, negative or not, and sets
//! the next window. Once the last window is acknowledged, the transfer
//! waits for the answer.
//!
//! Each body is the code it was, moved as it stood, except for two things:
//! it reaches the request's fixed values through `Transfer`, and it hands
//! back an outcome where it used to break out of the loop. There is no
//! behaviour change: the same frames go in the same order, under the same
//! locks, and the transaction guard keeps its scope in
//! `segmented_confirmed_request`. The module, and so the log target, is
//! unchanged.

use super::first_window::FirstWindow;
use super::segmentation_context::{ConfirmedWait, SegmentedRequestLimits};
use super::segmented_request::{OutgoingSegmentContext, OutgoingSegmentSend};
use super::source_audit::Refused;
use super::*;
use std::future::Future;

/// What one segmented request's transfer reads and leaves alone.
#[derive(Clone, Copy)]
struct Transfer<'a> {
    /// What each segment's header carries, and its transaction.
    send_context: OutgoingSegmentContext<'a>,
    /// The request's service data, split into segments.
    segments: &'a [Bytes],
    /// Tseg: the segment timeout.
    timeout_duration: Duration,
    /// How many times a window goes again before the request fails.
    max_ack_retries: u8,
    /// The first window's size (`first_window`, #2742).
    window_size: usize,
}

/// How the wait for a window's acknowledgement ended, other than with an
/// error.
enum WindowAck {
    /// The acknowledgement, of a segment in the window.
    Ack(SegmentAckPdu),
    /// The transaction ended, with this result.
    Terminal(Result<TsmResponse, Error>),
    /// A segmented response began: no more segments go.
    SegmentedResponse,
}

/// Split a request's service data into segments the peer takes, at most
/// `remote_max_segments` of them when it set a bound.
fn split_request(
    service_data: &[u8],
    remote_max_apdu: u16,
    remote_max_segments: Option<u32>,
) -> Result<Vec<Bytes>, Error> {
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
    Ok(segments)
}

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
        let segments = split_request(service_data, remote_max_apdu, remote_max_segments)?;
        let total_segments = segments.len();

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
                &tsm_mac,
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
        let (window_size, mut first) = self.first_window(&tsm_mac);
        let mut owns_tsm_cleanup = true;
        let transfer = Transfer {
            send_context: OutgoingSegmentContext {
                target,
                service_choice,
                advertised_max_apdu,
                remote_max_apdu,
                invoke_id,
                total_segments,
                tsm_mac: &tsm_mac,
                owner: &owner,
            },
            segments: &segments,
            timeout_duration,
            max_ack_retries,
            window_size,
        };

        let mut result = self
            .send_segments(
                transfer,
                &mut first,
                &mut owns_tsm_cleanup,
                registration.response,
                registration.progress,
                &mut seg_ack_rx,
            )
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

    /// The transfer: each window sent, then its acknowledgement awaited and
    /// taken, until the last; then the wait for the answer. A transaction
    /// that ends meanwhile ends it. `owns_tsm_cleanup` is cleared when the
    /// transaction's end is no longer the request's to clean up, and
    /// `first` follows how the first window went.
    ///
    /// Not an `async fn`, which would hold its moved arguments twice: the
    /// block below captures them once (#1713).
    #[allow(
        clippy::manual_async_fn,
        reason = "an async fn would hold its moved arguments twice (#1713)"
    )]
    fn send_segments<'a>(
        &'a self,
        transfer: Transfer<'a>,
        first: &'a mut FirstWindow,
        owns_tsm_cleanup: &'a mut bool,
        mut response_rx: oneshot::Receiver<TsmResponse>,
        mut progress_rx: ProgressReceiver,
        seg_ack_rx: &'a mut mpsc::Receiver<SegmentAckPdu>,
    ) -> impl Future<Output = Result<TsmResponse, Error>> + Send + 'a {
        async move {
            let total_segments = transfer.send_context.total_segments;
            let mut window_size = transfer.window_size;
            let mut next_seq: usize = 0;
            let mut neg_ack_retries: u32 = 0;
            let mut segmented_response_started = false;
            'send: while next_seq < total_segments {
                let window_start = next_seq;
                let window_end = (window_start + window_size).min(total_segments);

                match self
                    .send_window(
                        &transfer,
                        (window_start, window_end),
                        &mut response_rx,
                        &mut progress_rx,
                    )
                    .await?
                {
                    OutgoingSegmentSend::Sent => {}
                    OutgoingSegmentSend::Terminal(response) => {
                        *owns_tsm_cleanup = false;
                        return Ok(response);
                    }
                    OutgoingSegmentSend::SegmentedResponse => {
                        segmented_response_started = true;
                        break 'send;
                    }
                }

                let ack = match self
                    .await_window_ack(
                        &transfer,
                        (window_start, window_end),
                        first,
                        &mut response_rx,
                        &mut progress_rx,
                        seg_ack_rx,
                    )
                    .await?
                {
                    WindowAck::Ack(ack) => ack,
                    WindowAck::Terminal(response) => {
                        *owns_tsm_cleanup = false;
                        return response;
                    }
                    WindowAck::SegmentedResponse => {
                        segmented_response_started = true;
                        break 'send;
                    }
                };

                (window_size, next_seq) = self.take_window_ack(
                    *first,
                    window_start,
                    &ack,
                    transfer.send_context.tsm_mac,
                    &mut neg_ack_retries,
                )?;
            }

            let OutgoingSegmentContext {
                target,
                invoke_id,
                tsm_mac,
                owner,
                ..
            } = transfer.send_context;
            if !segmented_response_started {
                self.tsm
                    .lock()
                    .finish_segmented_request(tsm_mac, invoke_id, owner);
            }
            // Once entered, the phase-aware waiter owns terminal TSM removal.
            // A later key-only cancellation could target a reused invoke ID.
            *owns_tsm_cleanup = false;
            self.wait_for_confirmed_response(
                ConfirmedWait {
                    target,
                    tsm_mac,
                    invoke_id,
                    owner,
                    retry_apdu: None,
                },
                response_rx,
                progress_rx,
            )
            .await
        }
    }

    /// Send the segments of the window `window` (start, end) for the first
    /// time, in order; the last segment's send is the final segment's issue
    /// (`BACnetClient::send_outgoing_segment`). Ends early, with that
    /// outcome, when the transaction ends or a segmented response begins.
    #[allow(
        clippy::manual_async_fn,
        reason = "an async fn would hold its moved arguments twice (#1713)"
    )]
    fn send_window<'a>(
        &'a self,
        transfer: &'a Transfer<'a>,
        window: (usize, usize),
        response_rx: &'a mut oneshot::Receiver<TsmResponse>,
        progress_rx: &'a mut ProgressReceiver,
    ) -> impl Future<Output = Result<OutgoingSegmentSend, Error>> + Send + 'a {
        let (window_start, window_end) = window;
        async move {
            let total_segments = transfer.send_context.total_segments;
            for (seq, segment_data) in transfer.segments[window_start..window_end]
                .iter()
                .enumerate()
                .map(|(i, s)| (window_start + i, s))
            {
                let is_last = seq == total_segments - 1;
                let sent = self
                    .send_outgoing_segment(
                        transfer.send_context,
                        seq,
                        segment_data,
                        is_last,
                        response_rx,
                        progress_rx,
                    )
                    .await;
                // Once segment 0 left, a link that stopped under the request
                // cuts it off as sent (#1833).
                let sent = match sent {
                    Err(error) if seq > 0 => Err(super::requests::retry_refused(error)),
                    sent => sent,
                };
                match sent? {
                    OutgoingSegmentSend::Sent => {
                        debug!(seq, is_last, "Sent segment");
                    }
                    ended => return Ok(ended),
                }
            }
            Ok(OutgoingSegmentSend::Sent)
        }
    }

    /// Wait for the acknowledgement of the window `window` (start, end). An
    /// acknowledgement outside it is discarded, and each segment timeout
    /// sends the window again, until the retries run out; a window sent
    /// again issues no final segment.
    #[allow(
        clippy::manual_async_fn,
        reason = "an async fn would hold its moved arguments twice (#1713)"
    )]
    fn await_window_ack<'a>(
        &'a self,
        transfer: &'a Transfer<'a>,
        window: (usize, usize),
        first: &'a mut FirstWindow,
        response_rx: &'a mut oneshot::Receiver<TsmResponse>,
        progress_rx: &'a mut ProgressReceiver,
        seg_ack_rx: &'a mut mpsc::Receiver<SegmentAckPdu>,
    ) -> impl Future<Output = Result<WindowAck, Error>> + Send + 'a {
        let (window_start, window_end) = window;
        async move {
            // Wider than `apdu_retries`, so the expiry after the last of
            // 255 retries still ends the request (#2809).
            let mut ack_retries: u16 = 0;
            loop {
                let segment_timer = tokio::time::sleep(transfer.timeout_duration);
                tokio::pin!(segment_timer);
                tokio::select! {
                    biased;
                    response = &mut *response_rx => {
                        return Ok(WindowAck::Terminal(
                            response.map_err(|_| super::requests::cut_off(true)),
                        ));
                    }
                    // A transaction that left the TSM resolves the
                    // response arm above; progress stays silent.
                    () = progress_rx.changed() => {
                        if matches!(
                            progress_rx.borrow_and_update(),
                            TransactionProgress::SegmentedResponse { .. }
                        ) {
                            return Ok(WindowAck::SegmentedResponse);
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

                        return Ok(WindowAck::Ack(ack));
                        }
                        None => {
                        // Can't happen: the request holds a sender of this
                        // channel until its transfer ends. Were it to, the
                        // segments sent so far may have reached the device.
                        return Err(super::requests::cut_off(true));
                        }
                    },
                    _ = &mut segment_timer => {
                        *first = self.segment_zero_unanswered(
                            *first,
                            window_start,
                            transfer.send_context.tsm_mac,
                        );
                        ack_retries += 1;
                        if ack_retries > u16::from(transfer.max_ack_retries) {
                            return Err(Error::abort(bacnet_types::enums::AbortReason::TSM_TIMEOUT));
                        }
                        warn!(
                            attempt = ack_retries,
                            "Retransmitting segmented request window"
                        );
                        for (seq, segment_data) in transfer.segments[window_start..window_end]
                            .iter()
                            .enumerate()
                            .map(|(i, s)| (window_start + i, s))
                        {
                            let is_last = seq == transfer.send_context.total_segments - 1;
                            // A resend: the window left before, so a link
                            // that stopped since cuts the request off as
                            // sent (#1833).
                            match self
                                .send_outgoing_segment(
                                    transfer.send_context,
                                    seq,
                                    segment_data,
                                    false,
                                    response_rx,
                                    progress_rx,
                                )
                                .await
                                .map_err(super::requests::retry_refused)?
                            {
                                OutgoingSegmentSend::Sent => {
                                    debug!(seq, is_last, "Retransmitted segment");
                                }
                                OutgoingSegmentSend::Terminal(response) => {
                                    return Ok(WindowAck::Terminal(Ok(response)));
                                }
                                OutgoingSegmentSend::SegmentedResponse => {
                                    return Ok(WindowAck::SegmentedResponse);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Take a window's acknowledgement: the window it grants the next
    /// group, and the segment that group starts at. Returns the two.
    fn take_window_ack(
        &self,
        first: FirstWindow,
        window_start: usize,
        ack: &SegmentAckPdu,
        tsm_mac: &MacAddr,
        neg_ack_retries: &mut u32,
    ) -> Result<(usize, usize), Error> {
        // A local livelock bound with no counterpart in Clause 5.4.4.2, which
        // bounds SEGMENTED_REQUEST only by SegmentTimer/Nretry and resets
        // SegmentRetryCount on every in-window ack. Ten in-window NAKs in a
        // row means a peer that keeps asking for the same window; cutting the
        // transfer off is a local matter, like every bounded resource here.
        const MAX_NEG_ACK_RETRIES: u32 = 10;

        debug!(
            seq = ack.sequence_number,
            negative = ack.negative_ack,
            window = ack.actual_window_size,
            "Received SegmentAck"
        );

        let window_size = self.granted_window(first, window_start, ack, tsm_mac);

        // Clause 5.4.4.2 gives positive and negative acks the same
        // effect on the window: 'sequence-number' is the last segment
        // the peer accepted, and this side continues from the one
        // after it. The NAK counter is the only difference — a local
        // livelock bound on a peer that keeps re-asking for the same
        // window.
        let ack_seq = ack.sequence_number as usize;
        if ack.negative_ack {
            *neg_ack_retries += 1;
            if *neg_ack_retries > MAX_NEG_ACK_RETRIES {
                return Err(
                    bacnet_types::error::SegmentationError::NegativeAckRetriesExhausted.into(),
                );
            }
        } else {
            *neg_ack_retries = 0;
        }
        Ok((window_size, ack_seq + 1))
    }
}
