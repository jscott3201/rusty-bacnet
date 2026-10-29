use bacnet_objects::audit::{
    AuditBatchStage, CompletedAuditReceipt, ConfirmedAuditNotificationOutcome, StagedAuditBatch,
};
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// Store one decoded and authorized notification batch in its explicit sink.
///
/// The commit runs while the caller holds the database, and the batch is in
/// memory only once it is durable. The bundled server stages the commit
/// instead, so it runs without the database guard (#1270).
pub fn handle_audit_notification(
    db: &mut ObjectDatabase,
    sink: ObjectIdentifier,
    request: &AuditNotificationRequest,
) -> Result<(), Error> {
    handle_audit_notification_with_change(db, sink, request).map(|_| ())
}

pub(crate) fn handle_audit_notification_with_change(
    db: &mut ObjectDatabase,
    sink: ObjectIdentifier,
    request: &AuditNotificationRequest,
) -> Result<bool, Error> {
    if sink.object_type() != ObjectType::AUDIT_LOG {
        return Err(service_request_denied());
    }
    {
        let object = db.get_mut(&sink).ok_or_else(service_request_denied)?;
        let storage = object
            .audit_log_notification_sink_internal()
            .ok_or_else(service_request_denied)?;
        if !storage.notification_logging_enabled() {
            return Err(service_request_denied());
        }
    }
    let apdu_timeout_ms = configured_apdu_timeout(db)?;
    let object = db
        .get_mut(&sink)
        .expect("sink existence was checked before Device timeout lookup");
    let storage = object
        .audit_log_notification_sink_internal()
        .expect("sink capability was checked before Device timeout lookup");
    storage.store_notifications_with_change(&request.notifications, apdu_timeout_ms)
}

/// Store one decoded and authorized confirmed notification batch.
///
/// Retained as a compatibility alias for the original confirmed-only receiver
/// API; both inbound AuditNotification forms use the same atomic storage owner.
pub fn handle_confirmed_audit_notification(
    db: &mut ObjectDatabase,
    sink: ObjectIdentifier,
    request: &AuditNotificationRequest,
) -> Result<(), Error> {
    handle_audit_notification(db, sink, request)
}

/// Check one sink-owned completed confirmed receipt without mutation.
pub(crate) fn has_completed_confirmed_audit_receipt(
    db: &mut ObjectDatabase,
    sink: ObjectIdentifier,
    key: &[u8],
    now_unix_millis: u64,
) -> Result<bool, Error> {
    if sink.object_type() != ObjectType::AUDIT_LOG {
        return Err(service_request_denied());
    }
    let object = db.get_mut(&sink).ok_or_else(service_request_denied)?;
    let storage = object
        .audit_log_notification_sink_internal()
        .ok_or_else(service_request_denied)?;
    storage.has_completed_confirmed_receipt(key, now_unix_millis)
}

/// Stage one decoded and authorized batch in its sink (#1270): a confirmed
/// batch with its completed receipt, both stored atomically, or an
/// unconfirmed one without. The commit is queued so the caller can await it
/// without the guard and then [finish](finish_audit_notification) the batch.
pub(crate) fn stage_audit_notification(
    db: &mut ObjectDatabase,
    sink: ObjectIdentifier,
    request: &AuditNotificationRequest,
    receipt: Option<CompletedAuditReceipt>,
) -> Result<AuditBatchStage, Error> {
    if sink.object_type() != ObjectType::AUDIT_LOG {
        return Err(service_request_denied());
    }
    {
        let object = db.get_mut(&sink).ok_or_else(service_request_denied)?;
        let storage = object
            .audit_log_notification_sink_internal()
            .ok_or_else(service_request_denied)?;
        if !storage.notification_logging_enabled() {
            return Err(service_request_denied());
        }
    }
    let apdu_timeout_ms = configured_apdu_timeout(db)?;
    let storage = db
        .get_mut(&sink)
        .and_then(|object| object.audit_log_notification_sink_internal())
        .expect("sink capability was checked before Device timeout lookup");
    storage.stage_notification_batch(&request.notifications, apdu_timeout_ms, receipt)
}

/// Take a batch [`stage_audit_notification`] staged, once its commit has run.
///
/// A batch whose commit failed is refused with DEVICE / OPERATIONAL_PROBLEM
/// (#1366), as a Log_Enable or Buffer_Size write or a purge whose commit
/// fails is (#1238). The log's writer has already logged the storage error,
/// which is no protocol error to hand the sender.
pub(crate) fn finish_audit_notification(
    db: &mut ObjectDatabase,
    sink: ObjectIdentifier,
    staged: StagedAuditBatch,
) -> Result<(ConfirmedAuditNotificationOutcome, bool), Error> {
    db.get_mut(&sink)
        .and_then(|object| object.audit_log_notification_sink_internal())
        .ok_or_else(service_request_denied)?
        .finish_notification_batch(staged)
        .map_err(|_| operational_problem())
}

/// The [selected Device](ObjectDatabase::selected_device)'s APDU_Timeout.
fn configured_apdu_timeout(db: &ObjectDatabase) -> Result<u32, Error> {
    let Some(device) = db.selected_device().and_then(|oid| db.get(&oid)) else {
        return Err(operational_problem());
    };
    let Ok(PropertyValue::Unsigned(timeout)) =
        device.read_property(PropertyIdentifier::APDU_TIMEOUT, None)
    else {
        return Err(operational_problem());
    };
    u32::try_from(timeout)
        .ok()
        .filter(|timeout| *timeout != 0)
        .ok_or_else(operational_problem)
}

fn service_request_denied() -> Error {
    Error::protocol(ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED)
}

fn operational_problem() -> Error {
    Error::protocol(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM)
}
