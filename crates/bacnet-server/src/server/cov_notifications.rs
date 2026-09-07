use super::*;
use crate::cov::{AtomicCovCounters, CovInFlightTracker, InFlightAcquireError};

mod life_safety;
mod multiple;

#[derive(Debug)]
pub(super) struct EventBudget {
    max_notifications: usize,
    max_bytes: usize,
    notifications_sent: usize,
    bytes_sent: usize,
}

impl EventBudget {
    pub(super) fn new(policy: &CovPolicy) -> Self {
        Self {
            max_notifications: policy.max_notifications_per_event,
            max_bytes: policy.max_notification_bytes_per_event,
            notifications_sent: 0,
            bytes_sent: 0,
        }
    }

    pub(super) fn with_limits(max_notifications: usize, max_bytes: usize) -> Self {
        Self {
            max_notifications,
            max_bytes,
            notifications_sent: 0,
            bytes_sent: 0,
        }
    }

    pub(super) fn remaining_notifications(&self) -> usize {
        self.max_notifications
            .saturating_sub(self.notifications_sent)
    }

    pub(super) fn remaining_bytes(&self) -> usize {
        self.max_bytes.saturating_sub(self.bytes_sent)
    }

    pub(super) fn is_exhausted(&self) -> bool {
        self.notifications_sent >= self.max_notifications || self.bytes_sent >= self.max_bytes
    }

    #[allow(dead_code)]
    pub(super) fn has_capacity(&self) -> bool {
        !self.is_exhausted()
    }

    pub(super) fn try_consume(&mut self, bytes: usize) -> bool {
        if self.notifications_sent >= self.max_notifications {
            return false;
        }
        if self.bytes_sent.saturating_add(bytes) > self.max_bytes {
            return false;
        }
        self.notifications_sent += 1;
        self.bytes_sent = self.bytes_sent.saturating_add(bytes);
        true
    }

    #[allow(dead_code)]
    pub(super) fn refund(&mut self, bytes: usize) {
        self.notifications_sent = self.notifications_sent.saturating_sub(1);
        self.bytes_sent = self.bytes_sent.saturating_sub(bytes);
    }

    pub(super) fn consume_sub_budget(&mut self, sub_budget: &EventBudget) {
        self.notifications_sent = self
            .notifications_sent
            .saturating_add(sub_budget.notifications_sent);
        self.bytes_sent = self.bytes_sent.saturating_add(sub_budget.bytes_sent);
    }
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

    fn canonical_cov_peer(
        sub: &CovSubscription,
    ) -> bacnet_endpoint_core::coordinator::CanonicalPeer {
        match &sub.subscriber_network {
            Some(destination) => {
                canonical_routed_peer(destination.network, &destination.mac_address)
            }
            None => canonical_direct_peer(&sub.subscriber_mac),
        }
    }

    /// Fire COV notifications for all active subscriptions on the given object.
    /// Skipped when DCC is active (comm_state >= 1).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn fire_cov_notifications(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        notification_transactions: &Arc<NotificationTransactions>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        oid: &ObjectIdentifier,
    ) {
        Self::fire_cov_notifications_inner(
            db,
            network,
            cov_table,
            cov_in_flight,
            notification_transactions,
            comm_state,
            config,
            oid,
            None,
        )
        .await;
    }

    pub(super) async fn fire_cov_notifications_inner(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        notification_transactions: &Arc<NotificationTransactions>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        oid: &ObjectIdentifier,
        snapshot: Option<&dyn bacnet_objects::traits::BACnetObject>,
    ) {
        if comm_state.load(Ordering::Acquire) >= 1 {
            return;
        }
        let (subs, counters, in_flight_tracker, dispatch_turn) = {
            let mut table = cov_table.write().await;
            (
                table
                    .subscriptions_for(oid)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>(),
                Arc::clone(table.counters()),
                Arc::clone(table.in_flight_tracker()),
                table.next_dispatch_turn(),
            )
        };

        if subs.is_empty() {
            return;
        }

        let (single_subs, multiple_subs): (Vec<_>, Vec<_>) = subs
            .into_iter()
            .partition(|sub| sub.notification_kind == CovNotificationKind::Single);

        let mut budget = EventBudget::new(&config.cov_policy);

        if single_subs.is_empty() {
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
                Some(oid),
                &multiple_subs,
                snapshot,
                &mut budget,
            )
            .await;
        } else if multiple_subs.is_empty() {
            Self::fire_cov_notifications_for_subscriptions(
                db,
                network,
                cov_table,
                cov_in_flight,
                &in_flight_tracker,
                &counters,
                notification_transactions,
                config,
                oid,
                &single_subs,
                snapshot,
                &mut budget,
            )
            .await;
        } else {
            let single_first = dispatch_turn % 2 == 0;
            let first_notif_cap = (budget.remaining_notifications() + 1) / 2;
            let first_bytes_cap = (budget.remaining_bytes() + 1) / 2;
            let mut first_budget = EventBudget::with_limits(first_notif_cap, first_bytes_cap);

            if single_first {
                Self::fire_cov_notifications_for_subscriptions(
                    db,
                    network,
                    cov_table,
                    cov_in_flight,
                    &in_flight_tracker,
                    &counters,
                    notification_transactions,
                    config,
                    oid,
                    &single_subs,
                    snapshot,
                    &mut first_budget,
                )
                .await;
                budget.consume_sub_budget(&first_budget);

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
                    Some(oid),
                    &multiple_subs,
                    snapshot,
                    &mut budget,
                )
                .await;
            } else {
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
                    Some(oid),
                    &multiple_subs,
                    snapshot,
                    &mut first_budget,
                )
                .await;
                budget.consume_sub_budget(&first_budget);

                Self::fire_cov_notifications_for_subscriptions(
                    db,
                    network,
                    cov_table,
                    cov_in_flight,
                    &in_flight_tracker,
                    &counters,
                    notification_transactions,
                    config,
                    oid,
                    &single_subs,
                    snapshot,
                    &mut budget,
                )
                .await;
            }
        }
    }

    /// Fire the initial COV notification for a newly accepted subscription.
    /// Skipped when DCC is active (comm_state >= 1).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn fire_initial_cov_notification(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        notification_transactions: &Arc<NotificationTransactions>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        subscription: &CovSubscription,
    ) {
        if comm_state.load(Ordering::Acquire) >= 1 {
            return;
        }

        let (counters, in_flight_tracker) = {
            let table = cov_table.read().await;
            (
                Arc::clone(table.counters()),
                Arc::clone(table.in_flight_tracker()),
            )
        };
        let mut budget = EventBudget::new(&config.cov_policy);

        Self::fire_cov_notifications_for_subscriptions(
            db,
            network,
            cov_table,
            cov_in_flight,
            &in_flight_tracker,
            &counters,
            notification_transactions,
            config,
            &subscription.monitored_object_identifier,
            std::slice::from_ref(subscription),
            None,
            &mut budget,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn fire_cov_notifications_for_subscriptions(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        in_flight_tracker: &Arc<CovInFlightTracker>,
        counters: &Arc<AtomicCovCounters>,
        notification_transactions: &Arc<NotificationTransactions>,
        config: &ServerConfig,
        oid: &ObjectIdentifier,
        subs: &[CovSubscription],
        snapshot: Option<&dyn bacnet_objects::traits::BACnetObject>,
        budget: &mut EventBudget,
    ) {
        if budget.is_exhausted() {
            return;
        }

        let device_oid = {
            let db = db.read().await;
            db.list_objects()
                .into_iter()
                .find(|o| o.object_type() == ObjectType::DEVICE)
                .unwrap_or_else(|| ObjectIdentifier::new(ObjectType::DEVICE, 0).unwrap())
        };
        let (values, current_pv, cov_increment) = {
            let db = if snapshot.is_none() {
                Some(db.read().await)
            } else {
                None
            };
            let object = match snapshot.or_else(|| db.as_deref()?.get(oid)) {
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

            (values, current_pv, cov_increment)
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

            if budget.is_exhausted() {
                counters
                    .notifications_throttled_fanout
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            }

            let time_remaining = sub.expires_at.map_or(0, |exp| {
                exp.saturating_duration_since(Instant::now()).as_secs() as u32
            });

            let notification_values = if let Some(prop) = sub.monitored_property {
                if let Some(object) = snapshot {
                    life_safety::single_property_values(
                        object,
                        prop,
                        sub.monitored_property_array_index,
                    )
                    .unwrap_or_else(|| values.clone())
                } else {
                    let db = db.read().await;
                    db.get(oid)
                        .and_then(|object| {
                            life_safety::single_property_values(
                                object,
                                prop,
                                sub.monitored_property_array_index,
                            )
                        })
                        .unwrap_or_else(|| values.clone())
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
                let guard = match in_flight_tracker.try_acquire(
                    sub.peer_key(),
                    config.cov_policy.max_confirmed_in_flight_per_peer,
                    cov_in_flight,
                ) {
                    Ok(guard) => guard,
                    Err(InFlightAcquireError::PeerLimitExceeded) => {
                        counters
                            .notifications_throttled_peer
                            .fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    Err(InFlightAcquireError::GlobalPoolExhausted) => {
                        warn!(
                            object = ?oid,
                            "255 confirmed COV notifications in-flight, skipping notification"
                        );
                        continue;
                    }
                };

                let (operation, result_rx) = match notification_transactions.reserve(
                    Self::canonical_cov_peer(sub),
                    ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                ) {
                    Ok(reservation) => reservation,
                    Err(error) => {
                        warn!(
                            %error,
                            object = ?oid,
                            "No free invoke ID for confirmed COV notification"
                        );
                        continue;
                    }
                };
                let id = operation.invoke_id();

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

                if !budget.try_consume(buf.len()) {
                    counters
                        .notifications_throttled_fanout
                        .fetch_add(1, Ordering::Relaxed);
                    continue;
                }

                counters.notifications_sent.fetch_add(1, Ordering::Relaxed);
                counters
                    .notifications_confirmed
                    .fetch_add(1, Ordering::Relaxed);
                counters
                    .notification_bytes_sent
                    .fetch_add(buf.len() as u64, Ordering::Relaxed);

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
                                        attempt, "Confirmed COV notification sent"
                                    ),
                                    Err(error) => warn!(
                                        %error,
                                        attempt, "COV notification send failed"
                                    ),
                                }
                                result
                            }
                        },
                    )
                    .await;
                    match result {
                        NotificationWorkerResult::Ack => {
                            debug!(invoke_id = id, "COV notification acknowledged");
                        }
                        NotificationWorkerResult::Error => {
                            warn!(invoke_id = id, "COV notification rejected by subscriber");
                        }
                        NotificationWorkerResult::Exhausted => warn!(
                            invoke_id = id,
                            "COV notification failed after {} retries", apdu_retries
                        ),
                        NotificationWorkerResult::Closed => {}
                    }
                });
            } else {
                let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                    service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
                    service_request: service_buf.freeze(),
                });

                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                if !budget.try_consume(buf.len()) {
                    counters
                        .notifications_throttled_fanout
                        .fetch_add(1, Ordering::Relaxed);
                    continue;
                }

                counters.notifications_sent.fetch_add(1, Ordering::Relaxed);
                counters
                    .notifications_unconfirmed
                    .fetch_add(1, Ordering::Relaxed);
                counters
                    .notification_bytes_sent
                    .fetch_add(buf.len() as u64, Ordering::Relaxed);

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
