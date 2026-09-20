use bacnet_types::constructed::BACnetDeviceObjectReference;

use super::{AuditLogObject, AuditReporterStatus};

/// Instance-owned, memory-only forwarding configuration and delivery health.
/// This is not a pending-send ledger and does not change the durable snapshot.
#[doc(hidden)]
pub struct AuditLogForwarding {
    parent: BACnetDeviceObjectReference,
    status: AuditReporterStatus,
}

impl AuditLogForwarding {
    /// Locally configured parent; validity and route availability are checked by the server.
    pub fn parent(&self) -> &BACnetDeviceObjectReference {
        &self.parent
    }

    /// Reuse the instance-owned delivery health rules without Reporter production.
    pub fn status(&self) -> &AuditReporterStatus {
        &self.status
    }
}

impl AuditLogObject {
    /// Configure immediate, best-effort confirmed forwarding to one parent.
    ///
    /// Only the server's explicit `audit_notification_sink` forwards accepted
    /// notification batches. No backlog is replayed, no records are deleted, and
    /// restart can lose forwarding progress. Reapply this local configuration on
    /// restart; it is not part of the persisted snapshot. `None` removes the
    /// forwarding properties. Invalid/unresolved parents remain visible as
    /// CONFIGURATION_ERROR and cause no forwarding I/O.
    pub fn set_member_of(&mut self, parent: Option<BACnetDeviceObjectReference>) {
        self.forwarding = parent.map(|parent| {
            std::sync::Arc::new(AuditLogForwarding {
                parent,
                status: AuditReporterStatus::default(),
            })
        });
    }
}
