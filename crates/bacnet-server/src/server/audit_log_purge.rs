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
    /// dropped before its task holds the database guard to commit changes
    /// nothing the log serves: the staged purge is let go as soon as the
    /// guard is free, and storage is set back to the state the log serves
    /// (#2493). One dropped after that may still purge the log, and the drop
    /// does not say which: a call dropped part way is as
    /// [`write_local`](Self::write_local)'s Cancellation section describes (#2772).
    ///
    /// The future must be awaited inside a Tokio runtime, which the release
    /// task needs: outside one it fails with [`Error::Unsupported`] before
    /// anything is staged or looked up, so an unknown object gets that error
    /// too, not UNKNOWN_OBJECT.
    ///
    /// [`AuditLogObject::purge`]: bacnet_objects::audit::AuditLogObject::purge
    pub async fn purge_audit_log(&self, oid: &ObjectIdentifier) -> Result<(), Error> {
        super::local_writes::local_runtime()?;
        self.active_network()?;
        let oid = *oid;
        let (db, releases) = (
            Arc::clone(&self.db),
            Arc::clone(self.request_tasks.stage_releases()),
        );
        let take = async move {
            let staged = durable_writes::stage_held(
                &db,
                &releases,
                durable_writes::DurableTarget::purge(oid),
            )
            .await;
            // The guard first, so a caller gone by now lets the stage go at once.
            (db.write_owned().await, staged)
        };
        let purge = move |(mut db, staged): super::local_writes::Held| {
            let result = match db.get_mut(&oid) {
                None => Err(protocol_error(
                    ErrorClass::OBJECT,
                    ErrorCode::UNKNOWN_OBJECT,
                )),
                Some(object) => match object.durable_writes() {
                    Some(writes) => writes.commit_purge(),
                    None => Err(protocol_error(
                        ErrorClass::OBJECT,
                        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
                    )),
                },
            };
            staged.release(&mut db);
            result
        };
        // Neither the stage nor the guard is waited for in the caller's
        // task (#2772).
        self.request_tasks
            .for_caller(take, purge)
            .await
            // `None`: no change; its runtime dropped the task before it committed.
            .unwrap_or_else(|| Err(broadcaster::stopped()))
    }
}

fn protocol_error(class: ErrorClass, code: ErrorCode) -> Error {
    Error::protocol(class, code)
}

#[cfg(test)]
#[path = "audit_log_purge_tests.rs"]
mod tests;
