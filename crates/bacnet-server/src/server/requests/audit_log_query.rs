//! An admitted AuditLogQuery, moved out of the general handler's body
//! (#2558): the query runs under the database's read guard, which is gone
//! before the acknowledgment is encoded.
use super::audit_reporter::{ReadAuditIntent, WriteAudit};
use super::other_services::ServiceCall;
use super::*;

/// Answer `call`'s AuditLogQuery, encoding the acknowledgment into `ack_buf`.
/// The reads it completed join `read_audits`, which a failed encoding empties.
pub(super) async fn response<T: TransportPort + 'static>(
    call: &ServiceCall<'_, T>,
    mut ack_buf: BytesMut,
    read_audits: &mut Vec<ReadAuditIntent>,
    audit: &WriteAudit<'_, T>,
) -> Apdu {
    let ServiceCall {
        services,
        req,
        invoke_id,
        service_choice,
        ..
    } = *call;
    let db = &services.db;
    // Query under the read guard, then release it before ACK
    // construction/encoding and the generic segmentation path.
    let query_result = {
        let db = db.read().await;
        handlers::handle_audit_log_query_observed(&db, &req.service_request, |target, result| {
            read_audits.extend(audit.completed_read_intent(&db, target, None, result));
        })
    };
    match query_result {
        Ok((audit_log, page)) => {
            let ack = bacnet_services::audit::AuditLogQueryAck {
                audit_log,
                records: page.records,
                no_more_items: page.no_more_items,
            };
            match ack.try_encode(&mut ack_buf) {
                Ok(()) => complex_ack(invoke_id, service_choice, ack_buf),
                Err(e) => {
                    // Execution alone is not a completed query response.
                    read_audits.clear();
                    bacnet_device_core::reply::error_apdu(invoke_id, service_choice, &e)
                }
            }
        }
        Err(e) => bacnet_device_core::reply::error_apdu(invoke_id, service_choice, &e),
    }
}
