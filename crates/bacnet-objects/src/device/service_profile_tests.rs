use super::*;
use bacnet_types::error::RemoteError;

#[test]
fn standalone_device_cov_properties_follow_declared_service_profile() {
    for (services, active, multiple) in [
        (vec![], false, false),
        (vec![ServiceSupported::SUBSCRIBE_COV], true, false),
        (vec![ServiceSupported::SUBSCRIBE_COV_PROPERTY], true, false),
        (
            vec![ServiceSupported::SUBSCRIBE_COV_PROPERTY_MULTIPLE],
            false,
            true,
        ),
        (EXECUTED_SERVICES.to_vec(), true, true),
    ] {
        let mut object = DeviceObject::new(DeviceConfig::default()).unwrap();
        object.set_services_supported(&services);
        for (property, present) in [
            (PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, active),
            (
                PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
                multiple,
            ),
        ] {
            assert_eq!(object.property_list().contains(&property), present);
            assert_eq!(
                object
                    .property_metadata()
                    .iter()
                    .any(|row| row.property_identifier == property),
                present
            );
            let result = object.read_property(property, None);
            if present {
                assert_eq!(result.unwrap(), PropertyValue::ApplicationData(vec![]));
            } else {
                assert!(
                    matches!(result, Err(Error::Remote(RemoteError::Error { class, code })) if class == ErrorClass::PROPERTY && code == ErrorCode::UNKNOWN_PROPERTY)
                );
            }
            let PropertyValue::List(list) = object
                .read_property(PropertyIdentifier::PROPERTY_LIST, None)
                .unwrap()
            else {
                panic!("property list");
            };
            assert_eq!(
                list.contains(&PropertyValue::Enumerated(property.to_raw())),
                present
            );
        }
    }
}
