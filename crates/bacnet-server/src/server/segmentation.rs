use super::*;

/// The handles a segmented ComplexAck transfer runs on: the network it sends
/// through, the registry that routes SegmentAcks to it and the permits that
/// bound live senders.
pub(super) struct SegmentedSendResources<'a, T: TransportPort + 'static> {
    pub(super) network: &'a Arc<NetworkLayer<T>>,
    pub(super) seg_ack_senders: &'a Arc<segmented_send::SegmentedSendRegistry>,
    pub(super) seg_send_permits: &'a Arc<Semaphore>,
}

/// Identity of the confirmed request a ComplexAck answers and the limits the
/// client advertised for the response.
#[derive(Clone, Copy)]
pub(super) struct ComplexAckParams {
    pub(super) invoke_id: u8,
    pub(super) service_choice: ConfirmedServiceChoice,
    pub(super) client_max_apdu: u16,
    pub(super) client_max_segments: Option<u8>,
}

/// Whether a requester's Abort, a newer answer under the same requester and
/// invoke ID, or a different request under them has closed this send; logs
/// which.
fn send_closed(
    handle: &SegmentedSendHandle,
    control_rx: &mut watch::Receiver<Option<SegmentedSendControlEvent>>,
    registry: &segmented_send::SegmentedSendRegistry,
    invoke_id: u8,
) -> bool {
    if !handle.closed.load(Ordering::Acquire) {
        return false;
    }
    match control_rx.borrow_and_update().clone() {
        Some(SegmentedSendControlEvent::Abort(abort)) => {
            warn_throttled!(
                registry.warning,
                invoke_id = abort.invoke_id,
                reason = ?abort.abort_reason,
                "Client aborted segmented ComplexAck send"
            );
        }
        Some(SegmentedSendControlEvent::Cancel) | None => {
            warn_throttled!(
                registry.warning,
                invoke_id,
                "Segmented ComplexAck send cancelled by newer same-peer transaction"
            );
        }
        Some(SegmentedSendControlEvent::Supersede) => {
            warn_throttled!(
                registry.warning,
                invoke_id,
                "Segmented ComplexAck send ended by a different request under its invoke ID"
            );
        }
    }
    true
}

#[derive(Debug)]
enum SegmentedSendWaitResult {
    SegmentAck(SegmentAckPdu),
    Abort(AbortPdu),
    Cancel,
    ChannelClosed,
    Timeout,
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Register without awaiting the transfer: parent notifications stay concurrent.
    pub(super) fn spawn_segmented_complex_ack(
        resources: SegmentedSendResources<'_, T>,
        request_tasks: &super::request_tasks::RequestTaskSpawner,
        target: ResponseTarget<'_>,
        ack: ComplexAckParams,
        service_ack_data: Bytes,
        pending: Option<PendingConfirmedRequest>,
        segment_timeout: Duration,
    ) {
        let route = target.route.clone();
        let network = Arc::clone(resources.network);
        let seg_ack_senders = Arc::clone(resources.seg_ack_senders);
        let seg_send_permits = Arc::clone(resources.seg_send_permits);
        let source_mac = MacAddr::from_slice(target.source_mac);
        let source_network = target.source_network.cloned();
        request_tasks.spawn(async move {
            Self::send_segmented_complex_ack_with_options(
                SegmentedSendResources {
                    network: &network,
                    seg_ack_senders: &seg_ack_senders,
                    seg_send_permits: &seg_send_permits,
                },
                ResponseTarget {
                    source_mac: &source_mac,
                    source_network: source_network.as_ref(),
                    route: &route,
                },
                ack,
                &service_ack_data,
                SegmentedSendOptions {
                    segment_timeout,
                    ..Default::default()
                },
                pending,
            )
            .await;
        });
    }

    /// Send a ComplexAck response using segmented transfer.
    ///
    /// Splits the service ack data into segments that fit within the client's
    /// max APDU length, sends each segment, and waits for SegmentAck from
    /// the client before sending the next (window size 1).
    #[cfg(test)]
    pub(super) async fn send_segmented_complex_ack(
        resources: SegmentedSendResources<'_, T>,
        target: ResponseTarget<'_>,
        ack: ComplexAckParams,
        service_ack_data: &[u8],
        pending: Option<PendingConfirmedRequest>,
    ) {
        Self::send_segmented_complex_ack_with_options(
            resources,
            target,
            ack,
            service_ack_data,
            SegmentedSendOptions::default(),
            pending,
        )
        .await;
    }

    pub(super) async fn send_segmented_complex_ack_with_options(
        resources: SegmentedSendResources<'_, T>,
        target: ResponseTarget<'_>,
        ack: ComplexAckParams,
        service_ack_data: &[u8],
        options: SegmentedSendOptions,
        mut pending: Option<PendingConfirmedRequest>,
    ) {
        let SegmentedSendResources {
            network,
            seg_ack_senders,
            seg_send_permits,
        } = resources;
        let ResponseTarget {
            source_mac,
            source_network,
            route,
        } = target;
        let ComplexAckParams {
            invoke_id,
            service_choice,
            client_max_apdu,
            client_max_segments,
        } = ack;
        let max_seg_size = max_segment_payload(client_max_apdu, SegmentedPduType::ComplexAck);
        let segments = match split_payload(service_ack_data, max_seg_size) {
            Ok(segments) => segments,
            Err(e) => {
                warn_throttled!(
                    seg_ack_senders.warning,
                    error = %e,
                    "Response requires too many segments, aborting"
                );
                let abort = Apdu::Abort(AbortPdu {
                    sent_by_server: true,
                    invoke_id,
                    abort_reason: AbortReason::BUFFER_OVERFLOW,
                });
                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &abort).expect("valid APDU encoding");
                let _ = Self::issue_terminal_response(
                    network,
                    &buf,
                    source_mac,
                    source_network,
                    route,
                    pending.take(),
                )
                .await;
                return;
            }
        };
        let total_segments = segments.len();

        if let Some(max_segments) = client_max_segments
            && total_segments > max_segments as usize
        {
            warn_throttled!(
                seg_ack_senders.warning,
                total_segments,
                max_segments,
                "Response exceeds client's max-segments-accepted, aborting"
            );
            let abort = Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: AbortReason::BUFFER_OVERFLOW,
            });
            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &abort).expect("valid APDU encoding");
            let _ = Self::issue_terminal_response(
                network,
                &buf,
                source_mac,
                source_network,
                route,
                pending.take(),
            )
            .await;
            return;
        }

        if total_segments > 256 {
            warn_throttled!(
                seg_ack_senders.warning,
                total_segments,
                "Response requires too many segments, aborting"
            );
            let abort = Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: AbortReason::BUFFER_OVERFLOW,
            });
            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &abort).expect("valid APDU encoding");
            let _ = Self::issue_terminal_response(
                network,
                &buf,
                source_mac,
                source_network,
                route,
                pending.take(),
            )
            .await;
            return;
        }

        let _sender_permit = match Arc::clone(seg_send_permits).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                warn_throttled!(
                    seg_ack_senders.warning,
                    invoke_id,
                    limit = MAX_SEG_SENDERS,
                    "Too many live segmented ComplexAck senders, aborting"
                );
                let abort = Apdu::Abort(AbortPdu {
                    sent_by_server: true,
                    invoke_id,
                    abort_reason: AbortReason::BUFFER_OVERFLOW,
                });
                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &abort).expect("valid APDU encoding");
                let _ = Self::issue_terminal_response(
                    network,
                    &buf,
                    source_mac,
                    source_network,
                    route,
                    pending.take(),
                )
                .await;
                return;
            }
        };

        debug!(
            total_segments,
            max_seg_size,
            payload_len = service_ack_data.len(),
            "Starting segmented ComplexAck send"
        );

        let (seg_ack_tx, mut seg_ack_rx) = mpsc::channel::<SegmentAckPdu>(16);
        let (control_tx, mut control_rx) =
            watch::channel::<Option<SegmentedSendControlEvent>>(None);
        let send_handle = Arc::new(SegmentedSendHandle::new(
            seg_ack_tx.clone(),
            control_tx,
            total_segments,
            pending.as_ref().and_then(PendingConfirmedRequest::id),
        ));
        let key =
            segmented_transaction_key(source_mac, source_network, invoke_id, route.provenance());
        let insert_result = {
            let mut senders = seg_ack_senders.lock();
            if !senders.contains_key(&key) && senders.len() >= MAX_SEG_SENDERS {
                Err(())
            } else {
                Ok(senders.insert(key.clone(), Arc::clone(&send_handle)))
            }
        };
        let replaced = match insert_result {
            Ok(replaced) => replaced,
            Err(()) => {
                warn_throttled!(
                    seg_ack_senders.warning,
                    invoke_id,
                    limit = MAX_SEG_SENDERS,
                    "Too many active segmented ComplexAck senders, aborting"
                );
                let abort = Apdu::Abort(AbortPdu {
                    sent_by_server: true,
                    invoke_id,
                    abort_reason: AbortReason::BUFFER_OVERFLOW,
                });
                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &abort).expect("valid APDU encoding");
                let _ = Self::issue_terminal_response(
                    network,
                    &buf,
                    source_mac,
                    source_network,
                    route,
                    pending.take(),
                )
                .await;
                return;
            }
        };
        let _registration = segmented_send::SegmentedSendRegistration {
            registry: seg_ack_senders,
            key,
            sender: seg_ack_tx,
            _permit: _sender_permit,
        };
        if let Some(replaced) = replaced {
            warn_throttled!(
                seg_ack_senders.warning,
                invoke_id,
                "Replacing active segmented ComplexAck sender for peer/invoke-id"
            );
            replaced.send_control(SegmentedSendControlEvent::Cancel);
        }
        // From here a different request under this requester and invoke ID
        // ends the answer (#2770); every way out passes `finish` below.
        if let Some(pending) = &pending {
            pending.answering(&send_handle);
        }

        let seg_timeout = options.segment_timeout;
        let mut seg_idx: usize = 0;
        let mut retransmission_retries: u8 = 0;

        'send_segments: while seg_idx < total_segments {
            // Before every send, not only while waiting: a segment sent
            // again after a requester's Abort, or after a different request
            // took the final acknowledgment as lost (#2770), could reach a
            // new request under the same invoke ID.
            if send_closed(&send_handle, &mut control_rx, seg_ack_senders, invoke_id)
                || !send_handle.sending()
            {
                break;
            }
            let is_last = seg_idx == total_segments - 1;

            let pdu = Apdu::ComplexAck(ComplexAck {
                segmented: true,
                more_follows: !is_last,
                invoke_id,
                sequence_number: Some(seg_idx as u8),
                proposed_window_size: Some(1),
                service_choice,
                service_ack: segments[seg_idx].clone(),
            });

            let mut buf = BytesMut::with_capacity(client_max_apdu as usize);
            encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

            send_handle
                .current_sequence
                .store(seg_idx as u16, Ordering::Release);

            if let Err(e) = Self::send_confirmed_response_apdu_expecting_reply(
                network,
                &buf,
                source_mac,
                source_network,
                route,
                true,
            )
            .await
            {
                warn!(error = %e, seq = seg_idx, "Failed to send segment");
                break;
            }

            debug!(seq = seg_idx, is_last, "Sent ComplexAck segment");
            if is_last {
                send_handle.last_sent();
            }

            let ack_deadline = tokio::time::Instant::now() + seg_timeout;
            loop {
                if send_closed(&send_handle, &mut control_rx, seg_ack_senders, invoke_id) {
                    break 'send_segments;
                }

                let wait_result = tokio::select! {
                    biased;
                    changed = control_rx.changed() => {
                        if changed.is_err() {
                            SegmentedSendWaitResult::ChannelClosed
                        } else {
                            match control_rx.borrow_and_update().clone() {
                                Some(SegmentedSendControlEvent::Abort(abort)) => {
                                    SegmentedSendWaitResult::Abort(abort)
                                }
                                Some(SegmentedSendControlEvent::Cancel) => {
                                    SegmentedSendWaitResult::Cancel
                                }
                                // The check above logs it and stops.
                                Some(SegmentedSendControlEvent::Supersede) | None => continue,
                            }
                        }
                    }
                    ack = seg_ack_rx.recv() => {
                        match ack {
                            Some(ack) => SegmentedSendWaitResult::SegmentAck(ack),
                            None => SegmentedSendWaitResult::ChannelClosed,
                        }
                    }
                    _ = tokio::time::sleep_until(ack_deadline) => {
                        SegmentedSendWaitResult::Timeout
                    }
                };

                match wait_result {
                    SegmentedSendWaitResult::SegmentAck(ack) => {
                        if send_handle.closed.load(Ordering::Acquire) {
                            continue;
                        }

                        debug!(
                            seq = ack.sequence_number,
                            negative = ack.negative_ack,
                            sent_by_server = ack.sent_by_server,
                            "Received SegmentAck for ComplexAck"
                        );

                        if ack.sent_by_server {
                            warn_throttled!(
                                seg_ack_senders.warning,
                                seq = ack.sequence_number,
                                negative = ack.negative_ack,
                                "Ignoring SegmentAck with server bit set for server ComplexAck send"
                            );
                            continue;
                        }

                        let Some(disposition) =
                            segment_ack_disposition(&ack, seg_idx, total_segments)
                        else {
                            warn_throttled!(
                                seg_ack_senders.warning,
                                seq = ack.sequence_number,
                                negative = ack.negative_ack,
                                current = seg_idx,
                                total = total_segments,
                                "Ignoring SegmentAck outside current send window"
                            );
                            continue;
                        };

                        if disposition == SegmentAckDisposition::Retransmit {
                            if retransmission_retries >= options.max_retries {
                                warn_throttled!(
                                    seg_ack_senders.warning,
                                    invoke_id,
                                    retries = retransmission_retries,
                                    "Too many negative SegmentAck retries, aborting segmented send"
                                );
                                // Settled first, so this is the one Abort
                                // even if a different request has just
                                // ended the answer (#2770), when it is that
                                // request's; until it is issued, requests
                                // under the invoke ID are dropped.
                                let abort_reason = if send_handle.aborting() {
                                    AbortReason::INVALID_APDU_IN_THIS_STATE
                                } else {
                                    AbortReason::TSM_TIMEOUT
                                };
                                let abort = Apdu::Abort(AbortPdu {
                                    sent_by_server: true,
                                    invoke_id,
                                    abort_reason,
                                });
                                let mut abort_buf = BytesMut::new();
                                encode_apdu(&mut abort_buf, &abort).expect("valid APDU encoding");
                                let _ = Self::issue_terminal_response(
                                    network,
                                    &abort_buf,
                                    source_mac,
                                    source_network,
                                    route,
                                    pending.take(),
                                )
                                .await;
                                break 'send_segments;
                            }
                            retransmission_retries += 1;

                            debug!(
                                seq = ack.sequence_number,
                                current = seg_idx,
                                "Negative SegmentAck retransmitting current sequence"
                            );
                            continue 'send_segments;
                        }

                        retransmission_retries = 0;
                        seg_idx += 1;
                        continue 'send_segments;
                    }
                    SegmentedSendWaitResult::Abort(abort) => {
                        warn_throttled!(
                            seg_ack_senders.warning,
                            invoke_id = abort.invoke_id,
                            reason = ?abort.abort_reason,
                            "Client aborted segmented ComplexAck send"
                        );
                        break 'send_segments;
                    }
                    SegmentedSendWaitResult::Cancel => {
                        warn_throttled!(
                            seg_ack_senders.warning,
                            invoke_id,
                            "Segmented ComplexAck send cancelled by newer same-peer transaction"
                        );
                        break 'send_segments;
                    }
                    SegmentedSendWaitResult::ChannelClosed => {
                        warn!("Segmented send event channel closed during segmented send");
                        break 'send_segments;
                    }
                    SegmentedSendWaitResult::Timeout => {
                        if retransmission_retries < options.max_retries {
                            retransmission_retries += 1;
                            warn_throttled!(
                                seg_ack_senders.warning,
                                seq = seg_idx,
                                retries = retransmission_retries,
                                "Timeout waiting for SegmentAck, retransmitting segment"
                            );
                            continue 'send_segments;
                        }

                        warn_throttled!(
                            seg_ack_senders.warning,
                            seq = seg_idx,
                            retries = retransmission_retries,
                            "SegmentAck retry budget exhausted, ending segmented send"
                        );
                        break 'send_segments;
                    }
                }
            }
        }
        // A different request came under this requester and invoke ID
        // midway (Clause 5.4.5.4, #2770): its Abort goes out here, after the
        // last segment this task sent, so no segment follows it.
        if send_handle.finish() {
            let abort = Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: AbortReason::INVALID_APDU_IN_THIS_STATE,
            });
            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &abort).expect("valid APDU encoding");
            let _ = Self::issue_terminal_response(
                network,
                &buf,
                source_mac,
                source_network,
                route,
                pending.take(),
            )
            .await;
        }
    }
}
