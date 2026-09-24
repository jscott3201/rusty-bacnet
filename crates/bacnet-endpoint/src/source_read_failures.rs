//! Memory-only source resource losses, never ordinary-record retries.
use super::*;
use tokio::time::{timeout_at, Instant};

pub(super) fn record_drop(
    source: &Arc<SourceRead>,
    owner: &NotificationTransactions,
    ticket: Option<AuditFailureTicket<MacAddr>>,
    timestamp: BACnetTimeStamp,
) {
    let Some(ticket) = ticket else {
        return;
    };
    let Some(mut worker) = source.failures.record_drop(owner, ticket, timestamp, 1) else {
        return;
    };
    let source = Arc::downgrade(source);
    owner.spawn(async move {
        while let Some((batch, permit, reserved)) = worker.next().await {
            let Some(source) = source.upgrade() else {
                return;
            };
            let deadline = Instant::now() + delivery::DEADLINE;
            let Ok(db) = timeout_at(deadline, source.db.read()).await else {
                continue;
            };
            // Old status Arcs can survive database replacement/removal. Check
            // both membership and mutation generation without requiring another READ.
            let current = db
                .get(&source.selected)
                .and_then(|object| object.audit_reporter_internal())
                .is_some_and(|reporter| {
                    Arc::ptr_eq(&reporter.status_internal(), &batch.context.status)
                });
            if !current
                || db.find_by_type(ObjectType::DEVICE) != [batch.context.device]
                || source.operations.is_closed()
                || !source
                    .runtime
                    .upgrade()
                    .is_some_and(|runtime| runtime.owner.is_active())
            {
                continue;
            }
            let Some(completion) = delivery::Completion::auditing_failure(
                Arc::clone(&batch.context.status),
                batch.context.epoch,
            ) else {
                continue;
            };
            let invoke = reserved
                .as_ref()
                .map_or(0, |(operation, _)| operation.invoke_id());
            let Some(encoded) = delivery::encode(
                &batch.notification(),
                batch.context.confirmed,
                source.max_apdu,
                invoke,
            ) else {
                continue;
            };
            // Keep the DB read guard through synchronous egress admission, not
            // through transport or ACK waits. Later changes cannot recall a send.
            let sent = delivery::admit_encoded(
                &source.egress,
                batch.context.route.clone(),
                encoded,
                batch.context.confirmed,
                deadline,
            );
            drop(db);
            if matches!(
                sent,
                Err(
                    bacnet_endpoint_core::endpoint_ingress::EndpointEgressAdmissionError::QueueFull
                )
            ) {
                let egress = source.egress.clone();
                worker.restore(batch);
                #[cfg(test)]
                source.summary_queue_full.notify_one();
                drop(source);
                drop(permit);
                drop(reserved);
                drop(completion);
                if egress.wait_for_capacity().await.is_err() {
                    return;
                }
                continue;
            }
            drop(source);
            completion.finish(delivery::finish_send(sent, reserved, deadline).await);
        }
    });
}
