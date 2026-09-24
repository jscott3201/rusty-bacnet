use super::*;
use crate::traits::BACnetObject;

#[test]
fn reporter_delay_pair_is_optional_typed_scalar_and_nonnullable_relinquishment() {
    let mut reporter = AuditReporterObject::new(1, "delay").unwrap();
    for property in [
        PropertyIdentifier::MAXIMUM_SEND_DELAY,
        PropertyIdentifier::SEND_NOW,
    ] {
        assert!(!reporter.property_list().contains(&property));
        assert!(reporter.read_property(property, None).is_err());
        assert!(reporter
            .write_property(property, None, PropertyValue::Null, None)
            .is_err());
    }
    assert!(AuditSendDelay::new(3601).is_err());
    for seconds in [0, 1, 3600] {
        reporter
            .set_maximum_send_delay(Some(AuditSendDelay::new(seconds).unwrap()))
            .unwrap();
        assert_eq!(
            reporter
                .read_property(PropertyIdentifier::MAXIMUM_SEND_DELAY, None)
                .unwrap(),
            PropertyValue::Unsigned(u64::from(seconds))
        );
        for property in [
            PropertyIdentifier::MAXIMUM_SEND_DELAY,
            PropertyIdentifier::SEND_NOW,
        ] {
            assert!(reporter.property_list().contains(&property));
            let row = *reporter
                .property_metadata()
                .iter()
                .find(|row| row.property_identifier == property)
                .unwrap();
            assert_eq!(
                row.write_capability,
                crate::property_metadata::PropertyWriteCapability::Always
            );
            for index in [0, 1] {
                assert!(
                    matches!(reporter.write_property(property,Some(index),PropertyValue::Null,None),Err(Error::Protocol{code,..}) if code==ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32)
                );
                assert!(reporter.read_property(property, Some(index)).is_err());
            }
            reporter
                .write_property(property, None, PropertyValue::Null, None)
                .unwrap();
            assert!(reporter
                .write_property(
                    property,
                    None,
                    PropertyValue::CharacterString("bad".into()),
                    None
                )
                .is_err());
        }
        reporter.set_send_now(true).unwrap();
        reporter.set_send_now(true).unwrap();
        assert_eq!(
            reporter
                .read_property(PropertyIdentifier::SEND_NOW, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
    }
    reporter.set_maximum_send_delay(None).unwrap();
    assert!(!reporter
        .property_list()
        .contains(&PropertyIdentifier::SEND_NOW));
}
