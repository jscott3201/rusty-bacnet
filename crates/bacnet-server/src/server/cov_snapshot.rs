use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn fire_cov_notifications_from_snapshot(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        notification_transactions: &Arc<NotificationTransactions>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        oid: &ObjectIdentifier,
        snapshot: &dyn bacnet_objects::traits::BACnetObject,
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
            Some(snapshot),
        )
        .await;
    }
}
