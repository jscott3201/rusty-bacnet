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
        self.transactions
            .audit_failure_queue(status)?
            .observe(AuditFailureContext {
                epoch: status.auditing_failure_epoch()?,
                status: Arc::clone(status),
                device,
                confirmed,
                peer: route.canonical_peer.clone(),
                route,
                max_apdu: self.config.max_apdu_length,
            })
    }

    pub(super) fn resource_drop(&self, pending: &PendingWrite) {
        let Some(ticket) = pending.failure.clone() else {
            return;
        };
        let Some(queue) = self.transactions.audit_failure_queue(&pending.status) else {
            return;
        };
        let Some(mut worker) = queue.record_drop(
            self.transactions,
            ticket,
            pending
                .notification
                .target_timestamp
                .clone()
                .expect("completed record"),
            1,
        ) else {
            return;
        };
        let network = Arc::clone(self.network);
        let comm_state = Arc::clone(self.comm_state);
        self.transactions.spawn(async move {
            while let Some((batch, _permit, reserved)) = worker.next().await {
                let Some(completion) = DeliveryCompletion::auditing_failure(
                    Arc::clone(&batch.context.status),
                    batch.context.epoch,
                ) else {
                    continue;
                };
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
                completion.finish(delivered);
            }
        });
    }
}
