use super::event_notifications::resolve_committed_event_enrollment_transition;
use super::*;

pub(super) fn spawn_event_enrollment_task<T: TransportPort + 'static>(
    db: Arc<RwLock<ObjectDatabase>>,
    network: Arc<NetworkLayer<T>>,
    comm_state: Arc<AtomicU8>,
    server_tsm: Arc<Mutex<ServerTsm>>,
    notification_transactions: Arc<NotificationTransactions>,
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
    period: Duration,
    retry_ms: u64,
) -> JoinHandle<()> {
    let evaluation_interval_secs = period.as_secs().max(1);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(period);
        // A stalled runtime must not fire a burst of catch-up passes; the
        // adjacent intrinsic-reporting task sets this for the same reason.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            // Evaluate, commit, and project every result in enrollment order
            // while one database guard is held. The projector consumes
            // committed timestamp/ACK/clock inputs only; it neither stages
            // another timestamp nor repeats transition actions. The guard is
            // dropped before the first send.
            let (report, outbound) = {
                let mut db_guard = db.write().await;
                let evaluation = crate::event_enrollment::evaluate_event_enrollments_for_delivery(
                    &mut db_guard,
                    evaluation_interval_secs,
                );
                let outbound = evaluation
                    .deliveries
                    .into_iter()
                    .filter_map(|committed| {
                        resolve_committed_event_enrollment_transition(&db_guard, committed)
                    })
                    .filter(|(_, distribute, _)| *distribute)
                    .map(|(oid, _, transition)| (oid, transition))
                    .collect::<Vec<_>>();
                (evaluation.report, outbound)
            };
            for transition in &report.transitions {
                debug!(
                    enrollment = %transition.enrollment_oid,
                    monitored = %transition.monitored_oid,
                    from = ?transition.change.from,
                    to = ?transition.change.to,
                    distribute = transition.distribute,
                    "Event enrollment: state changed"
                );
            }
            crate::event_enrollment::log_evaluation_report(&report);
            for (oid, transition) in outbound {
                BACnetServer::<T>::build_and_send_event_notification_with_bindings(
                    &db,
                    &network,
                    &comm_state,
                    &server_tsm,
                    &notification_transactions,
                    &device_bindings,
                    &oid,
                    transition,
                    retry_ms,
                )
                .await;
            }
        }
    })
}

#[cfg(test)]
#[path = "event_enrollment_notification_tests.rs"]
mod tests;
