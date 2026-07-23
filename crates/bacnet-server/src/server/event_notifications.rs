use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Evaluate intrinsic reporting on an object and send event notifications
    /// to NotificationClass recipients (or broadcast if none configured).
    /// Skipped when DCC is active (comm_state >= 1).
    ///
    /// This is the per-write entry point: it probes the detector, which fires
    /// immediately only when `Time_Delay == 0`. For a nonzero delay the probe
    /// seeds a pending transition (returning `None`, so no notification is
    /// sent here) and the one-second [`intrinsic_reporting_task`](Self::start)
    /// advances the countdown and sends the notification on expiry.
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

        let change = {
            let mut db = db.write().await;
            match db.get_mut(oid) {
                Some(object) => object.evaluate_intrinsic_reporting(),
                None => return,
            }
        };

        if let Some(change) = change {
            Self::build_and_send_event_notification(
                db,
                network,
                comm_state,
                server_tsm,
                oid,
                change,
                retry_timeout_ms,
            )
            .await;
        }
    }

    /// Build an `EventNotificationRequest` for a pre-computed transition and
    /// send it to NotificationClass recipients (or broadcast if none).
    ///
    /// Shared by the per-write path ([`fire_event_notifications`]) and the
    /// periodic `Time_Delay` confirmation path, so both emit identical
    /// notifications. Skipped when DCC is active (comm_state >= 1). Re-reads
    /// `Notification_Class` / `Notify_Type` under a brief `db.write()` guard,
    /// then drops the lock before any network send.
    pub(super) async fn build_and_send_event_notification(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        comm_state: &Arc<AtomicU8>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        oid: &ObjectIdentifier,
        change: EventStateChange,
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
                    let peer_key = (target_mac.clone().unwrap_or_else(MacAddr::new), None);
                    let (id, result_rx) = match {
                        let mut tsm = server_tsm.lock().await;
                        tsm.allocate(peer_key.clone())
                    } {
                        Some(allocated) => allocated,
                        None => {
                            warn!("No free invoke ID for confirmed EventNotification");
                            continue;
                        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_objects::analog::AnalogInputObject;
    use bacnet_objects::device::{DeviceConfig, DeviceObject};
    use bacnet_objects::event::EventStateChange;
    use bacnet_types::enums::EventState;

    /// A DCC-disabled server (comm_state >= 1) suppresses the periodic event
    /// send: `build_and_send_event_notification` returns without sending,
    /// matching the per-write path's DCC gate. Verified against a recording
    /// transport that would otherwise capture the broadcast APDU.
    #[tokio::test]
    async fn dcc_suppresses_periodic_event_send() {
        use bacnet_transport::port::TransportPort;
        use bytes::Bytes;
        use std::sync::{Arc as StdArc, Mutex as StdMutex};
        use tokio::sync::mpsc;

        #[derive(Clone, Default)]
        struct RecordingTransport {
            sent_broadcast: StdArc<StdMutex<Vec<Bytes>>>,
            local_mac: Vec<u8>,
        }
        impl TransportPort for RecordingTransport {
            async fn start(
                &mut self,
            ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
                let (_tx, rx) = mpsc::channel(1);
                Ok(rx)
            }
            async fn stop(&mut self) -> Result<(), Error> {
                Ok(())
            }
            async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
                Ok(())
            }
            async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
                self.sent_broadcast
                    .lock()
                    .unwrap()
                    .push(Bytes::copy_from_slice(npdu));
                Ok(())
            }
            fn local_mac(&self) -> &[u8] {
                &self.local_mac
            }
        }

        let sent = StdArc::new(StdMutex::new(Vec::new()));
        let transport = RecordingTransport {
            sent_broadcast: StdArc::clone(&sent),
            local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
        };
        let network = Arc::new(NetworkLayer::new(transport));
        let comm_state = Arc::new(AtomicU8::new(1)); // DCC disabled
        let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));

        let mut db = ObjectDatabase::new();
        db.add(Box::new(AnalogInputObject::new(1, "AI-1", 0).unwrap()))
            .unwrap();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                instance: 1,
                name: "Dev".into(),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
        let db = Arc::new(tokio::sync::RwLock::new(db));
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        let change = EventStateChange {
            from: EventState::NORMAL,
            to: EventState::HIGH_LIMIT,
        };
        BACnetServer::<RecordingTransport>::build_and_send_event_notification(
            &db,
            &network,
            &comm_state,
            &server_tsm,
            &oid,
            change,
            1000,
        )
        .await;

        assert!(
            sent.lock().unwrap().is_empty(),
            "DCC-disabled server must not send event notifications"
        );
    }
}
