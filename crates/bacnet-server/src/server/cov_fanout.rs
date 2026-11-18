//! The COV fanout a background commit owes, sent after its guard is dropped,
//! and the follow-up fanout an acknowledged confirmed report owes (#896).
use super::cov_notify_context::CovNotifyContext;
use super::event_delivery::EventDelivery;
use super::*;
use crate::committed_cov::CommittedCov;

/// Shared handles for fanning COV out from a background task (#889). It uses
/// the write path's post-commit fanout, so the ordering, budget and DCC rules
/// are the same as for a network write.
pub(super) struct CovFanout<T: TransportPort + 'static> {
    pub(super) db: Arc<RwLock<ObjectDatabase>>,
    pub(super) cov_table: Arc<RwLock<CovSubscriptionTable>>,
    network: Arc<NetworkLayer<T>>,
    cov_in_flight: Arc<Semaphore>,
    pub(super) notification_transactions: Arc<NotificationTransactions>,
    comm_state: Arc<CommState>,
    event_suppressions: Arc<super::event_suppression::EventSuppressions>,
    learned_routers: Arc<Mutex<LearnedRouterCache>>,
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
    config: Arc<ServerConfig>,
}

impl<T: TransportPort + 'static> Clone for CovFanout<T> {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            cov_table: Arc::clone(&self.cov_table),
            network: Arc::clone(&self.network),
            cov_in_flight: Arc::clone(&self.cov_in_flight),
            notification_transactions: Arc::clone(&self.notification_transactions),
            comm_state: Arc::clone(&self.comm_state),
            event_suppressions: Arc::clone(&self.event_suppressions),
            learned_routers: Arc::clone(&self.learned_routers),
            device_bindings: Arc::clone(&self.device_bindings),
            config: Arc::clone(&self.config),
        }
    }
}

impl<T: TransportPort + 'static> CovFanout<T> {
    /// Own the handles of `ctx`, plus those the background tasks'
    /// EventNotification sends read: the event counters, the learned routers
    /// and the Device bindings.
    pub(super) fn new(
        ctx: &CovNotifyContext<'_, T>,
        event_suppressions: &Arc<super::event_suppression::EventSuppressions>,
        learned_routers: &Arc<Mutex<LearnedRouterCache>>,
        device_bindings: &Arc<RwLock<DeviceBindingTable>>,
    ) -> Self {
        Self {
            db: Arc::clone(ctx.db),
            cov_table: Arc::clone(ctx.cov_table),
            network: Arc::clone(ctx.network),
            cov_in_flight: Arc::clone(ctx.cov_in_flight),
            notification_transactions: Arc::clone(ctx.notification_transactions),
            comm_state: Arc::clone(ctx.comm_state),
            event_suppressions: Arc::clone(event_suppressions),
            learned_routers: Arc::clone(learned_routers),
            device_bindings: Arc::clone(device_bindings),
            config: Arc::clone(ctx.config),
        }
    }

    /// Borrow the handles a COV notification pass reads.
    pub(super) fn notify_context(&self) -> CovNotifyContext<'_, T> {
        CovNotifyContext {
            db: &self.db,
            network: &self.network,
            cov_table: &self.cov_table,
            cov_in_flight: &self.cov_in_flight,
            notification_transactions: &self.notification_transactions,
            comm_state: &self.comm_state,
            config: &self.config,
        }
    }

    /// Borrow the handles an EventNotification send reads. The retry timeout
    /// and APDU capacity come from the server config.
    pub(super) fn event_delivery(&self) -> EventDelivery<'_, T> {
        EventDelivery {
            db: &self.db,
            network: &self.network,
            comm_state: &self.comm_state,
            learned_routers: &self.learned_routers,
            notification_transactions: &self.notification_transactions,
            device_bindings: &self.device_bindings,
            suppressions: &self.event_suppressions,
            retry_timeout_ms: self.config.cov_retry_timeout_ms,
            local_apdu_capacity: u32::from(self.config.device.max_apdu_length()),
            device: crate::local_device::initiating_device(&self.config.device),
        }
    }

    pub(super) async fn fire(&self, committed: &CommittedCov) {
        if committed.is_empty() {
            return;
        }
        BACnetServer::<T>::fire_post_write_cov_notifications(
            &self.notify_context(),
            &committed.coarse,
            &committed.life_safety,
        )
        .await;
    }

    /// Fan out again each reference whose confirmed report was acknowledged,
    /// so changes held back while it was outstanding reach the subscriber
    /// (#896), each context whose queued timestamped changes reached
    /// their Max_Notification_Delay deadline (#856), and each reference
    /// whose confirmed report found no invoke ID free, once one comes back
    /// (#2661). Runs until the server aborts it.
    pub(super) async fn run_revisits(self) {
        use futures_util::FutureExt;
        let (revisits, timed) = {
            let table = self.cov_table.read().await;
            (Arc::clone(table.revisits()), table.timed().clone())
        };
        loop {
            // No wait loses work when another wins: revisits stay queued, a
            // due context comes back after its spacing, and references owed
            // an ID keep their places. Each takes what it returns in the
            // poll that returns it, and none holds a lock.
            let (keys, owed) = tokio::select! {
                keys = revisits.next() => (keys, false),
                keys = timed.next_due() => (keys, false),
                keys = owed_look(&revisits, &self.notification_transactions) => (keys, true),
            };
            let ctx = self.notify_context();
            let batch = BACnetServer::<T>::fire_cov_revisits(&ctx, &keys);
            // A panic ends only this batch; later acknowledgments still follow up.
            if std::panic::AssertUnwindSafe(batch)
                .catch_unwind()
                .await
                .is_err()
            {
                warn!(
                    references = keys.len(),
                    "COV follow-up fanout panicked; those references report on their next fanout"
                );
            }
            if owed {
                revisits.looked(&keys);
            }
        }
    }
}

/// Wait until references owed an invoke ID can be looked at again, and take
/// as many as there are IDs free, oldest first (#2661). Each look follows a
/// lease ending, or a reference owed while IDs were free, so at the limit
/// it waits rather than spins: a reference refused again keeps its place
/// for the next ID. While none is owed it waits for one to be, and an ID
/// coming back costs nothing. Cancel-safe: references are taken only in the
/// poll that returns them.
async fn owed_look(
    revisits: &crate::cov::CovRevisits,
    ids: &NotificationTransactions,
) -> Vec<bacnet_device_core::cov::CovSubscriptionKey> {
    loop {
        let owed = revisits.owed_notified();
        if !revisits.owes_invoke_id() {
            owed.await;
            continue;
        }
        // Made before the count, so a lease that ends after it still ends
        // this wait.
        let released = ids.released();
        let keys = revisits.take_owed(ids.free_invoke_ids());
        if !keys.is_empty() {
            return keys;
        }
        tokio::select! {
            () = released => {}
            () = owed => {}
        }
    }
}
