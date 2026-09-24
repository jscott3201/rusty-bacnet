use super::AuditReporterConfiguration;
use std::sync::Mutex;

use bacnet_types::enums::Reliability;

/// Internal, object-instance-owned delivery health, independent of BACnet writes.
#[doc(hidden)]
#[derive(Default)]
pub struct AuditReporterStatus(Mutex<State>);

/// Internal completion authority for one Reporter configuration and health epoch.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditDeliveryToken {
    configuration: u64,
    failure: u64,
}

#[derive(Default)]
struct State {
    configuration: AuditReporterConfiguration,
    overlap: bool,
    configured: bool,
    communication_failure: bool,
    failure_epoch: u64,
    configuration_epoch: u64,
}

impl AuditReporterStatus {
    pub fn configuration(&self) -> AuditReporterConfiguration {
        self.0.lock().unwrap().configuration.clone()
    }
    pub(super) fn confirmed(&self) -> bool {
        self.0.lock().unwrap().configuration.confirmed
    }
    /// Prepare all consequences before committing the complete configuration once.
    #[doc(hidden)]
    pub fn commit_configuration<R>(
        &self,
        next: AuditReporterConfiguration,
        prepare: impl FnOnce(AuditDeliveryToken) -> Result<R, bacnet_types::error::Error>,
    ) -> Result<R, bacnet_types::error::Error> {
        let mut state = self.0.lock().unwrap();
        if state.configuration == next {
            return prepare(AuditDeliveryToken {
                configuration: state.configuration_epoch,
                failure: state.failure_epoch,
            });
        }
        let configuration = state
            .configuration_epoch
            .checked_add(1)
            .filter(|v| *v != u64::MAX)
            .ok_or_else(|| {
                bacnet_types::error::Error::Encoding("audit generation exhausted".into())
            })?;
        let result = prepare(AuditDeliveryToken {
            configuration,
            failure: state.failure_epoch,
        })?;
        state.configuration = next;
        state.configuration_epoch = configuration;
        Ok(result)
    }
    #[doc(hidden)]
    pub fn set_overlap(&self, overlap: bool) {
        self.0.lock().unwrap().overlap = overlap;
    }
    /// Reserve the next configuration and both delivery tokens atomically with
    /// a synchronous recipient commit. A failed preparation leaves health and
    /// configuration untouched. The closure must not reenter status or a queue.
    #[doc(hidden)]
    pub fn commit_recipient_change<R>(
        &self,
        commit: impl FnOnce(bool, AuditDeliveryToken) -> Result<R, bacnet_types::error::Error>,
    ) -> Result<R, bacnet_types::error::Error> {
        let mut state = self.0.lock().unwrap();
        let configuration = state
            .configuration_epoch
            .checked_add(1)
            .filter(|epoch| *epoch != u64::MAX)
            .ok_or_else(|| {
                bacnet_types::error::Error::Encoding("audit generation exhausted".into())
            })?;
        let token = AuditDeliveryToken {
            configuration,
            failure: state.failure_epoch,
        };
        let result = commit(state.configuration.confirmed, token)?;
        state.configuration_epoch = configuration;
        state.configured = true;
        state.communication_failure = false;
        Ok(result)
    }

    /// Invalidate snapshots on actual configuration or database membership changes.
    #[doc(hidden)]
    pub(crate) fn configuration_changed(&self) {
        let mut state = self.0.lock().unwrap();
        state.configuration_epoch = state.configuration_epoch.saturating_add(1);
    }

    /// Instance-owned filter identity for memory-only resource-drop summaries.
    /// Any configuration change invalidates pending counts, including A-to-B-to-A.
    #[doc(hidden)]
    pub fn auditing_failure_epoch(&self) -> Option<u64> {
        let state = self.0.lock().unwrap();
        ((state.configuration.enabled()
            && state
                .configuration
                .auditable_operations
                .contains(bacnet_types::enums::AuditOperation::AUDITING_FAILURE))
            && state.configuration_epoch != u64::MAX)
            .then_some(state.configuration_epoch)
    }

    /// Fence all previous recipient snapshots and restore route availability.
    #[doc(hidden)]
    pub fn recipient_changed_internal(&self) {
        let mut state = self.0.lock().unwrap();
        state.configuration_epoch = state.configuration_epoch.saturating_add(1);
        state.configured = true;
        state.communication_failure = false;
    }

    /// Update destination/configuration availability without hiding send failures.
    pub fn set_configured(&self, configured: bool) {
        self.0.lock().unwrap().configured = configured;
    }

    /// Snapshot failure identity before a delivery is admitted.
    pub fn begin_delivery(&self) -> AuditDeliveryToken {
        let state = self.0.lock().unwrap();
        AuditDeliveryToken {
            configuration: state.configuration_epoch,
            failure: state.failure_epoch,
        }
    }

    /// Admit summary health only for the expected enabled configuration.
    /// Validation and failure-epoch capture share one lock, so an old summary
    /// cannot acquire authority for a configuration that replaced its context.
    #[doc(hidden)]
    pub fn begin_auditing_failure_delivery(
        &self,
        expected_configuration: u64,
    ) -> Option<AuditDeliveryToken> {
        let state = self.0.lock().unwrap();
        ((state.configuration.enabled()
            && state
                .configuration
                .auditable_operations
                .contains(bacnet_types::enums::AuditOperation::AUDITING_FAILURE))
            && state.configuration_epoch == expected_configuration
            && state.configuration_epoch != u64::MAX)
            .then_some(AuditDeliveryToken {
                configuration: state.configuration_epoch,
                failure: state.failure_epoch,
            })
    }

    /// A success cannot clear a failure that happened after this send began.
    pub fn complete_delivery(&self, epoch: AuditDeliveryToken, success: bool) {
        let mut state = self.0.lock().unwrap();
        if epoch.configuration != state.configuration_epoch || epoch.configuration == u64::MAX {
            return;
        }
        if !success {
            state.communication_failure = true;
            state.failure_epoch = state.failure_epoch.saturating_add(1);
        } else if epoch.failure == state.failure_epoch && epoch.failure != u64::MAX {
            state.communication_failure = false;
        }
    }

    pub(super) fn reliability(&self) -> Reliability {
        let state = self.0.lock().unwrap();
        if !state.configured || state.overlap {
            Reliability::CONFIGURATION_ERROR
        } else if state.communication_failure {
            Reliability::COMMUNICATION_FAILURE
        } else {
            Reliability::NO_FAULT_DETECTED
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::AuditReporterObject;
    use bacnet_types::{
        bitstring::AuditOperationFlags,
        enums::{AuditLevel, AuditOperation},
    };

    fn reporter() -> AuditReporterObject {
        let mut reporter = AuditReporterObject::new(1, "Reporter").unwrap();
        reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
        let mut operations = AuditOperationFlags::empty();
        operations.insert(AuditOperation::AUDITING_FAILURE);
        reporter.set_auditable_operations(operations).unwrap();
        reporter.status_internal().set_configured(true);
        reporter
    }

    #[test]
    fn auditing_failure_health_admission_rejects_mismatch_disabled_and_aba() {
        let mut reporter = reporter();
        let status = reporter.status_internal();
        let expected = status.auditing_failure_epoch().unwrap();
        assert!(status
            .begin_auditing_failure_delivery(expected + 1)
            .is_none());
        // The caller observed an enabled old batch, then configuration changed
        // before completion admission. Equal final settings must not revive it.
        let old_token = status.begin_auditing_failure_delivery(expected).unwrap();
        reporter.set_issue_confirmed_notifications(true).unwrap();
        reporter.set_issue_confirmed_notifications(false).unwrap();
        assert!(status.begin_auditing_failure_delivery(expected).is_none());
        status.complete_delivery(old_token, false);
        assert_eq!(status.reliability(), Reliability::NO_FAULT_DETECTED);
        let current = status.auditing_failure_epoch().unwrap();
        status.complete_delivery(
            status.begin_auditing_failure_delivery(current).unwrap(),
            false,
        );
        status.complete_delivery(old_token, true);
        assert_eq!(status.reliability(), Reliability::COMMUNICATION_FAILURE);
        reporter.set_audit_level(AuditLevel::NONE).unwrap();
        assert!(status.begin_auditing_failure_delivery(current).is_none());
        assert!(AuditReporterStatus::default()
            .begin_auditing_failure_delivery(0)
            .is_none());
    }

    #[test]
    fn auditing_failure_health_admission_captures_current_failure_epoch_for_recovery() {
        let reporter = reporter();
        let status = reporter.status_internal();
        let expected = status.auditing_failure_epoch().unwrap();
        let early = status.begin_auditing_failure_delivery(expected).unwrap();
        status.complete_delivery(status.begin_delivery(), false);
        // A summary admitted now may recover the loss that prompted it, but
        // the already-admitted earlier summary cannot clear that newer failure.
        let recovery = status.begin_auditing_failure_delivery(expected).unwrap();
        status.complete_delivery(early, true);
        assert_eq!(status.reliability(), Reliability::COMMUNICATION_FAILURE);
        status.complete_delivery(recovery, true);
        assert_eq!(status.reliability(), Reliability::NO_FAULT_DETECTED);
        status.complete_delivery(recovery, false);
        status.complete_delivery(recovery, true);
        assert_eq!(status.reliability(), Reliability::COMMUNICATION_FAILURE);
        status.complete_delivery(
            status.begin_auditing_failure_delivery(expected).unwrap(),
            true,
        );
        assert_eq!(status.reliability(), Reliability::NO_FAULT_DETECTED);
    }
}
