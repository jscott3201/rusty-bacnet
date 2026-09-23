//! Locally managed Reporter configuration and mutation-owned generations.
use super::*;

impl AuditReporterObject {
    /// Set the locally managed audit level.
    pub fn set_audit_level(&mut self, level: AuditLevel) -> Result<(), Error> {
        if level == AuditLevel::DEFAULT {
            return Err(Error::OutOfRange(
                "Audit Reporter audit level must not be DEFAULT".into(),
            ));
        }
        if self.audit_level != level {
            self.status.configuration_changed();
        }
        self.audit_level = level;
        self.update_auditing_failure_filter();
        Ok(())
    }

    /// Set the locally managed operation filter.
    pub fn set_auditable_operations(&mut self, operations: AuditOperationFlags) {
        if self.auditable_operations != operations {
            self.status.configuration_changed();
        }
        self.auditable_operations = operations;
        self.update_auditing_failure_filter();
    }

    fn update_auditing_failure_filter(&self) {
        self.status.set_auditing_failure_enabled(
            self.audit_level != AuditLevel::NONE
                && self
                    .auditable_operations
                    .contains(bacnet_types::enums::AuditOperation::AUDITING_FAILURE),
        );
    }

    /// Set the locally managed command-priority filter.
    pub fn set_audit_priority_filter(&mut self, filter: BACnetPriorityFilter) {
        if self.audit_priority_filter != filter {
            self.status.configuration_changed();
        }
        self.audit_priority_filter = filter;
    }

    /// Select confirmed or unconfirmed target audit notifications.
    pub fn set_issue_confirmed_notifications(&mut self, confirmed: bool) {
        if self.issue_confirmed_notifications != confirmed {
            self.status.configuration_changed();
        }
        self.issue_confirmed_notifications = confirmed;
    }

    /// Configure the optional Monitored_Objects array locally (never over BACnet).
    ///
    /// `None` removes the property and preserves catch-all target reporting.
    /// `Some(vec![])` or all NULL entries selects no ordinary targets. Object
    /// identifiers match exactly; object types match every instance of that type.
    /// Duplicates do not cause duplicate reports. Enabled external Reporter writes
    /// bypass this selection. This does not enable multi-Reporter arbitration.
    pub fn set_monitored_objects(&mut self, selectors: Option<Vec<BACnetObjectSelector>>) {
        if self.monitored_objects != selectors {
            self.status.configuration_changed();
        }
        self.monitored_objects = selectors;
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
                BACnetPriorityFilter::empty()
            )
            .is_err());
        assert_eq!(
            status.begin_delivery(),
            original,
            "invalid full configuration is atomic"
        );
        reporter.set_issue_confirmed_notifications(true);
        reporter.set_issue_confirmed_notifications(false);
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
