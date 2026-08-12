use super::*;

/// Timestamp fields for COVNotificationMultiple: the request-level
/// `timestamp [3] BACnetTimeStamp` and each value's `timeOfChange [3]` both
/// carry a sequence-number form. Clause 21's `BACnetTimeStamp` production
/// constrains `sequence-number` to `Unsigned (0..65535)` and the shared
/// codec rejects anything larger on encode — wrap seconds-of-epoch into the
/// valid window ONCE so both fields agree (previously the request timestamp
/// was wrapped but `timeOfChange` encoded raw seconds, reintroducing the
/// non-conformant >65535 value the wrap exists to prevent).
pub(crate) fn cov_multiple_timestamps(total_secs: u64) -> (BACnetTimeStamp, Vec<u8>) {
    let seconds = total_secs % 65_536;
    let mut timestamp_choice = BytesMut::new();
    encode_ctx_unsigned(&mut timestamp_choice, 1, seconds);
    (
        BACnetTimeStamp::SequenceNumber(seconds),
        timestamp_choice.to_vec(),
    )
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub(super) async fn send_cov_apdu(
        network: &NetworkLayer<T>,
        apdu: &[u8],
        sub: &CovSubscription,
        expecting_reply: bool,
    ) -> Result<(), Error> {
        if let Some(ref destination) = sub.subscriber_network {
            network
                .send_apdu_routed(
                    apdu,
                    destination.network,
                    &destination.mac_address,
                    &sub.subscriber_mac,
                    expecting_reply,
                    NetworkPriority::NORMAL,
                )
                .await
        } else {
            network
                .send_apdu(
                    apdu,
                    &sub.subscriber_mac,
                    expecting_reply,
                    NetworkPriority::NORMAL,
                )
                .await
        }
    }

    fn cov_peer(sub: &CovSubscription) -> TsmPeer {
        (sub.subscriber_mac.clone(), sub.subscriber_network.clone())
    }

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
        let subs: Vec<CovSubscription> = {
            let mut table = cov_table.write().await;
            table.subscriptions_for(oid).into_iter().cloned().collect()
        };

        if subs.is_empty() {
            return;
        }

        let (single_subs, multiple_subs): (Vec<_>, Vec<_>) = subs
            .into_iter()
            .partition(|sub| sub.notification_kind == CovNotificationKind::Single);

        Self::fire_cov_notifications_for_subscriptions(
            db,
            network,
            cov_table,
            cov_in_flight,
            server_tsm,
            config,
            oid,
            &single_subs,
        )
        .await;

        Self::fire_cov_notification_multiple_for_subscriptions(
            db,
            network,
            cov_table,
            cov_in_flight,
            server_tsm,
            comm_state,
            config,
            Some(oid),
            &multiple_subs,
        )
        .await;
    }

    /// Fire the initial COV notification for a newly accepted subscription.
    /// Skipped when DCC is active (comm_state >= 1).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn fire_initial_cov_notification(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        subscription: &CovSubscription,
    ) {
        if comm_state.load(Ordering::Acquire) >= 1 {
            return;
        }

        Self::fire_cov_notifications_for_subscriptions(
            db,
            network,
            cov_table,
            cov_in_flight,
            server_tsm,
            config,
            &subscription.monitored_object_identifier,
            std::slice::from_ref(subscription),
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn fire_cov_notification_multiple_for_subscriptions(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        changed_oid: Option<&ObjectIdentifier>,
        subscriptions: &[CovSubscription],
    ) {
        if comm_state.load(Ordering::Acquire) >= 1 || subscriptions.is_empty() {
            return;
        }

        let mut grouped: HashMap<(TsmPeer, u32, bool), Vec<CovSubscription>> = HashMap::new();

        if let Some(oid) = changed_oid {
            let (current_pv, cov_increment) = {
                let db = db.read().await;
                let object = match db.get(oid) {
                    Some(object) => object,
                    None => return,
                };

                let current_pv = match object.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                {
                    Ok(PropertyValue::Real(value)) => Some(value),
                    _ => None,
                };

                (current_pv, object.cov_increment())
            };

            for sub in subscriptions {
                if CovSubscriptionTable::should_notify(
                    sub,
                    current_pv,
                    sub.cov_increment.or(cov_increment),
                ) {
                    grouped
                        .entry((
                            Self::cov_peer(sub),
                            sub.subscriber_process_identifier,
                            sub.issue_confirmed_notifications,
                        ))
                        .or_default()
                        .push(sub.clone());
                }
            }
        } else {
            for sub in subscriptions {
                grouped
                    .entry((
                        Self::cov_peer(sub),
                        sub.subscriber_process_identifier,
                        sub.issue_confirmed_notifications,
                    ))
                    .or_default()
                    .push(sub.clone());
            }
        }

        for subs in grouped.values() {
            Self::send_cov_notification_multiple(
                db,
                network,
                cov_table,
                cov_in_flight,
                server_tsm,
                config,
                subs,
            )
            .await;
        }
    }

    pub(super) fn encode_confirmed_cov_multiple_apdu(
        notification: &COVNotificationMultipleRequest,
        invoke_id: u8,
        max_apdu_length: u16,
    ) -> Result<BytesMut, Error> {
        let mut service_buf = BytesMut::new();
        notification.encode(&mut service_buf)?;

        let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
            service_request: service_buf.freeze(),
        });

        let mut buf = BytesMut::new();
        encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");
        Ok(buf)
    }

    pub(super) fn encode_unconfirmed_cov_multiple_apdu(
        notification: &COVNotificationMultipleRequest,
    ) -> Result<BytesMut, Error> {
        let mut service_buf = BytesMut::new();
        notification.encode(&mut service_buf)?;

        let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
            service_request: service_buf.freeze(),
        });

        let mut buf = BytesMut::new();
        encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");
        Ok(buf)
    }

    /// Fire the initial COVNotificationMultiple for a newly accepted
    /// SubscribeCOVPropertyMultiple request.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn fire_initial_cov_notification_multiple(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        subscriptions: &[CovSubscription],
    ) {
        Self::fire_cov_notification_multiple_for_subscriptions(
            db,
            network,
            cov_table,
            cov_in_flight,
            server_tsm,
            comm_state,
            config,
            None,
            subscriptions,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_cov_notification_multiple(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        config: &ServerConfig,
        subscriptions: &[CovSubscription],
    ) {
        if subscriptions.is_empty() {
            return;
        }

        let representative = &subscriptions[0];
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let total_secs = now.as_secs();
        let (timestamp, timestamp_choice_bytes) = cov_multiple_timestamps(total_secs);
        let mut timestamp_choice = BytesMut::new();
        timestamp_choice.extend_from_slice(&timestamp_choice_bytes);

        let (device_oid, items, last_notified) = {
            let db = db.read().await;
            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|o| o.object_type() == ObjectType::DEVICE)
                .unwrap_or_else(|| ObjectIdentifier::new(ObjectType::DEVICE, 0).unwrap());

            let mut items: Vec<COVNotificationItem> = Vec::new();
            let mut last_notified = Vec::new();

            for sub in subscriptions {
                let Some(property_identifier) = sub.monitored_property else {
                    continue;
                };
                let Some(object) = db.get(&sub.monitored_object_identifier) else {
                    continue;
                };

                let Ok(property_value) =
                    object.read_property(property_identifier, sub.monitored_property_array_index)
                else {
                    continue;
                };
                let mut value_buf = BytesMut::new();
                if encode_property_value(&mut value_buf, &property_value).is_err() {
                    continue;
                }

                if let Ok(PropertyValue::Real(pv)) =
                    object.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                {
                    last_notified.push((
                        sub.subscriber_mac.clone(),
                        sub.subscriber_network.clone(),
                        sub.subscriber_process_identifier,
                        sub.monitored_object_identifier,
                        sub.monitored_property,
                        pv,
                    ));
                }

                let value = COVNotificationValue {
                    property_identifier,
                    property_array_index: sub.monitored_property_array_index,
                    value: value_buf.to_vec(),
                    time_of_change: sub.timestamped.then(|| timestamp_choice.to_vec()),
                };

                if let Some(item) = items.iter_mut().find(|item| {
                    item.monitored_object_identifier == sub.monitored_object_identifier
                }) {
                    item.list_of_values.push(value);
                } else {
                    items.push(COVNotificationItem {
                        monitored_object_identifier: sub.monitored_object_identifier,
                        list_of_values: vec![value],
                    });
                }
            }

            (device_oid, items, last_notified)
        };

        if items.is_empty() {
            return;
        }

        let notification = COVNotificationMultipleRequest {
            subscriber_process_identifier: representative.subscriber_process_identifier,
            initiating_device_identifier: device_oid,
            time_remaining: 0,
            timestamp,
            list_of_cov_notifications: items,
        };

        if representative.issue_confirmed_notifications {
            let permit = match cov_in_flight.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    warn!("255 confirmed COV notifications in-flight, skipping COVNotificationMultiple");
                    return;
                }
            };

            let peer = Self::cov_peer(representative);
            let (id, result_rx) = match {
                let mut tsm = server_tsm.lock().await;
                tsm.allocate(peer.clone())
            } {
                Some(allocated) => allocated,
                None => {
                    warn!("No free invoke ID for confirmed COVNotificationMultiple");
                    return;
                }
            };

            let buf = match Self::encode_confirmed_cov_multiple_apdu(
                &notification,
                id,
                config.max_apdu_length as u16,
            ) {
                Ok(buf) => buf,
                Err(e) => {
                    warn!(error = %e, "Failed to encode confirmed COVNotificationMultiple");
                    return;
                }
            };

            {
                let mut table = cov_table.write().await;
                for (mac, network, process_id, object_id, property_id, pv) in &last_notified {
                    table.set_last_notified_value(
                        mac,
                        network.as_ref(),
                        *process_id,
                        *object_id,
                        *property_id,
                        *pv,
                    );
                }
            }

            let network = Arc::clone(network);
            let sub = representative.clone();
            let apdu_timeout = Duration::from_millis(config.cov_retry_timeout_ms);
            let tsm = Arc::clone(server_tsm);
            let apdu_retries = DEFAULT_APDU_RETRIES;
            tokio::spawn(async move {
                let _permit = permit;
                let mut pending_rx: Option<oneshot::Receiver<CovAckResult>> = Some(result_rx);

                for attempt in 0..=apdu_retries {
                    if let Err(e) = Self::send_cov_apdu(&network, &buf, &sub, true).await {
                        warn!(error = %e, attempt, "COVNotificationMultiple send failed");
                    } else {
                        debug!(
                            invoke_id = id,
                            attempt, "Confirmed COVNotificationMultiple sent"
                        );
                    }

                    let rx = pending_rx
                        .take()
                        .expect("receiver always set for each attempt");
                    let result = match tokio::time::timeout(apdu_timeout, rx).await {
                        Ok(Ok(r)) => Ok(r),
                        Ok(Err(_)) => Err(()),
                        Err(_) => Err(()),
                    };

                    if result.is_err() && attempt < apdu_retries {
                        let new_rx = {
                            let mut tsm = tsm.lock().await;
                            tsm.register(peer.clone(), id)
                        };
                        pending_rx = Some(new_rx);
                    }

                    match result {
                        Ok(CovAckResult::Ack) => {
                            debug!(invoke_id = id, "COVNotificationMultiple acknowledged");
                            return;
                        }
                        Ok(CovAckResult::Error) => {
                            warn!(
                                invoke_id = id,
                                "COVNotificationMultiple rejected by subscriber"
                            );
                            return;
                        }
                        Err(_) => {
                            if attempt < apdu_retries {
                                debug!(
                                    invoke_id = id,
                                    attempt, "COVNotificationMultiple timeout, retrying"
                                );
                            } else {
                                warn!(
                                    invoke_id = id,
                                    "COVNotificationMultiple failed after {} retries", apdu_retries
                                );
                            }
                        }
                    }
                }

                let mut tsm = tsm.lock().await;
                tsm.remove(&peer, id);
            });
        } else {
            let buf = match Self::encode_unconfirmed_cov_multiple_apdu(&notification) {
                Ok(buf) => buf,
                Err(e) => {
                    warn!(error = %e, "Failed to encode unconfirmed COVNotificationMultiple");
                    return;
                }
            };

            if let Err(e) = Self::send_cov_apdu(network, &buf, representative, false).await {
                warn!(error = %e, "Failed to send COVNotificationMultiple");
            } else {
                let mut table = cov_table.write().await;
                for (mac, network, process_id, object_id, property_id, pv) in &last_notified {
                    table.set_last_notified_value(
                        mac,
                        network.as_ref(),
                        *process_id,
                        *object_id,
                        *property_id,
                        *pv,
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn fire_cov_notifications_for_subscriptions(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        config: &ServerConfig,
        oid: &ObjectIdentifier,
        subs: &[CovSubscription],
    ) {
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

        for sub in subs {
            if !CovSubscriptionTable::should_notify(
                sub,
                current_pv,
                sub.cov_increment.or(cov_increment),
            ) {
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

                let peer = Self::cov_peer(sub);
                let (id, result_rx) = match {
                    let mut tsm = server_tsm.lock().await;
                    tsm.allocate(peer.clone())
                } {
                    Some(allocated) => allocated,
                    None => {
                        warn!(
                            object = ?oid,
                            "No free invoke ID for confirmed COV notification"
                        );
                        continue;
                    }
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
                        sub.subscriber_network.as_ref(),
                        sub.subscriber_process_identifier,
                        sub.monitored_object_identifier,
                        sub.monitored_property,
                        pv,
                    );
                }

                let network = Arc::clone(network);
                let sub = sub.clone();
                let apdu_timeout = Duration::from_millis(config.cov_retry_timeout_ms);
                let tsm = Arc::clone(server_tsm);
                let apdu_retries = DEFAULT_APDU_RETRIES;
                let peer = peer.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let mut pending_rx: Option<oneshot::Receiver<CovAckResult>> = Some(result_rx);

                    for attempt in 0..=apdu_retries {
                        if let Err(e) = Self::send_cov_apdu(&network, &buf, &sub, true).await {
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
                                tsm.register(peer.clone(), id)
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
                    tsm.remove(&peer, id);
                });
            } else {
                let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                    service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
                    service_request: service_buf.freeze(),
                });

                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                if let Err(e) = Self::send_cov_apdu(network, &buf, sub, false).await {
                    warn!(error = %e, "Failed to send COV notification");
                } else if let Some(pv) = current_pv {
                    let mut table = cov_table.write().await;
                    table.set_last_notified_value(
                        &sub.subscriber_mac,
                        sub.subscriber_network.as_ref(),
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
