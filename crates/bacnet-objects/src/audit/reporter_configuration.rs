//! Fallible local setters share the complete object-owned change boundary.
use super::*;
impl AuditReporterObject {
    /// Set Audit_Level; live admission failure leaves all state unchanged.
    pub fn set_audit_level(&mut self, level: AuditLevel) -> Result<(), Error> {
        let mut next = self.configuration_internal();
        next.audit_level = level;
        self.change_configuration(next, None)
    }
    /// Set Auditable_Operations atomically with any required notification.
    pub fn set_auditable_operations(
        &mut self,
        operations: AuditOperationFlags,
    ) -> Result<(), Error> {
        let mut next = self.configuration_internal();
        next.auditable_operations = operations;
        self.change_configuration(next, None)
    }
    /// Set the command-priority filter atomically.
    pub fn set_audit_priority_filter(&mut self, filter: BACnetPriorityFilter) -> Result<(), Error> {
        let mut next = self.configuration_internal();
        next.audit_priority_filter = filter;
        self.change_configuration(next, None)
    }
    /// Set confirmed delivery mode atomically.
    pub fn set_issue_confirmed_notifications(&mut self, confirmed: bool) -> Result<(), Error> {
        let mut next = self.configuration_internal();
        next.confirmed = confirmed;
        self.change_configuration(next, None)
    }
    /// Set optional selectors: absent means catch-all; present empty means no nominal targets.
    pub fn set_monitored_objects(
        &mut self,
        selectors: Option<Vec<BACnetObjectSelector>>,
    ) -> Result<(), Error> {
        let mut next = self.configuration_internal();
        next.monitored_objects = selectors;
        self.change_configuration(next, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audit_reporter_configuration_aba_invalidates_delivery_authority_and_failed_updates_do_not() {
        let mut reporter = AuditReporterObject::new(1, "AR").unwrap();
        reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
        let status = reporter.status_internal();
        status.set_configured(true);
        let original = status.begin_delivery();
        assert!(reporter
            .configure_audit_reporter_internal(
                AuditLevel::DEFAULT,
                AuditOperationFlags::empty(),
                true,
                Some(vec![]),
                BACnetPriorityFilter::empty(),
                None
            )
            .is_err());
        assert_eq!(
            status.begin_delivery(),
            original,
            "invalid full configuration is atomic"
        );
        reporter.set_issue_confirmed_notifications(true).unwrap();
        reporter.set_issue_confirmed_notifications(false).unwrap();
        let current = status.begin_delivery();
        assert_ne!(original, current);
        status.complete_delivery(original, false);
        assert_eq!(reporter.reliability(), Reliability::NO_FAULT_DETECTED);
        status.complete_delivery(current, false);
        status.complete_delivery(original, true);
        assert_eq!(reporter.reliability(), Reliability::COMMUNICATION_FAILURE);
        status.complete_delivery(status.begin_delivery(), true);
        assert_eq!(reporter.reliability(), Reliability::NO_FAULT_DETECTED);
    }
}
