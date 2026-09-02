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
    monotonic_origin: Instant,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut next_deadline = None;
        loop {
            if let Some(deadline) = next_deadline {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = tokio::time::sleep_until(monotonic_origin + deadline) => {}
                }
            } else {
                interval.tick().await;
            }
            let now = Instant::now().saturating_duration_since(monotonic_origin);

            let changed = {
                let mut database = db.write().await;
                let mut changed = Vec::new();
                next_deadline = None;
                database.for_each_object_mut(|oid, object| {
                    if object.advance_monotonic_time_internal(now) {
                        if let Some(snapshot) = object.cov_snapshot_internal() {
                            changed.push((oid, snapshot));
                        }
                    }
                    if let Some(deadline) = object.next_monotonic_deadline_internal() {
                        next_deadline =
                            Some(next_deadline.map_or(deadline, |next| next.min(deadline)));
                    }
                });
                changed
            };

            for (oid, snapshot) in changed {
                BACnetServer::<T>::fire_cov_notifications_from_snapshot(
                    &db,
                    &network,
                    &cov_table,
                    &cov_in_flight,
                    &notification_transactions,
                    &comm_state,
                    &config,
                    &oid,
                    snapshot.as_ref(),
                )
                .await;
            }
        }
    })
}
