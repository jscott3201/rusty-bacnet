use super::*;

#[derive(Debug)]
enum SegmentedSendWaitResult {
    SegmentAck(SegmentAckPdu),
    Abort(AbortPdu),
    Cancel,
    ChannelClosed,
    Timeout,
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Send a ComplexAck response using segmented transfer.
    ///
    /// Splits the service ack data into segments that fit within the client's
    /// max APDU length, sends each segment, and waits for SegmentAck from
    /// the client before sending the next (window size 1).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn send_segmented_complex_ack(
        network: &Arc<NetworkLayer<T>>,
        seg_ack_senders: &Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
        seg_send_permits: &Arc<Semaphore>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        service_ack_data: &[u8],
        client_max_apdu: u16,
        client_max_segments: Option<u8>,
    ) {
        Self::send_segmented_complex_ack_with_options(
            network,
            seg_ack_senders,
            seg_send_permits,
            source_mac,
            source_network,
            invoke_id,
            service_choice,
            service_ack_data,
            client_max_apdu,
            client_max_segments,
            SegmentedSendOptions::default(),
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn send_segmented_complex_ack_with_options(
        network: &Arc<NetworkLayer<T>>,
        seg_ack_senders: &Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
        seg_send_permits: &Arc<Semaphore>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        service_ack_data: &[u8],
        client_max_apdu: u16,
        client_max_segments: Option<u8>,
        options: SegmentedSendOptions,
    ) {
        let max_seg_size = max_segment_payload(client_max_apdu, SegmentedPduType::ComplexAck);
        let segments = match split_payload(service_ack_data, max_seg_size) {
            Ok(segments) => segments,
            Err(e) => {
                warn!(error = %e, "Response requires too many segments, aborting");
                let abort = Apdu::Abort(AbortPdu {
                    sent_by_server: true,
                    invoke_id,
                    abort_reason: AbortReason::BUFFER_OVERFLOW,
                });
                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &abort).expect("valid APDU encoding");
                let _ =
                    Self::send_confirmed_response_apdu(network, &buf, source_mac, source_network)
                        .await;
                return;
            }
        };
        let total_segments = segments.len();

        if let Some(max_segments) = client_max_segments {
            if total_segments > max_segments as usize {
                warn!(
                    total_segments,
                    max_segments, "Response exceeds client's max-segments-accepted, aborting"
                );
                let abort = Apdu::Abort(AbortPdu {
                    sent_by_server: true,
                    invoke_id,
                    abort_reason: AbortReason::BUFFER_OVERFLOW,
                });
                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &abort).expect("valid APDU encoding");
                let _ =
                    Self::send_confirmed_response_apdu(network, &buf, source_mac, source_network)
                        .await;
                return;
            }
        }

        if total_segments > 256 {
            warn!(
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
            let _ =
                Self::send_confirmed_response_apdu(network, &buf, source_mac, source_network).await;
            return;
        }

        let _sender_permit = match Arc::clone(seg_send_permits).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                warn!(
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
                let _ =
                    Self::send_confirmed_response_apdu(network, &buf, source_mac, source_network)
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
        ));
        let key = segmented_transaction_key(source_mac, source_network, invoke_id);
        let insert_result = {
            let mut senders = seg_ack_senders.lock().await;
            if !senders.contains_key(&key) && senders.len() >= MAX_SEG_SENDERS {
                Err(())
            } else {
                Ok(senders.insert(key.clone(), Arc::clone(&send_handle)))
            }
        };
        let replaced = match insert_result {
            Ok(replaced) => replaced,
            Err(()) => {
                warn!(
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
                let _ =
                    Self::send_confirmed_response_apdu(network, &buf, source_mac, source_network)
                        .await;
                return;
            }
        };
        if let Some(replaced) = replaced {
            warn!(
                invoke_id,
                "Replacing active segmented ComplexAck sender for peer/invoke-id"
            );
            replaced.send_control(SegmentedSendControlEvent::Cancel);
        }

        let seg_timeout = options.segment_timeout;
        let mut seg_idx: usize = 0;
        let mut retransmission_retries: u8 = 0;

        'send_segments: while seg_idx < total_segments {
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
                true,
            )
            .await
            {
                warn!(error = %e, seq = seg_idx, "Failed to send segment");
                break;
            }

            debug!(seq = seg_idx, is_last, "Sent ComplexAck segment");

            let ack_deadline = tokio::time::Instant::now() + seg_timeout;
            loop {
                if send_handle.closed.load(Ordering::Acquire) {
                    match control_rx.borrow_and_update().clone() {
                        Some(SegmentedSendControlEvent::Abort(abort)) => {
                            warn!(
                                invoke_id = abort.invoke_id,
                                reason = ?abort.abort_reason,
                                "Client aborted segmented ComplexAck send"
                            );
                        }
                        Some(SegmentedSendControlEvent::Cancel) | None => {
                            warn!(
                                invoke_id,
                                "Segmented ComplexAck send cancelled by newer same-peer transaction"
                            );
                        }
                    }
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
                                None => continue,
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
                            tracing::warn!(
                                seq = ack.sequence_number,
                                negative = ack.negative_ack,
                                "Ignoring SegmentAck with server bit set for server ComplexAck send"
                            );
                            continue;
                        }

                        let Some(disposition) =
                            segment_ack_disposition(&ack, seg_idx, total_segments)
                        else {
                            tracing::warn!(
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
                                warn!(
                                    invoke_id,
                                    retries = retransmission_retries,
                                    "Too many negative SegmentAck retries, aborting segmented send"
                                );
                                let abort = Apdu::Abort(AbortPdu {
                                    sent_by_server: true,
                                    invoke_id,
                                    abort_reason: AbortReason::TSM_TIMEOUT,
                                });
                                let mut abort_buf = BytesMut::new();
                                encode_apdu(&mut abort_buf, &abort).expect("valid APDU encoding");
                                let _ = Self::send_confirmed_response_apdu(
                                    network,
                                    &abort_buf,
                                    source_mac,
                                    source_network,
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
                        warn!(
                            invoke_id = abort.invoke_id,
                            reason = ?abort.abort_reason,
                            "Client aborted segmented ComplexAck send"
                        );
                        break 'send_segments;
                    }
                    SegmentedSendWaitResult::Cancel => {
                        warn!(
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
                            warn!(
                                seq = seg_idx,
                                retries = retransmission_retries,
                                "Timeout waiting for SegmentAck, retransmitting segment"
                            );
                            continue 'send_segments;
                        }

                        warn!(
                            seq = seg_idx,
                            retries = retransmission_retries,
                            "SegmentAck retry budget exhausted, ending segmented send"
                        );
                        break 'send_segments;
                    }
                }
            }
        }

        {
            let mut senders = seg_ack_senders.lock().await;
            if senders
                .get(&key)
                .is_some_and(|sender| sender.same_channel(&seg_ack_tx))
            {
                senders.remove(&key);
            }
        }
    }
}
