use super::*;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Dispatch a received APDU to the appropriate handler.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn dispatch_apdu(
        tsm: &Arc<Mutex<Tsm>>,
        device_table: &Arc<Mutex<DeviceTable>>,
        network: &Arc<NetworkLayer<T>>,
        cov_tx: &broadcast::Sender<ReceivedCOVNotification>,
        confirmed_cov_ack_policy: &ConfirmedCOVNotificationAckPolicy,
        device_tx: &broadcast::Sender<DeviceEvent>,
        seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
        seg_ack_senders: &Arc<Mutex<HashMap<SegKey, mpsc::Sender<SegmentAckPdu>>>>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        apdu: Apdu,
        segmented_response_accepted: bool,
    ) {
        let tsm_mac = response_tsm_mac(source_mac, source_network);
        match apdu {
            Apdu::SimpleAck(ack) => {
                debug!(invoke_id = ack.invoke_id, "Received SimpleAck");
                let mut tsm = tsm.lock().await;
                tsm.complete_transaction(&tsm_mac, ack.invoke_id, TsmResponse::SimpleAck);
            }
            Apdu::ComplexAck(ack) => {
                if ack.segmented {
                    Self::handle_segmented_complex_ack(
                        tsm,
                        network,
                        seg_state,
                        source_mac,
                        source_network,
                        ack,
                        segmented_response_accepted,
                    )
                    .await;
                } else {
                    debug!(invoke_id = ack.invoke_id, "Received ComplexAck");
                    let mut tsm = tsm.lock().await;
                    tsm.complete_transaction(
                        &tsm_mac,
                        ack.invoke_id,
                        TsmResponse::ComplexAck {
                            service_data: ack.service_ack,
                        },
                    );
                }
            }
            Apdu::Error(err) => {
                debug!(invoke_id = err.invoke_id, "Received Error PDU");
                let mut tsm = tsm.lock().await;
                tsm.complete_transaction(
                    &tsm_mac,
                    err.invoke_id,
                    TsmResponse::Error {
                        class: err.error_class.to_raw() as u32,
                        code: err.error_code.to_raw() as u32,
                    },
                );
            }
            Apdu::Reject(rej) => {
                debug!(invoke_id = rej.invoke_id, "Received Reject PDU");
                let mut tsm = tsm.lock().await;
                tsm.complete_transaction(
                    &tsm_mac,
                    rej.invoke_id,
                    TsmResponse::Reject {
                        reason: rej.reject_reason.to_raw(),
                    },
                );
            }
            Apdu::Abort(abt) => {
                debug!(invoke_id = abt.invoke_id, "Received Abort PDU");
                let mut tsm = tsm.lock().await;
                tsm.complete_transaction(
                    &tsm_mac,
                    abt.invoke_id,
                    TsmResponse::Abort {
                        reason: abt.abort_reason.to_raw(),
                    },
                );
            }
            Apdu::ConfirmedRequest(req) => {
                if req.service_choice == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION {
                    match COVNotificationRequest::decode(&req.service_request) {
                        Ok(notification) => {
                            debug!(
                                object = ?notification.monitored_object_identifier,
                                "Received ConfirmedCOVNotification"
                            );
                            let received = ReceivedCOVNotification::new(
                                notification,
                                source_mac,
                                source_network,
                                COVNotificationDelivery::Confirmed,
                            );
                            let response = confirmed_cov_ack_policy(&received);
                            let _ = cov_tx.send(received);
                            Self::send_confirmed_cov_notification_response(
                                network,
                                source_mac,
                                source_network,
                                req.invoke_id,
                                req.service_choice,
                                response,
                            )
                            .await;
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to decode ConfirmedCOVNotification");
                        }
                    }
                } else {
                    debug!(
                        service = req.service_choice.to_raw(),
                        "Ignoring ConfirmedRequest (client mode)"
                    );
                }
            }
            Apdu::UnconfirmedRequest(req) => {
                if req.service_choice == UnconfirmedServiceChoice::I_AM {
                    match bacnet_services::who_is::IAmRequest::decode(&req.service_request) {
                        Ok(i_am) => {
                            debug!(
                                device = i_am.object_identifier.instance_number(),
                                vendor = i_am.vendor_id,
                                "Received IAm"
                            );
                            let (src_net, src_addr) = match source_network {
                                Some(npdu_addr) if !npdu_addr.mac_address.is_empty() => {
                                    (Some(npdu_addr.network), Some(npdu_addr.mac_address.clone()))
                                }
                                _ => (None, None),
                            };
                            let device = DiscoveredDevice {
                                object_identifier: i_am.object_identifier,
                                mac_address: MacAddr::from_slice(source_mac),
                                max_apdu_length: i_am.max_apdu_length,
                                segmentation_supported: i_am.segmentation_supported,
                                max_segments_accepted: None,
                                vendor_id: i_am.vendor_id,
                                last_seen: std::time::Instant::now(),
                                source_network: src_net,
                                source_address: src_addr,
                            };
                            let status =
                                device_table.lock().await.upsert_with_result(device.clone());
                            let kind = match status {
                                DeviceUpsertResult::Inserted => Some(DeviceEventKind::Discovered),
                                DeviceUpsertResult::Updated => Some(DeviceEventKind::Updated),
                                DeviceUpsertResult::Dropped => None,
                            };
                            if let Some(kind) = kind {
                                let _ = device_tx.send(DeviceEvent { kind, device });
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to decode IAm");
                        }
                    }
                } else if req.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION
                {
                    match COVNotificationRequest::decode(&req.service_request) {
                        Ok(notification) => {
                            debug!(
                                object = ?notification.monitored_object_identifier,
                                "Received UnconfirmedCOVNotification"
                            );
                            let received = ReceivedCOVNotification::new(
                                notification,
                                source_mac,
                                source_network,
                                COVNotificationDelivery::Unconfirmed,
                            );
                            let _ = cov_tx.send(received);
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to decode UnconfirmedCOVNotification");
                        }
                    }
                } else {
                    debug!(
                        service = req.service_choice.to_raw(),
                        "Ignoring unconfirmed service in client dispatch"
                    );
                }
            }
            Apdu::SegmentAck(sa) => {
                let key = (tsm_mac, sa.invoke_id);
                let senders = seg_ack_senders.lock().await;
                if let Some(tx) = senders.get(&key) {
                    let _ = tx.try_send(sa);
                } else {
                    debug!(
                        invoke_id = sa.invoke_id,
                        "Ignoring SegmentAck for unknown transaction"
                    );
                }
            }
        }
    }

    async fn send_confirmed_cov_notification_response(
        network: &Arc<NetworkLayer<T>>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        response: ConfirmedCOVNotificationResponse,
    ) {
        let apdu = match response {
            ConfirmedCOVNotificationResponse::Ack => Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice,
            }),
            ConfirmedCOVNotificationResponse::Reject(reject_reason) => Apdu::Reject(RejectPdu {
                invoke_id,
                reject_reason,
            }),
            ConfirmedCOVNotificationResponse::NoResponse => return,
        };

        let mut buf = BytesMut::with_capacity(4);
        if let Err(e) = encode_apdu(&mut buf, &apdu) {
            warn!(error = %e, "Failed to encode response for COV notification");
            return;
        }
        let send_result = match source_network {
            Some(address) if !address.mac_address.is_empty() => {
                network
                    .send_apdu_routed(
                        &buf,
                        address.network,
                        &address.mac_address,
                        source_mac,
                        false,
                        NetworkPriority::NORMAL,
                    )
                    .await
            }
            _ => {
                network
                    .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
                    .await
            }
        };
        if let Err(e) = send_result {
            warn!(error = %e, "Failed to send response for COV notification");
        }
    }
}
