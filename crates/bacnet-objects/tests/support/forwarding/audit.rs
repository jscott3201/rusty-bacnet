//! The probe's Audit policy and Audit Log, logged under `AuditPolicy::` and
//! `AuditLog::`: every method answers unlike its default, and the borrowed
//! answers are objects the probe owns.

use super::{Error, Probe};
use bacnet_objects::audit::{
    AuditLogForwarding, AuditLogNotificationSink, AuditLogStorage, AuditPolicyAuthority,
    ObjectAuditPolicy,
};
use bacnet_objects::traits::{AuditLog, AuditPolicy, BACnetObject};
use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::AuditLevel;
use std::sync::Arc;

impl AuditPolicy for Probe {
    fn object_policy(&self) -> ObjectAuditPolicy {
        self.called("AuditPolicy::object_policy", ());
        ObjectAuditPolicy {
            level: Some(AuditLevel::AUDIT_ALL),
            ..ObjectAuditPolicy::default()
        }
    }
    fn policy_authority(&mut self) -> Option<AuditPolicyAuthority<'_>> {
        self.called("AuditPolicy::policy_authority", ());
        self.value.audit_policy_mut()?.policy_authority()
    }
}

impl AuditLog for Probe {
    fn storage(&self) -> &dyn AuditLogStorage {
        self.called("AuditLog::storage", ());
        &self.audit_log
    }
    fn forwarding(&self) -> Option<Arc<AuditLogForwarding>> {
        self.called("AuditLog::forwarding", ());
        self.audit_log.audit_log()?.forwarding()
    }
    fn set_parent(&mut self, parent: BACnetDeviceObjectReference) -> Result<(), Error> {
        self.called("AuditLog::set_parent", (parent,));
        Ok(())
    }
    fn notification_sink(&mut self) -> Option<&mut dyn AuditLogNotificationSink> {
        self.called("AuditLog::notification_sink", ());
        Some(&mut self.audit_log)
    }
}
