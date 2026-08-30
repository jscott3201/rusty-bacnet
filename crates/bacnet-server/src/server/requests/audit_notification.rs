use super::*;

/// Decode, authorize, and durably store one confirmed Audit request.
///
/// `None` means an exact pending/completed duplicate was detected and must be
/// silently discarded. Every other request returns its service result.
pub(super) async fn receive_confirmed_audit_notification(
    db: &Arc<RwLock<ObjectDatabase>>,
    notification_transactions: &Arc<NotificationTransactions>,
    config: &ServerConfig,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    invoke_id: u8,
    service_request: &Bytes,
) -> Option<Result<(), Error>> {
    if let Err(error) = validate_payload_size("ConfirmedAuditNotification", service_request) {
        return Some(Err(error));
    }

    let pending = match notification_transactions
        .audit_notification_tracker()
        .begin(
            source_mac,
            source_network,
            invoke_id,
            service_request.clone(),
        ) {
        DuplicateAdmission::Duplicate => return None,
        DuplicateAdmission::New(pending) => pending,
    };
    let execution = decode_authorize_and_store(
        db,
        config,
        "ConfirmedAuditNotification",
        service_request,
        |sink, request| {
            let context = AuditNotificationAuthorizationContext {
                source_mac: MacAddr::from_slice(source_mac),
                source_network: source_network.cloned(),
                invoke_id,
                audit_log_sink: sink,
                request: request.clone(),
            };
            config
                .audit_notification_authorizer
                .as_ref()
                .is_some_and(|authorizer| fail_closed_authorize(|| authorizer(&context)))
        },
    )
    .await;
    pending.complete();
    Some(execution)
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
    let request = bacnet_services::audit::AuditNotificationRequest::decode(service_request)?;
    if request.notifications.len() > MAX_AUDIT_NOTIFICATIONS {
        return Err(Error::OutOfRange(format!(
            "{service} list exceeds {MAX_AUDIT_NOTIFICATIONS} items"
        )));
    }
    let sink = config.audit_notification_sink.ok_or_else(request_denied)?;
    if !authorize(sink, &request) {
        return Err(request_denied());
    }

    let mut db = db.write().await;
    handlers::handle_audit_notification(&mut db, sink, &request)
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
