//! One supervised scheduler; active sends are bounded by the existing shared permits.
use super::audit_batch_queue::{AuditBatchQueue, LocalDisposition, QueuedAudit};
use super::*;
use futures_util::{stream::FuturesUnordered, StreamExt};

pub(super) fn start<T: TransportPort + 'static>(
    queue: Arc<AuditBatchQueue>,
    network: Arc<NetworkLayer<T>>,
    transactions: &Arc<NotificationTransactions>,
    comm_state: Arc<AtomicU8>,
) {
    let weak = Arc::downgrade(transactions);
    transactions.spawn(async move {
        let mut sends = FuturesUnordered::new();
        loop {
            let changed = queue.changed.notified(); tokio::pin!(changed); changed.as_mut().enable();
            let Some(transactions) = weak.upgrade() else { queue.close(); return; };
            let now = tokio::time::Instant::now();
            if queue.stop_deadline().is_some_and(|deadline| now >= deadline || (queue.empty() && sends.is_empty() && transactions.audit_idle())) {
                // Only known locally unattempted records are resource losses. Close does
                // not invent remote-loss counts for active sends or confirmed ACK waits.
                for record in queue.close() { loss(&transactions,&network,&comm_state,&record); }
                transactions.close();
                return;
            }
            loop {
                let Some(records) = queue.take_due() else { break; };
                let local = LocalDisposition::new(Arc::clone(&queue), &records);
                let permit = transactions.try_admit_audit();
                let reserved = if records[0].confirmed {
                    transactions.reserve(records[0].route.canonical_peer.clone(), ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION).map(Some)
                } else { Ok(None) };
                match (permit,reserved) {
                    (Ok(permit),Ok(reserved)) => {
                        let bytes = encode_batch(&records, reserved.as_ref().map_or(0,|(operation,_)|operation.invoke_id()));
                        let network = Arc::clone(&network); let comm_state = Arc::clone(&comm_state);
                        let deadline = (now+Duration::from_secs(3)).min(queue.stop_deadline().unwrap_or(now+Duration::from_secs(3)));
                        sends.push(async move {
                            let _permit = permit;
                            let delivered = super::audit_reporter::deliver_observed(&network,&comm_state,&records[0].route,&bytes,reserved,deadline,Some(local)).await;
                            for record in records { record.completion.finish(delivered); }
                        });
                    }
                    _ => {
                        for record in records { loss(&transactions,&network,&comm_state,&record); }
                        drop(local);
                    }
                }
            }
            drop(transactions); // No strong JoinSet-owner cycle while suspended.
            let deadline = queue.next_deadline();
            tokio::select! {
                _ = &mut changed => {},
                _ = sends.next(), if !sends.is_empty() => {},
                _ = async { if let Some(deadline) = deadline { tokio::time::sleep_until(deadline).await; } else { std::future::pending::<()>().await; } } => {},
            }
        }
    });
}
fn loss<T: TransportPort + 'static>(
    transactions: &Arc<NotificationTransactions>,
    network: &Arc<NetworkLayer<T>>,
    comm_state: &Arc<AtomicU8>,
    record: &QueuedAudit,
) {
    super::audit_reporter::record_resource_drop(
        transactions,
        network,
        comm_state,
        &record.status,
        Some(record.failure.clone()),
        record.timestamp.clone(),
    );
}
fn encode_batch(records: &[QueuedAudit], invoke_id: u8) -> BytesMut {
    let first = &records[0];
    let mut service = BytesMut::new();
    bacnet_encoding::tags::encode_opening_tag(&mut service, 0);
    for record in records {
        service.extend_from_slice(&record.record);
    }
    bacnet_encoding::tags::encode_closing_tag(&mut service, 0);
    let pdu = if first.confirmed {
        Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: first.max_apdu as u16,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
            service_request: service.freeze(),
        })
    } else {
        Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION,
            service_request: service.freeze(),
        })
    };
    let mut bytes = BytesMut::new();
    encode_apdu(&mut bytes, &pdu).expect("validated captured records and APDU header");
    debug_assert!(bytes.len() <= first.max_apdu as usize);
    bytes
}
