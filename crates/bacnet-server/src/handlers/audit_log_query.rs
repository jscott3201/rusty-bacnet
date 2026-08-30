//! AuditLogQuery service handler (Clause 13.19).

use bacnet_objects::audit::AuditLogQueryPage;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::audit::AuditLogQueryRequest;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

fn unknown_object() -> Error {
    Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    }
}

fn optional_functionality_not_supported() -> Error {
    Error::Protocol {
        class: ErrorClass::SERVICES.to_raw() as u32,
        code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
    }
}

/// Decode and execute one AuditLogQuery against the object's retained state.
///
/// Decoding completes before object lookup or capability access. The returned
/// page owns its records, allowing the dispatcher to release the database read
/// guard before constructing and encoding the acknowledgment.
pub fn handle_audit_log_query(
    db: &ObjectDatabase,
    service_data: &[u8],
) -> Result<(ObjectIdentifier, AuditLogQueryPage), Error> {
    let request = AuditLogQueryRequest::decode(service_data)?;

    // Clause 13.19 specifies OBJECT / UNKNOWN_OBJECT for an Audit Log that
    // does not exist. A non-Audit identifier cannot designate the requested
    // Audit Log and follows the same public result instead of exposing an
    // object-type distinction.
    if request.audit_log.object_type() != ObjectType::AUDIT_LOG {
        return Err(unknown_object());
    }
    let object = db.get(&request.audit_log).ok_or_else(unknown_object)?;
    let storage = object
        .audit_log_storage_internal()
        .ok_or_else(optional_functionality_not_supported)?;
    let page = storage.query(
        &request.query_parameters,
        request.start_at_sequence_number,
        request.requested_count,
    );
    Ok((request.audit_log, page))
}
