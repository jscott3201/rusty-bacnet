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
    if service_request.len() > MAX_AUDIT_NOTIFICATION_BYTES {
        return Some(Err(Error::OutOfRange(format!(
            "ConfirmedAuditNotification payload exceeds {MAX_AUDIT_NOTIFICATION_BYTES} bytes"
        ))));
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
    let decoded = bacnet_services::audit::AuditNotificationRequest::decode(service_request);
    let execution = match decoded {
        Err(error) => Err(error),
        Ok(request) if request.notifications.len() > MAX_AUDIT_NOTIFICATIONS => {
            Err(Error::OutOfRange(format!(
                "ConfirmedAuditNotification list exceeds {MAX_AUDIT_NOTIFICATIONS} items"
            )))
        }
        Ok(request) => match config.audit_notification_sink {
            None => Err(request_denied()),
            Some(sink) => {
                let context = AuditNotificationAuthorizationContext {
                    source_mac: MacAddr::from_slice(source_mac),
                    source_network: source_network.cloned(),
                    invoke_id,
                    audit_log_sink: sink,
                    request: request.clone(),
                };
                let authorized =
                    config
                        .audit_notification_authorizer
                        .as_ref()
                        .is_some_and(|authorizer| {
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                authorizer(&context)
                            }))
                            .unwrap_or(false)
                        });
                if !authorized {
                    Err(request_denied())
                } else {
                    let mut db = db.write().await;
                    handlers::handle_confirmed_audit_notification(&mut db, sink, &request)
                }
            }
        },
    };
    pending.complete();
    Some(execution)
}

fn request_denied() -> Error {
    Error::Protocol {
        class: ErrorClass::SERVICES.to_raw() as u32,
        code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
    }
}
