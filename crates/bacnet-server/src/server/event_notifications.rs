use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Evaluate intrinsic reporting on an object and send event notifications
    /// to NotificationClass recipients (or broadcast if none configured).
    /// Skipped when DCC is active (comm_state >= 1).
    pub(super) async fn fire_event_notifications(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        comm_state: &Arc<AtomicU8>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        oid: &ObjectIdentifier,
        retry_timeout_ms: u64,
    ) {
        if comm_state.load(Ordering::Acquire) >= 1 {
            return;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let total_secs = now.as_secs();
        let dow = ((total_secs / 86400 + 3) % 7) as u8;
        let today_bit = 1u8 << dow;
        let day_secs = (total_secs % 86400) as u32;
        let current_time = Time {
            hour: (day_secs / 3600) as u8,
            minute: ((day_secs % 3600) / 60) as u8,
            second: (day_secs % 60) as u8,
            hundredths: (now.subsec_millis() / 10) as u8,
        };

        let (notification, recipients) = {
            let mut db = db.write().await;

            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|o| o.object_type() == ObjectType::DEVICE)
                .unwrap_or_else(|| ObjectIdentifier::new(ObjectType::DEVICE, 0).unwrap());

            let object = match db.get_mut(oid) {
                Some(o) => o,
                None => return,
            };

            let change = match object.evaluate_intrinsic_reporting() {
                Some(c) => c,
                None => return,
            };

            let notification_class = object
                .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
                .ok()
                .and_then(|v| match v {
                    PropertyValue::Unsigned(n) => Some(n as u32),
                    _ => None,
                })
                .unwrap_or(0);

            let notify_type = object
                .read_property(PropertyIdentifier::NOTIFY_TYPE, None)
                .ok()
                .and_then(|v| match v {
                    PropertyValue::Enumerated(n) => Some(n),
                    _ => None,
                })
                .unwrap_or(NotifyType::ALARM.to_raw());

            let priority = if change.to == bacnet_types::enums::EventState::NORMAL {
                200u8
            } else {
                100u8
            };

            let transition = change.transition();

            let base_notification = EventNotificationRequest {
                process_identifier: 0,
                initiating_device_identifier: device_oid,
                event_object_identifier: *oid,
                timestamp: BACnetTimeStamp::SequenceNumber(total_secs),
                notification_class,
                priority,
                event_type: change.event_type().to_raw(),
                message_text: None,
                notify_type,
                ack_required: notify_type == NotifyType::ALARM.to_raw(),
                from_state: change.from.to_raw(),
                to_state: change.to.to_raw(),
                event_values: None,
            };

            let recipients = get_notification_recipients(
                &db,
                notification_class,
                transition,
                today_bit,
                &current_time,
            );

            (base_notification, recipients)
        };

        if recipients.is_empty() {
            let mut service_buf = BytesMut::new();
            if let Err(e) = notification.encode(&mut service_buf) {
                warn!(error = %e, "Failed to encode EventNotification");
                return;
            }

            let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                service_choice: UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION,
                service_request: service_buf.freeze(),
            });

            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

            if let Err(e) = network
                .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                .await
            {
                warn!(error = %e, "Failed to broadcast EventNotification");
            }
        } else {
            for (recipient, process_id, confirmed) in &recipients {
                let mut targeted = notification.clone();
                targeted.process_identifier = *process_id;

                let mut service_buf = BytesMut::new();
                if let Err(e) = targeted.encode(&mut service_buf) {
                    warn!(error = %e, "Failed to encode EventNotification");
                    continue;
                }

                let service_bytes = service_buf.freeze();

                if *confirmed {
                    let target_mac = match recipient {
                        bacnet_types::constructed::BACnetRecipient::Address(addr) => {
                            Some(addr.mac_address.clone())
                        }
                        bacnet_types::constructed::BACnetRecipient::Device(_) => None,
                    };
                    let peer_key = target_mac.clone().unwrap_or_else(MacAddr::new);
                    let (id, result_rx) = {
                        let mut tsm = server_tsm.lock().await;
                        tsm.allocate(peer_key.clone())
                    };

                    let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                        segmented: false,
                        more_follows: false,
                        segmented_response_accepted: false,
                        max_segments: None,
                        max_apdu_length: 1476,
                        invoke_id: id,
                        sequence_number: None,
                        proposed_window_size: None,
                        service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
                        service_request: service_bytes,
                    });

                    let mut buf = BytesMut::new();
                    encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                    let network = Arc::clone(network);
                    let tsm = Arc::clone(server_tsm);
                    let timeout = Duration::from_millis(retry_timeout_ms);
                    let apdu_retries = DEFAULT_APDU_RETRIES;
                    tokio::spawn(async move {
                        let mut pending_rx: Option<oneshot::Receiver<CovAckResult>> =
                            Some(result_rx);

                        for attempt in 0..=apdu_retries {
                            let send_result = if let Some(mac) = target_mac.as_ref() {
                                network
                                    .send_apdu(&buf, mac, true, NetworkPriority::NORMAL)
                                    .await
                            } else {
                                network
                                    .broadcast_apdu(&buf, true, NetworkPriority::NORMAL)
                                    .await
                            };

                            if let Err(e) = send_result {
                                warn!(error = %e, attempt, "Confirmed EventNotification send failed");
                            } else {
                                debug!(invoke_id = id, attempt, "Confirmed EventNotification sent");
                            }

                            let rx = pending_rx
                                .take()
                                .expect("receiver always set for each attempt");
                            let result = match tokio::time::timeout(timeout, rx).await {
                                Ok(Ok(r)) => Ok(r),
                                Ok(Err(_)) => Err(()),
                                Err(_) => Err(()),
                            };

                            if result.is_err() && attempt < apdu_retries {
                                let new_rx = {
                                    let mut tsm = tsm.lock().await;
                                    tsm.register(peer_key.clone(), id)
                                };
                                pending_rx = Some(new_rx);
                            }

                            match result {
                                Ok(CovAckResult::Ack) => {
                                    debug!(invoke_id = id, "EventNotification acknowledged");
                                    return;
                                }
                                Ok(CovAckResult::Error) => {
                                    warn!(
                                        invoke_id = id,
                                        "EventNotification rejected by recipient"
                                    );
                                    return;
                                }
                                Err(_) => {
                                    if attempt < apdu_retries {
                                        debug!(
                                            invoke_id = id,
                                            attempt, "EventNotification timeout, retrying"
                                        );
                                    } else {
                                        warn!(
                                            invoke_id = id,
                                            "EventNotification failed after {} retries",
                                            apdu_retries
                                        );
                                    }
                                }
                            }
                        }

                        let mut tsm = tsm.lock().await;
                        tsm.remove(&peer_key, id);
                    });
                } else {
                    let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                        service_choice: UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION,
                        service_request: service_bytes,
                    });

                    let mut buf = BytesMut::new();
                    encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                    match recipient {
                        bacnet_types::constructed::BACnetRecipient::Address(addr) => {
                            if let Err(e) = network
                                .send_apdu(&buf, &addr.mac_address, false, NetworkPriority::NORMAL)
                                .await
                            {
                                warn!(
                                    error = %e,
                                    "Failed to send unconfirmed EventNotification"
                                );
                            }
                        }
                        bacnet_types::constructed::BACnetRecipient::Device(_) => {
                            if let Err(e) = network
                                .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                                .await
                            {
                                warn!(
                                    error = %e,
                                    "Failed to broadcast unconfirmed EventNotification"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
