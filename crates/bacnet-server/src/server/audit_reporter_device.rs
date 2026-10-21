//! Audit records for operations on the device as a whole (Clause 19.6,
//! Table 19-5): DeviceCommunicationControl (#1387) and ReinitializeDevice
//! (#1564).
use super::*;

impl<T: TransportPort + 'static> WriteAudit<'_, T> {
    /// Report an operation on the whole device that has taken effect, named
    /// by `operation`: DEVICE_DISABLE_COMM or DEVICE_ENABLE_COMM for a
    /// DeviceCommunicationControl change, DEVICE_RESET, DEVICE_BACKUP or
    /// DEVICE_RESTORE for an accepted ReinitializeDevice request or a reset
    /// the application reports.
    ///
    /// The operation is on the device, so the record goes through the
    /// Reporter that monitors the local Device object, filtered by that
    /// Device's audit policy, with its Maximum_Send_Delay, and its resource
    /// losses summarized as AUDITING_FAILURE, as a CREATE is. Table 19-5
    /// gives these operations no target object, property or value, so the
    /// record names only this Device as its target. Its source and invoke ID
    /// are the requester's, or this device's own with no invoke ID for a
    /// change no request caused ([`WriteAudit::local`]). Only an operation
    /// carried out is reported, so the record has no Result.
    ///
    /// Called under the database guard, once the operation has been carried
    /// out.
    pub(in crate::server) fn device_operation(
        &mut self,
        db: &mut ObjectDatabase,
        operation: AuditOperation,
    ) {
        let Some(device) = db.local_device().identifier() else {
            return;
        };
        self.before_lifecycle(db, operation, Some(device), ObjectType::DEVICE, true);
        if let Some(pending) = &mut self.pending {
            pending.notification.target_object = None;
        }
        self.lifecycle_completed(db, &Ok(()));
    }
}
