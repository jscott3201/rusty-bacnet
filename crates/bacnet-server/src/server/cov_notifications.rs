use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Fire COV notifications for all active subscriptions on the given object.
    /// Skipped when DCC is active (comm_state >= 1).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn fire_cov_notifications(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        oid: &ObjectIdentifier,
    ) {
        if comm_state.load(Ordering::Acquire) >= 1 {
            return;
        }
        let subs: Vec<crate::cov::CovSubscription> = {
            let mut table = cov_table.write().await;
            table.subscriptions_for(oid).into_iter().cloned().collect()
        };

        if subs.is_empty() {
            return;
        }

        let (device_oid, values, current_pv, cov_increment) = {
            let db = db.read().await;
            let object = match db.get(oid) {
                Some(o) => o,
                None => return,
            };

            let cov_increment = object.cov_increment();

            let mut current_pv: Option<f32> = None;
            let mut values = Vec::new();
            if let Ok(pv) = object.read_property(PropertyIdentifier::PRESENT_VALUE, None) {
                if let PropertyValue::Real(v) = &pv {
                    current_pv = Some(*v);
                }
                let mut buf = BytesMut::new();
                if encode_property_value(&mut buf, &pv).is_ok() {
                    values.push(BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        value: buf.to_vec(),
                        priority: None,
                    });
                }
            }
            if let Ok(sf) = object.read_property(PropertyIdentifier::STATUS_FLAGS, None) {
                let mut buf = BytesMut::new();
                if encode_property_value(&mut buf, &sf).is_ok() {
                    values.push(BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::STATUS_FLAGS,
                        property_array_index: None,
                        value: buf.to_vec(),
                        priority: None,
                    });
                }
            }

            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|o| o.object_type() == ObjectType::DEVICE)
                .unwrap_or_else(|| ObjectIdentifier::new(ObjectType::DEVICE, 0).unwrap());

            (device_oid, values, current_pv, cov_increment)
        };

        if values.is_empty() {
            return;
        }

        for sub in &subs {
            if !CovSubscriptionTable::should_notify(sub, current_pv, cov_increment) {
                continue;
            }
            let time_remaining = sub.expires_at.map_or(0, |exp| {
                exp.saturating_duration_since(Instant::now()).as_secs() as u32
            });

            let notification_values = if let Some(prop) = sub.monitored_property {
                let db = db.read().await;
                if let Some(object) = db.get(oid) {
                    if let Ok(pv) = object.read_property(prop, sub.monitored_property_array_index) {
                        let mut buf = BytesMut::new();
                        if encode_property_value(&mut buf, &pv).is_ok() {
                            vec![BACnetPropertyValue {
                                property_identifier: prop,
                                property_array_index: sub.monitored_property_array_index,
                                value: buf.to_vec(),
                                priority: None,
                            }]
                        } else {
                            values.clone()
                        }
                    } else {
                        values.clone()
                    }
                } else {
                    values.clone()
                }
            } else {
                values.clone()
            };

            let notification = COVNotificationRequest {
                subscriber_process_identifier: sub.subscriber_process_identifier,
                initiating_device_identifier: device_oid,
                monitored_object_identifier: *oid,
                time_remaining,
                list_of_values: notification_values,
            };

            let mut service_buf = BytesMut::new();
            notification.encode(&mut service_buf);

            if sub.issue_confirmed_notifications {
                let permit = match cov_in_flight.clone().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        warn!(
                            object = ?oid,
                            "255 confirmed COV notifications in-flight, skipping notification"
                        );
                        continue;
                    }
                };

                let (id, result_rx) = {
                    let mut tsm = server_tsm.lock().await;
                    tsm.allocate(sub.subscriber_mac.clone())
                };

                let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                    segmented: false,
                    more_follows: false,
                    segmented_response_accepted: false,
                    max_segments: None,
                    max_apdu_length: config.max_apdu_length as u16,
                    invoke_id: id,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                    service_request: service_buf.freeze(),
                });

                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                if let Some(pv) = current_pv {
                    let mut table = cov_table.write().await;
                    table.set_last_notified_value(
                        &sub.subscriber_mac,
                        sub.subscriber_process_identifier,
                        sub.monitored_object_identifier,
                        sub.monitored_property,
                        pv,
                    );
                }

                let network = Arc::clone(network);
                let mac = sub.subscriber_mac.clone();
                let apdu_timeout = Duration::from_millis(config.cov_retry_timeout_ms);
                let tsm = Arc::clone(server_tsm);
                let apdu_retries = DEFAULT_APDU_RETRIES;
                tokio::spawn(async move {
                    let _permit = permit;
                    let mut pending_rx: Option<oneshot::Receiver<CovAckResult>> = Some(result_rx);

                    for attempt in 0..=apdu_retries {
                        if let Err(e) = network
                            .send_apdu(&buf, &mac, true, NetworkPriority::NORMAL)
                            .await
                        {
                            warn!(error = %e, attempt, "COV notification send failed");
                        } else {
                            debug!(invoke_id = id, attempt, "Confirmed COV notification sent");
                        }

                        let rx = pending_rx
                            .take()
                            .expect("receiver always set for each attempt");
                        let result = match tokio::time::timeout(apdu_timeout, rx).await {
                            Ok(Ok(r)) => Ok(r),
                            Ok(Err(_)) => Err(()), // channel closed
                            Err(_) => Err(()),     // timeout
                        };

                        if result.is_err() && attempt < apdu_retries {
                            let new_rx = {
                                let mut tsm = tsm.lock().await;
                                tsm.register(mac.clone(), id)
                            };
                            pending_rx = Some(new_rx);
                        }

                        match result {
                            Ok(CovAckResult::Ack) => {
                                debug!(invoke_id = id, "COV notification acknowledged");
                                return;
                            }
                            Ok(CovAckResult::Error) => {
                                warn!(invoke_id = id, "COV notification rejected by subscriber");
                                return;
                            }
                            Err(_) => {
                                if attempt < apdu_retries {
                                    debug!(
                                        invoke_id = id,
                                        attempt, "COV notification timeout, retrying"
                                    );
                                } else {
                                    warn!(
                                        invoke_id = id,
                                        "COV notification failed after {} retries", apdu_retries
                                    );
                                }
                            }
                        }
                    }

                    let mut tsm = tsm.lock().await;
                    tsm.remove(&mac, id);
                });
            } else {
                let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                    service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
                    service_request: service_buf.freeze(),
                });

                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                if let Err(e) = network
                    .send_apdu(&buf, &sub.subscriber_mac, false, NetworkPriority::NORMAL)
                    .await
                {
                    warn!(error = %e, "Failed to send COV notification");
                } else if let Some(pv) = current_pv {
                    let mut table = cov_table.write().await;
                    table.set_last_notified_value(
                        &sub.subscriber_mac,
                        sub.subscriber_process_identifier,
                        sub.monitored_object_identifier,
                        sub.monitored_property,
                        pv,
                    );
                }
            }
        }
    }
}
