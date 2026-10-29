use super::super::audit_forwarder::ForwardBatch;
use super::*;

mod durable_receipt;
pub(super) use bacnet_objects::audit::ConfirmedAuditNotificationOutcome::{Duplicate, Stored};

/// Decode, authorize, and durably store one admitted confirmed Audit request.
pub(super) async fn receive_confirmed_audit_notification(
    db: &Arc<RwLock<ObjectDatabase>>,
    config: &ServerConfig,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    provenance: bacnet_transport::port::TransportProvenance,
    confirmed: &bacnet_encoding::apdu::ConfirmedRequest,
) -> Result<
    (
        bacnet_objects::audit::ConfirmedAuditNotificationOutcome,
        Option<ForwardBatch>,
    ),
    Error,
> {
    validate_payload_size("ConfirmedAuditNotification", &confirmed.service_request)?;
    let request = decode_request("ConfirmedAuditNotification", &confirmed.service_request)?;
    let sink = config.audit_notification_sink.ok_or_else(request_denied)?;
    let precheck_at = current_unix_millis()?;
    let receipt_identity =
        durable_receipt::completed_receipt(source_mac, source_network, confirmed, precheck_at)?;
    {
        let mut db = db.write().await;
        if handlers::has_completed_confirmed_audit_receipt(
            &mut db,
            sink,
            receipt_identity.key(),
            precheck_at,
        )? {
            return Ok((Duplicate, None));
        }
    }

    let context = AuditNotificationAuthorizationContext {
        source_mac: MacAddr::from_slice(source_mac),
        source_network: source_network.cloned(),
        provenance,
        invoke_id: confirmed.invoke_id,
        audit_log_sink: sink,
        request: request.clone(),
    };
    let allowed = match &config.policy {
        Some(policy) => {
            ask_policy(policy, context, |policy, context| {
                policy.authorize_audit_notification(context)
            })
            .await
        }
        None => false,
    };
    if !allowed {
        return Err(request_denied());
    }

    let receipt = bacnet_objects::audit::CompletedAuditReceipt::new(
        receipt_identity.key().to_vec(),
        current_unix_millis()?,
    )?;
    store_staged(
        db,
        sink,
        &request,
        Some(receipt),
        &confirmed.service_request,
    )
    .await
}

/// Store an authorized batch in `sink` with its commit staged, so the commit
/// runs while the database guard is dropped (#1270). The batch reaches
/// memory, and a confirmed one's acknowledgment goes out, only once the
/// commit is durable. Returns the outcome and the post-commit forwarding.
async fn store_staged(
    db: &Arc<RwLock<ObjectDatabase>>,
    sink: ObjectIdentifier,
    request: &bacnet_services::audit::AuditNotificationRequest,
    receipt: Option<bacnet_objects::audit::CompletedAuditReceipt>,
    payload: &Bytes,
) -> Result<
    (
        bacnet_objects::audit::ConfirmedAuditNotificationOutcome,
        Option<ForwardBatch>,
    ),
    Error,
> {
    use bacnet_objects::audit::AuditBatchStage;
    let staging_db = db;
    loop {
        let staged = {
            let mut db = db.write().await;
            match handlers::stage_audit_notification(&mut db, sink, request, receipt.clone())? {
                AuditBatchStage::Done(outcome, changed) => {
                    let forward = ForwardBatch::after_commit(&db, sink, changed, payload.clone());
                    return Ok((outcome, forward));
                }
                AuditBatchStage::Busy(wait) => {
                    drop(db);
                    super::super::durable_writes::note_busy(staging_db);
                    let _ = tokio::time::timeout(super::super::durable_writes::BUSY_RECHECK, wait)
                        .await;
                    continue;
                }
                AuditBatchStage::Staged(staged) => staged,
            }
        };
        super::super::durable_writes::saved(staged.saved()).await;
        let mut db = db.write().await;
        let (outcome, changed) = handlers::finish_audit_notification(&mut db, sink, staged)?;
        let forward = ForwardBatch::after_commit(&db, sink, changed, payload.clone());
        return Ok((outcome, forward));
    }
}

/// Decode, authorize, and durably store one unconfirmed Audit request.
///
/// The caller may start post-commit forwarding, but this unconfirmed service
/// never emits a response APDU, including when storage or forwarding fails.
pub(super) async fn receive_unconfirmed_audit_notification(
    db: &Arc<RwLock<ObjectDatabase>>,
    config: &ServerConfig,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    provenance: bacnet_transport::port::TransportProvenance,
    service_request: &Bytes,
) -> Result<Option<ForwardBatch>, Error> {
    validate_payload_size("UnconfirmedAuditNotification", service_request)?;
    let request = decode_request("UnconfirmedAuditNotification", service_request)?;
    let sink = config.audit_notification_sink.ok_or_else(request_denied)?;
    let context = UnconfirmedAuditNotificationAuthorizationContext {
        source_mac: MacAddr::from_slice(source_mac),
        source_network: source_network.cloned(),
        provenance,
        audit_log_sink: sink,
        request: request.clone(),
    };
    let allowed = match &config.policy {
        Some(policy) => {
            ask_policy(policy, context, |policy, context| {
                policy.authorize_unconfirmed_audit_notification(context)
            })
            .await
        }
        None => false,
    };
    if !allowed {
        return Err(request_denied());
    }
    let (_, forward) = store_staged(db, sink, &request, None, service_request).await?;
    Ok(forward)
}

fn validate_payload_size(service: &str, service_request: &Bytes) -> Result<(), Error> {
    if service_request.len() > MAX_AUDIT_NOTIFICATION_BYTES {
        return Err(Error::OutOfRange(format!(
            "{service} payload exceeds {MAX_AUDIT_NOTIFICATION_BYTES} bytes"
        )));
    }
    Ok(())
}

fn decode_request(
    service: &str,
    service_request: &Bytes,
) -> Result<bacnet_services::audit::AuditNotificationRequest, Error> {
    let request = bacnet_services::audit::AuditNotificationRequest::decode(service_request)
        .map_err(Error::into_request_reject)?;
    if request.notifications.len() > MAX_AUDIT_NOTIFICATIONS {
        return Err(Error::OutOfRange(format!(
            "{service} list exceeds {MAX_AUDIT_NOTIFICATIONS} items"
        )));
    }
    Ok(request)
}

fn current_unix_millis() -> Result<u64, Error> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::OutOfRange("system time precedes Unix epoch".into()))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| Error::OutOfRange("system time exceeds Unix millisecond range".into()))
}

pub(in crate::server) fn fail_closed_authorize(authorize: impl FnOnce() -> bool) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(authorize)).unwrap_or(false)
}

/// Ask `policy` about `context` through `ask` on Tokio's blocking pool, as
/// [`ask_mutation_policy`] does for mutations
/// (#2032): the request task awaits the answer, so a policy that blocks holds
/// no runtime worker and `stop()` can still abort the request. A panic, or a
/// call that never reports back, denies.
pub(in crate::server) async fn ask_policy<C: Send + 'static>(
    policy: &Arc<dyn crate::ServerPolicy>,
    context: C,
    ask: fn(&dyn crate::ServerPolicy, &C) -> bool,
) -> bool {
    let policy = Arc::clone(policy);
    tokio::task::spawn_blocking(move || fail_closed_authorize(|| ask(&*policy, &context)))
        .await
        .unwrap_or(false)
}

pub(super) fn request_denied() -> Error {
    Error::protocol(ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED)
}
