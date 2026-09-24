use super::*;

#[test]
fn ordinary_reporter_has_no_source_role_even_after_local_configuration() {
    let mut reporter = AuditReporterObject::new(1, "Reporter").unwrap();
    for level in [AuditLevel::NONE, AuditLevel::AUDIT_ALL] {
        reporter
            .configure_audit_reporter_internal(
                level,
                AuditOperationFlags::from_bits(0xff).unwrap(),
                true,
                Some(vec![]),
                BACnetPriorityFilter::empty(),
                None,
            )
            .unwrap();
        assert_eq!(
            reporter
                .read_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        assert!(reporter.is_deleteable());
        assert!(!reporter.is_createable());
        assert!(!reporter.is_writable_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER));
        assert!(matches!(reporter.write_property(
            PropertyIdentifier::AUDIT_SOURCE_REPORTER, None, PropertyValue::Boolean(true), None,
        ), Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32));
        assert_eq!(
            reporter
                .read_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
    }
}
