use super::event_send::Settle;
use super::*;

/// Owned handles and settings the event-enrollment task moves into its loop.
pub(super) struct EventEnrollmentTask<T: TransportPort + 'static> {
    pub(super) db: Arc<RwLock<ObjectDatabase>>,
    pub(super) network: Arc<NetworkLayer<T>>,
    pub(super) comm_state: Arc<CommState>,
    pub(super) learned_routers: Arc<Mutex<LearnedRouterCache>>,
    pub(super) notification_transactions: Arc<NotificationTransactions>,
    pub(super) device_bindings: Arc<RwLock<DeviceBindingTable>>,
    pub(super) suppressions: Arc<super::event_suppression::EventSuppressions>,
    /// Evaluation period, already clamped by the caller.
    pub(super) period: Duration,
    /// The confirmed retry timeout, the APDU capacity and the identity whose
    /// Device the notifications name as their initiator (#2217).
    pub(super) config: Arc<ServerConfig>,
}

pub(super) fn spawn_event_enrollment_task<T: TransportPort + 'static>(
    task: EventEnrollmentTask<T>,
) -> JoinHandle<()> {
    let EventEnrollmentTask {
        db,
        network,
        comm_state,
        learned_routers,
        notification_transactions,
        device_bindings,
        suppressions,
        period,
        config,
    } = task;
    let evaluation_interval_secs = crate::event_enrollment::core_interval(period.as_secs()).max(1);
    let owner = notification_transactions.audit_owner_lease();
    super::heap_futures::spawn_boxed(move || async move {
        let _owner = owner;
        let mut interval = tokio::time::interval(period);
        // A stalled runtime must not fire a burst of catch-up passes; the
        // adjacent intrinsic-reporting task sets this for the same reason.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            // Evaluate and commit every enrollment, in database order, while
            // one database guard is held. Each delivery carries the
            // timestamp, message text, ACK policy and clock it committed, so
            // sending it stages no timestamp and repeats no transition
            // action. The guard is dropped before the first send.
            let report = {
                let mut db_guard = db.write().await;
                let evaluation = crate::event_enrollment::evaluate_event_enrollments_for_delivery(
                    &mut db_guard,
                    evaluation_interval_secs,
                );
                crate::event_enrollment::log_suppressed_deliveries(&evaluation.suppressed);
                // Each delivery joins its recipients' lanes under this guard,
                // in commit order (#2537); nothing here waits for them.
                let delivery = EventDelivery {
                    db: &db,
                    network: &network,
                    comm_state: &comm_state,
                    learned_routers: &learned_routers,
                    notification_transactions: &notification_transactions,
                    device_bindings: &device_bindings,
                    suppressions: &suppressions,
                    retry_timeout_ms: config.cov_retry_timeout_ms,
                    local_apdu_capacity: u32::from(config.device.max_apdu_length()),
                    device: crate::local_device::initiating_device(&config.device),
                };
                let mut settle = Settle::default();
                for committed in evaluation.deliveries {
                    if committed.distribute() {
                        BACnetServer::<T>::enqueue_event_notification(
                            &delivery,
                            &mut db_guard,
                            &committed.enrollment_oid(),
                            committed.into_notification(),
                            &mut settle,
                        );
                    }
                }
                evaluation.report
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
        }
    })
}

#[cfg(test)]
#[path = "event_enrollment_notification_tests.rs"]
mod tests;
