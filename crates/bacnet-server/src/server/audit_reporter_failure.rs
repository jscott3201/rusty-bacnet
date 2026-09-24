//! Context capture and bounded resource-loss delivery for target records.
use super::*;

impl<T: TransportPort + 'static> WriteAudit<'_, T> {
    pub(super) fn failure_ticket(
        &self,
        status: &Arc<AuditReporterStatus>,
        confirmed: bool,
        device: ObjectIdentifier,
        route: Option<Arc<ConfirmedRecipientRoute>>,
    ) -> Option<AuditFailureTicket<Arc<ConfirmedRecipientRoute>>> {
        let route = route?;
        let ticket = self
            .transactions
            .audit_failure_queue(status)?
            .observe(AuditFailureContext {
                epoch: status.configuration_epoch(),
                status: Arc::clone(status),
                device,
                confirmed,
                peer: route.canonical_peer.clone(),
                route,
                max_apdu: self.config.max_apdu_length,
            });
        if ticket.is_none() {
            // Checked admission identity exhaustion refuses capture without wrapping.
            status.complete_delivery(status.begin_delivery(), false);
        }
        ticket
    }

    pub(super) fn resource_drop(&self, pending: &PendingWrite) {
        record_resource_drop(
            self.transactions,
            self.network,
            self.comm_state,
            &pending.status,
            pending.failure.clone(),
            pending
                .notification
                .target_timestamp
                .clone()
                .expect("completed record"),
        );
    }
}
pub(in crate::server) fn record_resource_drop<T: TransportPort + 'static>(
    transactions: &Arc<NotificationTransactions>,
    network: &Arc<NetworkLayer<T>>,
    comm_state: &Arc<AtomicU8>,
    status: &Arc<AuditReporterStatus>,
    ticket: Option<AuditFailureTicket<Arc<ConfirmedRecipientRoute>>>,
    timestamp: bacnet_types::primitives::BACnetTimeStamp,
) {
    let Some(ticket) = ticket else {
        return;
    };
    let Some(queue) = transactions.audit_failure_queue(status) else {
        return;
    };
    let Some(mut worker) = queue.record_drop(transactions, ticket, timestamp, 1) else {
        return;
    };
    let network = Arc::clone(network);
    let comm_state = Arc::clone(comm_state);
    transactions.spawn(async move {
        while let Some((batch, _permit, reserved)) = worker.next().await {
            let completion = DeliveryCompletion::auditing_failure(
                Arc::clone(&batch.context.status),
                batch.context.epoch,
            );
            let deadline = tokio::time::Instant::now() + DELIVERY_TIMEOUT;
            let invoke = reserved
                .as_ref()
                .map_or(0, |(operation, _)| operation.invoke_id());
            let Some(bytes) = encode_notification(
                &batch.notification(),
                batch.context.confirmed,
                batch.context.max_apdu,
                invoke,
            ) else {
                continue;
            };
            let delivered = deliver(
                &network,
                &comm_state,
                &batch.context.route,
                &bytes,
                reserved,
                deadline,
            )
            .await;
            if let Some(completion) = completion {
                completion.finish(delivered);
            }
        }
    });
}
