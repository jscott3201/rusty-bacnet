use super::cov_clock::{cov_multiple_datetime, cov_multiple_time_remaining};
use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Fire the initial COVNotificationMultiple for a newly accepted
    /// SubscribeCOVPropertyMultiple request.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::server) async fn fire_initial_cov_notification_multiple(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        notification_transactions: &Arc<NotificationTransactions>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        subscriptions: &[CovSubscription],
    ) {
        let (counters, in_flight_tracker) = {
            let table = cov_table.read().await;
            (
                Arc::clone(table.counters()),
                Arc::clone(table.in_flight_tracker()),
            )
        };
        let mut budget = EventBudget::new(&config.cov_policy);
        Self::fire_cov_notification_multiple_for_subscriptions(
            db,
            network,
            cov_table,
            cov_in_flight,
            &in_flight_tracker,
            &counters,
            notification_transactions,
            comm_state,
            config,
            None,
            subscriptions,
            None,
            &mut budget,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::server) async fn fire_cov_notification_multiple_for_subscriptions(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        in_flight_tracker: &Arc<CovInFlightTracker>,
        counters: &Arc<AtomicCovCounters>,
        notification_transactions: &Arc<NotificationTransactions>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        changed_oid: Option<&ObjectIdentifier>,
        subscriptions: &[CovSubscription],
        snapshot: Option<&dyn bacnet_objects::traits::BACnetObject>,
        budget: &mut EventBudget,
    ) {
        if comm_state.load(Ordering::Acquire) >= 1 || subscriptions.is_empty() {
            return;
        }

        let mut grouped: HashMap<(TsmPeer, u32, bool), Vec<CovSubscription>> = HashMap::new();

        if let Some(oid) = changed_oid {
            let (current_pv, cov_increment) = {
                let db = db.read().await;
                let object = match snapshot.or_else(|| db.get(oid)) {
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
                in_flight_tracker,
                counters,
                notification_transactions,
                config,
                subs,
                snapshot,
                budget,
            )
            .await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_cov_notification_multiple(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        in_flight_tracker: &Arc<CovInFlightTracker>,
        counters: &Arc<AtomicCovCounters>,
        notification_transactions: &Arc<NotificationTransactions>,
        config: &ServerConfig,
        subscriptions: &[CovSubscription],
        snapshot: Option<&dyn bacnet_objects::traits::BACnetObject>,
        budget: &mut EventBudget,
    ) {
        if subscriptions.is_empty() {
            return;
        }

        let representative = &subscriptions[0];
        let (device_oid, timestamp) = {
            let db = db.read().await;
            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|o| o.object_type() == ObjectType::DEVICE)
                .unwrap_or_else(|| ObjectIdentifier::new(ObjectType::DEVICE, 0).unwrap());
            let timestamp = if subscriptions.iter().any(|sub| sub.timestamped) {
                match db.clock_frame() {
                    Some(clock_frame) if clock_frame.is_valid_actual_datetime() => {
                        Some(cov_multiple_datetime(clock_frame))
                    }
                    _ => {
                        warn!(
                            "Skipping timestamped COVNotificationMultiple without a valid Device clock"
                        );
                        return;
                    }
                }
            } else {
                None
            };

            (device_oid, timestamp)
        };
        let (items, last_notified) = {
            let db = if snapshot.is_none() {
                Some(db.read().await)
            } else {
                None
            };
            let mut items: Vec<COVNotificationItem> = Vec::new();
            let mut last_notified = Vec::new();

            for sub in subscriptions {
                let Some(property_identifier) = sub.monitored_property else {
                    continue;
                };
                let Some(object) = snapshot
                    .filter(|object| object.object_identifier() == sub.monitored_object_identifier)
                    .or_else(|| db.as_deref()?.get(&sub.monitored_object_identifier))
                else {
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
                    time_of_change: if sub.timestamped {
                        timestamp.map(|(_, time)| time)
                    } else {
                        None
                    },
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

            if let Some(db) = db.as_deref() {
                life_safety::append_status_flags(db, subscriptions, timestamp, &mut items);
            }

            (items, last_notified)
        };

        if items.is_empty() {
            return;
        }

        let time_remaining = cov_multiple_time_remaining(representative.expires_at);

        let notification = COVNotificationMultipleRequest {
            subscriber_process_identifier: representative.subscriber_process_identifier,
            initiating_device_identifier: device_oid,
            time_remaining,
            timestamp,
            list_of_cov_notifications: items,
        };

        if representative.issue_confirmed_notifications {
            let (operation, result_rx) = match notification_transactions.reserve(
                Self::canonical_cov_peer(representative),
                ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
            ) {
                Ok(reservation) => reservation,
                Err(error) => {
                    warn!(%error, "No free invoke ID for confirmed COVNotificationMultiple");
                    return;
                }
            };
            let id = operation.invoke_id();

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

            if !budget.try_consume(buf.len()) {
                counters
                    .notifications_throttled_fanout
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }

            let guard = match in_flight_tracker.try_acquire(
                representative.peer_key(),
                config.cov_policy.max_confirmed_in_flight_per_peer,
                cov_in_flight,
            ) {
                Ok(guard) => guard,
                Err(InFlightAcquireError::PeerLimitExceeded) => {
                    counters
                        .notifications_throttled_peer
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(InFlightAcquireError::GlobalPoolExhausted) => {
                    warn!("255 confirmed COV notifications in-flight, skipping COVNotificationMultiple");
                    return;
                }
            };

            counters.notifications_sent.fetch_add(1, Ordering::Relaxed);
            counters
                .notifications_confirmed
                .fetch_add(1, Ordering::Relaxed);
            counters
                .notification_bytes_sent
                .fetch_add(buf.len() as u64, Ordering::Relaxed);

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
            let apdu_retries = DEFAULT_APDU_RETRIES;
            tokio::spawn(async move {
                let _guard = guard;
                let result = run_notification_worker(
                    operation,
                    result_rx,
                    apdu_timeout,
                    apdu_retries,
                    |attempt| {
                        let network = Arc::clone(&network);
                        let buf = buf.clone();
                        let sub = sub.clone();
                        async move {
                            let result = Self::send_cov_apdu(&network, &buf, &sub, true).await;
                            match &result {
                                Ok(()) => debug!(
                                    invoke_id = id,
                                    attempt, "Confirmed COVNotificationMultiple sent"
                                ),
                                Err(error) => warn!(
                                    %error,
                                    attempt, "COVNotificationMultiple send failed"
                                ),
                            }
                            result
                        }
                    },
                )
                .await;
                match result {
                    NotificationWorkerResult::Ack => {
                        debug!(invoke_id = id, "COVNotificationMultiple acknowledged");
                    }
                    NotificationWorkerResult::Error => warn!(
                        invoke_id = id,
                        "COVNotificationMultiple rejected by subscriber"
                    ),
                    NotificationWorkerResult::Exhausted => warn!(
                        invoke_id = id,
                        "COVNotificationMultiple failed after {} retries", apdu_retries
                    ),
                    NotificationWorkerResult::Closed => {}
                }
            });
        } else {
            let buf = match Self::encode_unconfirmed_cov_multiple_apdu(&notification) {
                Ok(buf) => buf,
                Err(e) => {
                    warn!(error = %e, "Failed to encode unconfirmed COVNotificationMultiple");
                    return;
                }
            };

            if !budget.try_consume(buf.len()) {
                counters
                    .notifications_throttled_fanout
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }

            counters.notifications_sent.fetch_add(1, Ordering::Relaxed);
            counters
                .notifications_unconfirmed
                .fetch_add(1, Ordering::Relaxed);
            counters
                .notification_bytes_sent
                .fetch_add(buf.len() as u64, Ordering::Relaxed);

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
}
