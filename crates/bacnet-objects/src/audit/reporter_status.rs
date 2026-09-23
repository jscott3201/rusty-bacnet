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
    configured: bool,
    communication_failure: bool,
    failure_epoch: u64,
    auditing_failure_enabled: bool,
    configuration_epoch: u64,
}

impl AuditReporterStatus {
    pub(super) fn set_auditing_failure_enabled(&self, enabled: bool) {
        let mut state = self.0.lock().unwrap();
        state.auditing_failure_enabled = enabled;
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
        (state.auditing_failure_enabled && state.configuration_epoch != u64::MAX)
            .then_some(state.configuration_epoch)
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
        if !state.configured {
            Reliability::CONFIGURATION_ERROR
        } else if state.communication_failure {
            Reliability::COMMUNICATION_FAILURE
        } else {
            Reliability::NO_FAULT_DETECTED
        }
    }
}
