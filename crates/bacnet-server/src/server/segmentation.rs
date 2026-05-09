use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Send a ComplexAck response using segmented transfer.
    ///
    /// Splits the service ack data into segments that fit within the client's
    /// max APDU length, sends each segment, and waits for SegmentAck from
    /// the client before sending the next (window size 1).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn send_segmented_complex_ack(
        network: &Arc<NetworkLayer<T>>,
        seg_ack_senders: &Arc<Mutex<HashMap<SegKey, mpsc::Sender<SegmentAckPdu>>>>,
        source_mac: &[u8],
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        service_ack_data: &[u8],
        client_max_apdu: u16,
        client_max_segments: Option<u8>,
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
                let _ = network
                    .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
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
                let _ = network
                    .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
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
            let _ = network
                .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
                .await;
            return;
        }

        debug!(
            total_segments,
            max_seg_size,
            payload_len = service_ack_data.len(),
            "Starting segmented ComplexAck send"
        );

        let (seg_ack_tx, mut seg_ack_rx) = mpsc::channel(16);
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        {
            seg_ack_senders.lock().await.insert(key.clone(), seg_ack_tx);
        }

        let seg_timeout = Duration::from_secs(5);
        let mut seg_idx: usize = 0;
        let mut neg_ack_retries: u8 = 0;

        while seg_idx < total_segments {
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

            if let Err(e) = network
                .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
                .await
            {
                warn!(error = %e, seq = seg_idx, "Failed to send segment");
                break;
            }

            debug!(seq = seg_idx, is_last, "Sent ComplexAck segment");

            if !is_last {
                match tokio::time::timeout(seg_timeout, seg_ack_rx.recv()).await {
                    Ok(Some(ack)) => {
                        debug!(
                            seq = ack.sequence_number,
                            negative = ack.negative_ack,
                            "Received SegmentAck for ComplexAck"
                        );
                        if ack.negative_ack {
                            neg_ack_retries += 1;
                            if neg_ack_retries > MAX_NEG_SEGMENT_ACK_RETRIES {
                                warn!(
                                    invoke_id,
                                    retries = neg_ack_retries,
                                    "Too many negative SegmentAck retries, aborting segmented send"
                                );
                                let abort = Apdu::Abort(AbortPdu {
                                    sent_by_server: true,
                                    invoke_id,
                                    abort_reason: AbortReason::TSM_TIMEOUT,
                                });
                                let mut abort_buf = BytesMut::new();
                                encode_apdu(&mut abort_buf, &abort).expect("valid APDU encoding");
                                let _ = network
                                    .send_apdu(
                                        &abort_buf,
                                        source_mac,
                                        false,
                                        NetworkPriority::NORMAL,
                                    )
                                    .await;
                                break;
                            }
                            let acknowledged = ack.sequence_number as usize;
                            let requested = acknowledged + 1;
                            if requested > total_segments {
                                tracing::warn!(
                                    seq = acknowledged,
                                    total = total_segments,
                                    "negative SegmentAck requests out-of-range sequence, aborting"
                                );
                                break;
                            }
                            debug!(
                                seq = ack.sequence_number,
                                "Negative SegmentAck — retransmitting from requested sequence"
                            );
                            seg_idx = requested;
                            continue;
                        }
                    }
                    Ok(None) => {
                        warn!("SegmentAck channel closed during segmented send");
                        break;
                    }
                    Err(_) => {
                        warn!(
                            seq = seg_idx,
                            "Timeout waiting for SegmentAck, aborting segmented send"
                        );
                        let abort = Apdu::Abort(AbortPdu {
                            sent_by_server: true,
                            invoke_id,
                            abort_reason: AbortReason::TSM_TIMEOUT,
                        });
                        let mut abort_buf = BytesMut::new();
                        encode_apdu(&mut abort_buf, &abort).expect("valid APDU encoding");
                        let _ = network
                            .send_apdu(&abort_buf, source_mac, false, NetworkPriority::NORMAL)
                            .await;
                        break;
                    }
                }
            }

            seg_idx += 1;
        }

        match tokio::time::timeout(seg_timeout, seg_ack_rx.recv()).await {
            Ok(Some(_ack)) => {
                debug!("Received final SegmentAck for ComplexAck");
            }
            _ => {
                warn!("No final SegmentAck received for ComplexAck");
            }
        }

        seg_ack_senders.lock().await.remove(&key);
    }
}
