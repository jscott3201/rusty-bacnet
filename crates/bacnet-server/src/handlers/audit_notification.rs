use bacnet_objects::database::ObjectDatabase;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// Store one decoded and authorized notification batch in its explicit sink.
///
/// Persistence is synchronous under the database writer in this bounded
/// receiver foundation. That limits availability to the configured backend's
/// commit latency, but keeps the durable commit and memory apply atomic.
pub fn handle_confirmed_audit_notification(
    db: &mut ObjectDatabase,
    sink: ObjectIdentifier,
    request: &AuditNotificationRequest,
) -> Result<(), Error> {
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
    storage.store_notifications(&request.notifications, apdu_timeout_ms)
}

fn configured_apdu_timeout(db: &ObjectDatabase) -> Result<u32, Error> {
    let devices: Vec<_> = db
        .list_objects()
        .into_iter()
        .filter(|oid| oid.object_type() == ObjectType::DEVICE)
        .collect();
    let [device_oid] = devices.as_slice() else {
        return Err(operational_problem());
    };
    let Some(device) = db.get(device_oid) else {
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
    Error::Protocol {
        class: ErrorClass::SERVICES.to_raw() as u32,
        code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
    }
}

fn operational_problem() -> Error {
    Error::Protocol {
        class: ErrorClass::DEVICE.to_raw() as u32,
        code: ErrorCode::OPERATIONAL_PROBLEM.to_raw() as u32,
    }
}
