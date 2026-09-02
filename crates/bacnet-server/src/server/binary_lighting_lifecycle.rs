use super::*;
use tokio::time::{Instant, MissedTickBehavior};

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_binary_lighting_operation_task<T: TransportPort + 'static>(
    db: Arc<RwLock<ObjectDatabase>>,
    network: Arc<NetworkLayer<T>>,
    cov_table: Arc<RwLock<CovSubscriptionTable>>,
    cov_in_flight: Arc<Semaphore>,
    notification_transactions: Arc<NotificationTransactions>,
    comm_state: Arc<AtomicU8>,
    config: ServerConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut previous = Instant::now();

        loop {
            interval.tick().await;
            let now = Instant::now();
            let elapsed = now.saturating_duration_since(previous);
            previous = now;

            let changed = {
                let mut database = db.write().await;
                let mut changed = Vec::new();
                database.for_each_object_mut(|oid, object| {
                    if object.advance_time_internal(elapsed) {
                        changed.push(oid);
                    }
                });
                changed
            };

            for oid in changed {
                BACnetServer::<T>::fire_cov_notifications(
                    &db,
                    &network,
                    &cov_table,
                    &cov_in_flight,
                    &notification_transactions,
                    &comm_state,
                    &config,
                    &oid,
                )
                .await;
            }
        }
    })
}
