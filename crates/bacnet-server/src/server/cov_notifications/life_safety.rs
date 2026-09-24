use super::*;

/// Fire exact-delta COV notifications for one Life Safety object.
///
/// Whole-object subscriptions observe only Present_Value/Status_Flags.
/// Property subscriptions observe their property and every actual
/// Status_Flags change. Callers supply committed readback deltas after
/// releasing the object-database write lock.
impl<T: TransportPort + 'static> BACnetServer<T> {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::server) async fn fire_life_safety_cov_notifications(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        notification_transactions: &Arc<NotificationTransactions>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        oid: &ObjectIdentifier,
        changed_properties: &[PropertyIdentifier],
    ) {
        if comm_state.load(Ordering::Acquire) >= 1 || changed_properties.is_empty() {
            return;
        }
        let status_changed = changed_properties.contains(&PropertyIdentifier::STATUS_FLAGS);
        let (subs, counters, in_flight_tracker, dispatch_turn) = {
            let mut table = cov_table.write().await;
            (
                table
                    .subscriptions_for(oid)
                    .into_iter()
                    .filter(|sub| match sub.monitored_property {
                        Some(property) => status_changed || changed_properties.contains(&property),
                        None => {
                            status_changed
                                || changed_properties.contains(&PropertyIdentifier::PRESENT_VALUE)
                        }
                    })
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
                None,
                status_changed,
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
                None,
                status_changed,
                &mut budget,
            )
            .await;
        } else {
            let single_first = dispatch_turn % 2 == 0;
            let rem_notifs = budget.remaining_notifications();
            let first_notif_cap = (rem_notifs / 2) + (rem_notifs % 2);
            let rem_bytes = budget.remaining_bytes();
            let first_bytes_cap = (rem_bytes / 2) + (rem_bytes % 2);
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
                    None,
                    status_changed,
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
                    None,
                    status_changed,
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
                    None,
                    status_changed,
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
                    None,
                    status_changed,
                    &mut budget,
                )
                .await;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::server) async fn fire_post_write_cov_notifications(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        notification_transactions: &Arc<NotificationTransactions>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        coarse_oids: &[ObjectIdentifier],
        exact_changes: &[crate::life_safety_cov::LifeSafetyCovChange],
    ) {
        for oid in coarse_oids {
            Self::fire_cov_notifications(
                db,
                network,
                cov_table,
                cov_in_flight,
                notification_transactions,
                comm_state,
                config,
                oid,
            )
            .await;
        }
        for change in exact_changes {
            Self::fire_life_safety_cov_notifications(
                db,
                network,
                cov_table,
                cov_in_flight,
                notification_transactions,
                comm_state,
                config,
                &change.object_identifier,
                &change.changed_properties,
            )
            .await;
        }
    }
}
