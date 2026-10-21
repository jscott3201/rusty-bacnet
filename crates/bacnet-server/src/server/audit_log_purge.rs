//! The application's purge of an Audit Log (#1238).
//!
//! No peer can purge an Audit Log (Clause 12.64.11 keeps its Record_Count
//! read-only), so the server offers the purge to the application. It is
//! staged like a durable write ([`super::durable_writes`]): the commit runs
//! with the database guard dropped, and the log serves the purged state only
//! once storage holds it.

use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Purge an Audit Log: clear its records and append a BUFFER_PURGED
    /// status record, as [`AuditLogObject::purge`] describes.
    ///
    /// Peers have no way to do this, so it is the application's to call.
    /// The log builds the purged state and queues its commit; the server
    /// waits for the commit with the object database guard dropped, so other
    /// requests carry on, and the log serves the purged state only once it is
    /// durable. Changes to one log land one at a time, in the order they
    /// were staged: a notification batch whose commit is running lands first
    /// and is purged with the rest, and one that arrives while the purge
    /// commits waits for it and follows the purge record.
    ///
    /// # Errors
    ///
    /// - OBJECT / UNKNOWN_OBJECT when `oid` names no object, and OBJECT /
    ///   OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED when the object cannot be
    ///   purged (it is not a built-in Audit Log).
    /// - DEVICE / OPERATIONAL_PROBLEM without a valid local clock, or when
    ///   the commit fails; the log's writer logs the storage error.
    /// - The stopped-server error once [`stop`](Self::stop) has run.
    ///
    /// On any error the log is left as it was. A purge whose future is
    /// dropped before it finishes is handled as an abandoned write request
    /// is: the log keeps serving its state, drops the staged purge once it
    /// has waited long enough, and sets storage back to that state.
    ///
    /// [`AuditLogObject::purge`]: bacnet_objects::audit::AuditLogObject::purge
    pub async fn purge_audit_log(&self, oid: &ObjectIdentifier) -> Result<(), Error> {
        self.active_network()?;
        let staged =
            durable_writes::stage(&self.db, durable_writes::DurableTarget::purge(*oid)).await;
        let mut db = self.db.write().await;
        let result = match db.get_mut(oid) {
            None => Err(protocol_error(
                ErrorClass::OBJECT,
                ErrorCode::UNKNOWN_OBJECT,
            )),
            Some(object) => match object.durable_writes_internal() {
                Some(writes) => writes.commit_purge(),
                None => Err(protocol_error(
                    ErrorClass::OBJECT,
                    ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
                )),
            },
        };
        staged.release(&mut db);
        result
    }
}

fn protocol_error(class: ErrorClass, code: ErrorCode) -> Error {
    Error::Protocol { class, code }
}

#[cfg(test)]
#[path = "audit_log_purge_tests.rs"]
mod tests;
