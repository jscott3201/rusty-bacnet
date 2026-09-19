use std::sync::Mutex;

use bacnet_types::enums::Reliability;

/// Internal, object-instance-owned delivery health, independent of BACnet writes.
#[doc(hidden)]
#[derive(Default)]
pub struct AuditReporterStatus(Mutex<State>);

#[derive(Default)]
struct State {
    configured: bool,
    communication_failure: bool,
    failure_epoch: u64,
    auditing_failure_enabled: bool,
    auditing_failure_epoch: u64,
}

impl AuditReporterStatus {
    pub(super) fn set_auditing_failure_enabled(&self, enabled: bool) {
        let mut state = self.0.lock().unwrap();
        if state.auditing_failure_enabled && !enabled {
            state.auditing_failure_epoch = state.auditing_failure_epoch.saturating_add(1);
        }
        state.auditing_failure_enabled = enabled;
    }

    /// Instance-owned filter identity for memory-only resource-drop summaries.
    /// Disabling invalidates pending counts even if re-enabled before admission.
    #[doc(hidden)]
    pub fn auditing_failure_epoch(&self) -> Option<u64> {
        let state = self.0.lock().unwrap();
        (state.auditing_failure_enabled && state.auditing_failure_epoch != u64::MAX)
            .then_some(state.auditing_failure_epoch)
    }

    /// Update destination/configuration availability without hiding send failures.
    pub fn set_configured(&self, configured: bool) {
        self.0.lock().unwrap().configured = configured;
    }

    /// Snapshot failure identity before a delivery is admitted.
    pub fn begin_delivery(&self) -> u64 {
        self.0.lock().unwrap().failure_epoch
    }

    /// A success cannot clear a failure that happened after this send began.
    pub fn complete_delivery(&self, epoch: u64, success: bool) {
        let mut state = self.0.lock().unwrap();
        if !success {
            state.communication_failure = true;
            state.failure_epoch = state.failure_epoch.saturating_add(1);
        } else if epoch == state.failure_epoch && epoch != u64::MAX {
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
