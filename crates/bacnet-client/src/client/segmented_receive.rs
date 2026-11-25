//! Receiving a segmented ComplexACK: admission, the reassembly session and
//! its window (#2558). Split from `segmentation.rs`, which keeps the sending
//! side, with no behavior change.
//!
//! `handle_segmented_complex_ack` runs two steps in turn: `admit_segment`
//! admits the segment to its transaction, and `receive_admitted_segment`
//! takes it into the transaction's reassembly session, acknowledges the
//! window and completes the transaction on the final segment.
//!
//! The log lines moved with the code, so their target is now
//! `bacnet_client::client::segmented_receive`, not
//! `bacnet_client::client::segmentation`: a log filter on the old target
//! no longer matches them.

use super::segmentation_context::{InboundSegmentSource, ReassemblyAbortTarget};
use super::*;
use crate::tsm::{CompletionOutcome, SegmentedAckArrival};
use bacnet_encoding::apdu::ComplexAck;

/// A segment its transaction admitted: what receiving it into the
/// reassembly reads besides the segment itself.
struct AdmittedSegment {
    /// The reassembly session's key.
    key: SegKey,
    /// TSM key MAC of the transaction, the key's first part.
    tsm_mac: MacAddr,
    /// The transaction's owner, as admission found it.
    owner: TransactionOwner,
    /// The segment's sequence number.
    seq: u8,
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Handle a segmented ComplexAck: accumulate segments, send SegmentAcks,
    /// and reassemble when all segments are received.
    ///
    /// Not an `async fn`: one moves its arguments into locals of its own, so
    /// its future would hold `source` and `ack` twice. The block below
    /// captures them once (#1713).
    #[allow(
        clippy::manual_async_fn,
        reason = "an async fn would hold its moved arguments twice (#1713)"
    )]
    pub(super) fn handle_segmented_complex_ack<'a>(
        tsm: &'a Arc<SyncMutex<Tsm>>,
        network: &'a Arc<NetworkLayer<T>>,
        seg_state: &'a mut HashMap<SegKey, SegmentedReceiveState>,
        source: InboundSegmentSource<'a>,
        ack: bacnet_encoding::apdu::ComplexAck,
        limits: ResponseLimits,
    ) -> impl Future<Output = ()> + Send + 'a {
        async move {
            let Some(admitted) =
                Self::admit_segment(tsm, network, seg_state, &source, &ack, limits).await
            else {
                return;
            };
            Self::receive_admitted_segment(tsm, network, seg_state, source, ack, limits, admitted)
                .await;
        }
    }

    /// Admit a segment to its transaction: its transaction peer, the
    /// provenance check, the admission loop, the stale-owner and
    /// session-capacity checks. `None` when the segment was refused, after
    /// whatever the refusal sends.
    async fn admit_segment(
        tsm: &Arc<SyncMutex<Tsm>>,
        network: &Arc<NetworkLayer<T>>,
        seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
        source: &InboundSegmentSource<'_>,
        ack: &ComplexAck,
        limits: ResponseLimits,
    ) -> Option<AdmittedSegment> {
        let InboundSegmentSource {
            mac: source_mac,
            network: source_network,
            ref provenance,
            warning,
        } = *source;
        let seq = ack.sequence_number.unwrap_or(0);
        let coordinator_apdu = Apdu::ComplexAck(ack.clone());
        let TransactionPeer {
            tsm_mac,
            canonical: canonical_peer,
        } = TransactionPeer::of_answer_in(
            tsm,
            source_mac,
            source_network.as_ref(),
            network.local_network_number().get(),
            &coordinator_apdu,
        )
        .await;
        let key = (tsm_mac.clone(), ack.invoke_id, provenance.clone());
        if let Some(conflict) = super::response_admission::find_provenance_conflict(
            seg_state,
            &tsm_mac,
            ack.invoke_id,
            provenance,
        ) {
            if let Some(state) = seg_state.remove(&conflict) {
                warn_throttled!(
                    warning,
                    invoke_id = ack.invoke_id,
                    "Aborting segmented reassembly on provenance mismatch (fail-closed)"
                );
                Self::abort_reassembly(
                    tsm,
                    network,
                    state.abort_target(&tsm_mac),
                    ack.invoke_id,
                    bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
                )
                .await;
            }
            return None;
        }

        let mut deferred_owner = None;
        let owner = loop {
            let admission = {
                let mut tsm = tsm.lock();
                if let Some(owner) = deferred_owner.as_ref() {
                    tsm.coordinated_admit_segmented_complex_ack_for_owner(
                        SegmentedAckArrival {
                            source_mac: &tsm_mac,
                            invoke_id: ack.invoke_id,
                            sequence_number: seq,
                            segmented_response_accepted: limits.segmented_response_accepted,
                        },
                        owner,
                        &canonical_peer,
                        &coordinator_apdu,
                    )
                } else {
                    tsm.coordinated_admit_segmented_complex_ack(
                        &tsm_mac,
                        ack.invoke_id,
                        seq,
                        limits.segmented_response_accepted,
                        &canonical_peer,
                        &coordinator_apdu,
                    )
                }
            };
            match admission {
                SegmentedResponseAdmission::Active(owner) => break owner,
                SegmentedResponseAdmission::FinalSegmentSendPolling { owner, issue } => {
                    issue.wait_until_polled().await;
                    deferred_owner = Some(owner);
                }
                SegmentedResponseAdmission::InitialResponseAborted { wire_reason } => {
                    seg_state.remove(&key);
                    Self::send_client_abort(
                        network,
                        source_mac,
                        source_network,
                        ack.invoke_id,
                        wire_reason,
                    )
                    .await;
                    return None;
                }
                SegmentedResponseAdmission::CoordinatorRejected => return None,
                SegmentedResponseAdmission::NoTransaction => {
                    seg_state.remove(&key);
                    Self::send_client_abort(
                        network,
                        source_mac,
                        source_network,
                        ack.invoke_id,
                        bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
                    )
                    .await;
                    return None;
                }
            }
        };

        debug!(
            invoke_id = ack.invoke_id,
            seq = seq,
            more = ack.more_follows,
            "Received segmented ComplexAck"
        );

        // Admission is owner-qualified on every segment. A receive session
        // left by an earlier owner cannot attach to a reused invoke ID.
        if seg_state
            .get(&key)
            .is_some_and(|state| !state.owner.same_as(&owner))
        {
            seg_state.remove(&key);
        }

        // Admission precedes this capacity check so an unsolicited segment
        // draws the Clause 5.4.4.1 Abort even when every slot is occupied.
        const MAX_CONCURRENT_SEG_SESSIONS: usize = 64;
        if !seg_state.contains_key(&key) && seg_state.len() >= MAX_CONCURRENT_SEG_SESSIONS {
            warn_throttled!(
                warning,
                invoke_id = ack.invoke_id,
                sessions = seg_state.len(),
                "Max concurrent segmented sessions reached, aborting newcomer"
            );
            Self::abort_reassembly(
                tsm,
                network,
                ReassemblyAbortTarget {
                    tsm_mac: &tsm_mac,
                    owner: &owner,
                    reply_mac: &MacAddr::from_slice(source_mac),
                    reply_network: source_network,
                },
                ack.invoke_id,
                bacnet_types::enums::AbortReason::BUFFER_OVERFLOW,
            )
            .await;
            return None;
        }

        Some(AdmittedSegment {
            key,
            tsm_mac,
            owner,
            seq,
        })
    }

    /// Take an admitted segment into its transaction's reassembly session,
    /// opening the session on the first: store it, acknowledge the window,
    /// and complete the transaction once the final segment is in.
    ///
    /// Not an `async fn`, for the reason `handle_segmented_complex_ack`
    /// gives: the arguments are taken apart here and the block captures the
    /// parts once.
    fn receive_admitted_segment<'a>(
        tsm: &'a Arc<SyncMutex<Tsm>>,
        network: &'a Arc<NetworkLayer<T>>,
        seg_state: &'a mut HashMap<SegKey, SegmentedReceiveState>,
        source: InboundSegmentSource<'a>,
        ack: ComplexAck,
        limits: ResponseLimits,
        admitted: AdmittedSegment,
    ) -> impl Future<Output = ()> + Send + 'a {
        let InboundSegmentSource {
            mac: source_mac,
            network: source_network,
            provenance,
            warning,
        } = source;
        let AdmittedSegment {
            key,
            tsm_mac,
            owner,
            seq,
        } = admitted;
        async move {
            let proposed_ws = ack.proposed_window_size.unwrap_or(1);
            let state = seg_state
                .entry(key.clone())
                .or_insert_with(|| SegmentedReceiveState {
                    receiver: SegmentReceiver::new(),
                    owner: owner.clone(),
                    provenance: provenance.clone(),
                    reply_mac: MacAddr::from_slice(source_mac),
                    reply_network: source_network.clone(),
                    expected_next_seq: 0,
                    initial_sequence_number: 0,
                    last_sequence_number: 0,
                    duplicate_count: 0,
                    window_position: 0,
                    actual_window_size: proposed_ws,
                    accepted_segments: 0,
                });
            // Compat-mode live read: the key already isolates contexts; the
            // snapshot must match the key or the session fails closed elsewhere.
            debug_assert_eq!(state.provenance, provenance);

            if state.accepted_segments > 0
                && tsm
                    .lock()
                    .record_segmented_response_activity(&tsm_mac, ack.invoke_id, &state.owner)
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
                Self::refuse_out_of_order_segment(
                    network,
                    state,
                    seq,
                    &ack,
                    source_mac,
                    source_network,
                    warning,
                )
                .await;
                return;
            }

            // Reached only by an in-order NEW segment: every duplicate and every
            // out-of-order segment returned above. That placement is the whole
            // point — Clause 5.4.4.4 requires a duplicate to be discarded, not
            // treated as an error, so a cap that duplicates could trip would abort
            // healthy transfers on retransmission.
            if state.accepted_segments >= limits.max_reassembly_segments {
                warn_throttled!(
                    warning,
                    invoke_id = ack.invoke_id,
                    accepted = state.accepted_segments,
                    limit = limits.max_reassembly_segments,
                    "Segmented response exceeds reassembly capacity, aborting"
                );
                let reply_mac = state.reply_mac.clone();
                let reply_network = state.reply_network.clone();
                let owner = state.owner.clone();
                seg_state.remove(&key);
                Self::abort_reassembly(
                    tsm,
                    network,
                    ReassemblyAbortTarget {
                        tsm_mac: &tsm_mac,
                        owner: &owner,
                        reply_mac: &reply_mac,
                        reply_network: &reply_network,
                    },
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
            let accepted = {
                let mut admission_tsm = first_accepted_segment.then(|| tsm.lock());
                if admission_tsm
                    .as_ref()
                    .is_some_and(|tsm| !tsm.owner_is_current(&tsm_mac, ack.invoke_id, &state.owner))
                {
                    Err(None)
                } else if let Err(e) = state.receiver.receive(seq, ack.service_ack) {
                    Err(Some(e))
                } else {
                    if let Some(tsm) = admission_tsm.as_mut() {
                        let generation =
                            tsm.begin_segmented_response(&tsm_mac, ack.invoke_id, &state.owner);
                        debug_assert!(
                            generation.is_some(),
                            "pending transaction disappeared while its TSM lock was held"
                        );
                    }
                    Ok(())
                }
            };
            match accepted {
                Ok(()) => {}
                Err(None) => {
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
                Err(Some(e)) => {
                    // This is another local failure to retain the segment, so
                    // Clause 5.4.4.4 wants the same Abort rather than leaving the
                    // caller to time out on a session that can no longer complete.
                    warn_throttled!(warning, error = %e, "Rejecting oversized segment");
                    let reply_mac = state.reply_mac.clone();
                    let reply_network = state.reply_network.clone();
                    let owner = state.owner.clone();
                    seg_state.remove(&key);
                    Self::abort_reassembly(
                        tsm,
                        network,
                        ReassemblyAbortTarget {
                            tsm_mac: &tsm_mac,
                            owner: &owner,
                            reply_mac: &reply_mac,
                            reply_network: &reply_network,
                        },
                        ack.invoke_id,
                        bacnet_types::enums::AbortReason::BUFFER_OVERFLOW,
                    )
                    .await;
                    return;
                }
            }
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
                if let Err(e) =
                    Self::send_reply_apdu(network, &buf, source_mac, source_network).await
                {
                    warn!(error = %e, "Failed to send SegmentAck");
                }
            }

            if !ack.more_follows {
                Self::complete_reassembly(
                    tsm,
                    seg_state,
                    key,
                    tsm_mac,
                    ack.invoke_id,
                    ack.service_choice,
                    warning,
                );
            }
        }
    }

    /// A segment that is not the next in order: discard a duplicate within
    /// the window's allowance, or answer with a negative SegmentAck.
    async fn refuse_out_of_order_segment(
        network: &Arc<NetworkLayer<T>>,
        state: &mut SegmentedReceiveState,
        seq: u8,
        ack: &ComplexAck,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        warning: &WarnThrottle,
    ) {
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
            warn_throttled!(
                warning,
                invoke_id = ack.invoke_id,
                seq,
                "Duplicate allowance exhausted, sending negative SegmentAck"
            );
        } else {
            state.initial_sequence_number = state.last_sequence_number;
            state.duplicate_count = 0;
            warn_throttled!(
                warning,
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
    }

    /// The final segment is in: reassemble the response and complete the
    /// transaction with it, or fail it when it doesn't reassemble or names
    /// another service.
    fn complete_reassembly(
        tsm: &Arc<SyncMutex<Tsm>>,
        seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
        key: SegKey,
        tsm_mac: MacAddr,
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        warning: &WarnThrottle,
    ) {
        let state = seg_state.remove(&key).unwrap();
        let total = state.receiver.received_count();
        match state.receiver.reassemble(total) {
            Ok(service_data) => {
                debug!(
                    invoke_id = invoke_id,
                    segments = total,
                    bytes = service_data.len(),
                    "Reassembled segmented ComplexAck"
                );
                let mut tsm = tsm.lock();
                let outcome = tsm.complete_admitted_transaction_for_owner(
                    &tsm_mac,
                    invoke_id,
                    &state.owner,
                    Some(service_choice),
                    TsmResponse::ComplexAck {
                        service_data: Bytes::from(service_data),
                    },
                );
                if let CompletionOutcome::ServiceChoiceMismatch { expected, observed } = outcome {
                    tsm.reset_segmented_response(&tsm_mac, invoke_id, &state.owner);
                    warn_throttled!(
                        warning,
                        invoke_id = invoke_id,
                        expected = expected.to_raw(),
                        observed = observed.to_raw(),
                        "Discarding reassembled ComplexAck labelled for a different service"
                    );
                }
            }
            Err(e) => {
                tsm.lock()
                    .reset_segmented_response(&tsm_mac, invoke_id, &state.owner);
                warn_throttled!(
                    warning,
                    error = %e,
                    "Failed to reassemble segmented ComplexAck"
                );
            }
        }
    }
}
