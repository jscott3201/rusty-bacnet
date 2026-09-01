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
        let subs: Vec<CovSubscription> = {
            let mut table = cov_table.write().await;
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
                .collect()
        };
        let (single_subs, multiple_subs): (Vec<_>, Vec<_>) = subs
            .into_iter()
            .partition(|sub| sub.notification_kind == CovNotificationKind::Single);

        Self::fire_cov_notifications_for_subscriptions(
            db,
            network,
            cov_table,
            cov_in_flight,
            notification_transactions,
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
            notification_transactions,
            comm_state,
            config,
            Some(oid),
            &multiple_subs,
        )
        .await;
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

pub(super) fn single_property_values(
    object: &dyn bacnet_objects::traits::BACnetObject,
    property: PropertyIdentifier,
    array_index: Option<u32>,
) -> Option<Vec<BACnetPropertyValue>> {
    let property_value = object.read_property(property, array_index).ok()?;
    let mut value_buf = BytesMut::new();
    encode_property_value(&mut value_buf, &property_value).ok()?;
    let mut values = vec![BACnetPropertyValue {
        property_identifier: property,
        property_array_index: array_index,
        value: value_buf.to_vec(),
        priority: None,
    }];
    if crate::life_safety_cov::is_life_safety_object(object.object_identifier())
        && property != PropertyIdentifier::STATUS_FLAGS
    {
        let status_flags = object
            .read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .ok()?;
        let mut status_buf = BytesMut::new();
        encode_property_value(&mut status_buf, &status_flags).ok()?;
        values.push(BACnetPropertyValue {
            property_identifier: PropertyIdentifier::STATUS_FLAGS,
            property_array_index: None,
            value: status_buf.to_vec(),
            priority: None,
        });
    }
    Some(values)
}

pub(super) fn append_status_flags(
    db: &ObjectDatabase,
    subscriptions: &[CovSubscription],
    timestamp: Option<(
        bacnet_types::primitives::Date,
        bacnet_types::primitives::Time,
    )>,
    items: &mut [COVNotificationItem],
) {
    for item in items {
        if !crate::life_safety_cov::is_life_safety_object(item.monitored_object_identifier)
            || item
                .list_of_values
                .iter()
                .any(|value| value.property_identifier == PropertyIdentifier::STATUS_FLAGS)
        {
            continue;
        }
        let Some(object) = db.get(&item.monitored_object_identifier) else {
            continue;
        };
        let Ok(status_flags) = object.read_property(PropertyIdentifier::STATUS_FLAGS, None) else {
            continue;
        };
        let mut value_buf = BytesMut::new();
        if encode_property_value(&mut value_buf, &status_flags).is_err() {
            continue;
        }
        let timestamped = subscriptions.iter().any(|sub| {
            sub.monitored_object_identifier == item.monitored_object_identifier && sub.timestamped
        });
        item.list_of_values.push(COVNotificationValue {
            property_identifier: PropertyIdentifier::STATUS_FLAGS,
            property_array_index: None,
            value: value_buf.to_vec(),
            time_of_change: timestamped
                .then(|| timestamp.map(|(_, time)| time))
                .flatten(),
        });
    }
}
