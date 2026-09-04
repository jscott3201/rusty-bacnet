use super::*;

mod durable_receipt;
pub(super) use bacnet_objects::audit::ConfirmedAuditNotificationOutcome::{Duplicate, Stored};

/// Decode, authorize, and durably store one admitted confirmed Audit request.
pub(super) async fn receive_confirmed_audit_notification(
    db: &Arc<RwLock<ObjectDatabase>>,
    config: &ServerConfig,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    confirmed: &bacnet_encoding::apdu::ConfirmedRequest,
) -> Result<bacnet_objects::audit::ConfirmedAuditNotificationOutcome, Error> {
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
            return Ok(bacnet_objects::audit::ConfirmedAuditNotificationOutcome::Duplicate);
        }
    }

    let context = AuditNotificationAuthorizationContext {
        source_mac: MacAddr::from_slice(source_mac),
        source_network: source_network.cloned(),
        invoke_id: confirmed.invoke_id,
        audit_log_sink: sink,
        request: request.clone(),
    };
    if !config
        .audit_notification_authorizer
        .as_ref()
        .is_some_and(|authorizer| fail_closed_authorize(|| authorizer(&context)))
    {
        return Err(request_denied());
    }

    let mut db = db.write().await;
    let receipt = bacnet_objects::audit::CompletedAuditReceipt::new(
        receipt_identity.key().to_vec(),
        current_unix_millis()?,
    )?;
    handlers::handle_confirmed_audit_notification_with_receipt(&mut db, sink, &request, receipt)
}

/// Decode, authorize, and durably store one unconfirmed Audit request.
///
/// The caller intentionally discards the result because an unconfirmed service
/// never emits a response APDU.
pub(super) async fn receive_unconfirmed_audit_notification(
    db: &Arc<RwLock<ObjectDatabase>>,
    config: &ServerConfig,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    service_request: &Bytes,
) -> Result<(), Error> {
    validate_payload_size("UnconfirmedAuditNotification", service_request)?;
    decode_authorize_and_store(
        db,
        config,
        "UnconfirmedAuditNotification",
        service_request,
        |sink, request| {
            let context = UnconfirmedAuditNotificationAuthorizationContext {
                source_mac: MacAddr::from_slice(source_mac),
                source_network: source_network.cloned(),
                audit_log_sink: sink,
                request: request.clone(),
            };
            config
                .unconfirmed_audit_notification_authorizer
                .as_ref()
                .is_some_and(|authorizer| fail_closed_authorize(|| authorizer(&context)))
        },
    )
    .await
}

fn validate_payload_size(service: &str, service_request: &Bytes) -> Result<(), Error> {
    if service_request.len() > MAX_AUDIT_NOTIFICATION_BYTES {
        return Err(Error::OutOfRange(format!(
            "{service} payload exceeds {MAX_AUDIT_NOTIFICATION_BYTES} bytes"
        )));
    }
    Ok(())
}

async fn decode_authorize_and_store<F>(
    db: &Arc<RwLock<ObjectDatabase>>,
    config: &ServerConfig,
    service: &str,
    service_request: &Bytes,
    authorize: F,
) -> Result<(), Error>
where
    F: FnOnce(ObjectIdentifier, &bacnet_services::audit::AuditNotificationRequest) -> bool,
{
    let request = decode_request(service, service_request)?;
    let sink = config.audit_notification_sink.ok_or_else(request_denied)?;
    if !authorize(sink, &request) {
        return Err(request_denied());
    }

    let mut db = db.write().await;
    handlers::handle_audit_notification(&mut db, sink, &request)
}

fn decode_request(
    service: &str,
    service_request: &Bytes,
) -> Result<bacnet_services::audit::AuditNotificationRequest, Error> {
    let request = bacnet_services::audit::AuditNotificationRequest::decode(service_request)?;
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

fn fail_closed_authorize(authorize: impl FnOnce() -> bool) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(authorize)).unwrap_or(false)
}

fn request_denied() -> Error {
    Error::Protocol {
        class: ErrorClass::SERVICES.to_raw() as u32,
        code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
    }
}
