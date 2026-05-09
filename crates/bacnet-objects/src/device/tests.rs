use super::*;

fn make_device() -> DeviceObject {
    DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "Test Device".into(),
        ..DeviceConfig::default()
    })
    .unwrap()
}

#[test]
fn read_object_identifier() {
    let dev = make_device();
    let val = dev
        .read_property(PropertyIdentifier::OBJECT_IDENTIFIER, None)
        .unwrap();
    let expected_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    assert_eq!(val, PropertyValue::ObjectIdentifier(expected_oid));
}

#[test]
fn read_object_name() {
    let dev = make_device();
    let val = dev
        .read_property(PropertyIdentifier::OBJECT_NAME, None)
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString("Test Device".into()));
}

#[test]
fn read_object_type() {
    let dev = make_device();
    let val = dev
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(ObjectType::DEVICE.to_raw()));
}

#[test]
fn read_vendor_name() {
    let dev = make_device();
    let val = dev
        .read_property(PropertyIdentifier::VENDOR_NAME, None)
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString("Rusty BACnet".into()));
}

#[test]
fn read_max_apdu_length() {
    let dev = make_device();
    let val = dev
        .read_property(PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(1476));
}

#[test]
fn read_unknown_property_fails() {
    let dev = make_device();
    // Use a property that Device doesn't have
    let result = dev.read_property(PropertyIdentifier::PRESENT_VALUE, None);
    assert!(result.is_err());
}

#[test]
fn write_property_denied() {
    let mut dev = make_device();
    let result = dev.write_property(
        PropertyIdentifier::OBJECT_NAME,
        None,
        PropertyValue::CharacterString("New Name".into()),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn device_description_default_empty() {
    let dev = make_device();
    let val = dev
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString(String::new()));
}

#[test]
fn device_description_write_read() {
    let mut dev = make_device();
    dev.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        PropertyValue::CharacterString("Main building controller".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        dev.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Main building controller".into())
    );
}

#[test]
fn device_set_description_convenience() {
    let mut dev = make_device();
    dev.set_description("Rooftop unit controller");
    assert_eq!(
        dev.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Rooftop unit controller".into())
    );
}

#[test]
fn device_description_in_property_list() {
    let dev = make_device();
    assert!(dev
        .property_list()
        .contains(&PropertyIdentifier::DESCRIPTION));
}

#[test]
fn object_list_default_contains_device() {
    let dev = make_device();
    // arrayIndex absent: returns the full array as a List
    let val = dev
        .read_property(PropertyIdentifier::OBJECT_LIST, None)
        .unwrap();
    let expected_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    assert_eq!(
        val,
        PropertyValue::List(vec![PropertyValue::ObjectIdentifier(expected_oid)])
    );
}

#[test]
fn object_list_array_index() {
    let dev = make_device();
    // Index 0 = length
    let val = dev
        .read_property(PropertyIdentifier::OBJECT_LIST, Some(0))
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(1));

    // Index 1 = first element (the device itself)
    let val = dev
        .read_property(PropertyIdentifier::OBJECT_LIST, Some(1))
        .unwrap();
    let expected_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    assert_eq!(val, PropertyValue::ObjectIdentifier(expected_oid));

    // Index 2 = out of range
    let result = dev.read_property(PropertyIdentifier::OBJECT_LIST, Some(2));
    assert!(result.is_err());
}

#[test]
fn set_object_list() {
    let mut dev = make_device();
    let dev_oid = dev.object_identifier();
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let ai2 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
    dev.set_object_list(vec![dev_oid, ai1, ai2]);

    // arrayIndex absent: returns the full array
    let val = dev
        .read_property(PropertyIdentifier::OBJECT_LIST, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(dev_oid),
            PropertyValue::ObjectIdentifier(ai1),
            PropertyValue::ObjectIdentifier(ai2),
        ])
    );

    // arrayIndex 0: returns the count
    let count = dev
        .read_property(PropertyIdentifier::OBJECT_LIST, Some(0))
        .unwrap();
    assert_eq!(count, PropertyValue::Unsigned(3));
}

#[test]
fn property_list_contains_expected() {
    let dev = make_device();
    let props = dev.property_list();
    assert!(props.contains(&PropertyIdentifier::OBJECT_IDENTIFIER));
    assert!(props.contains(&PropertyIdentifier::OBJECT_NAME));
    assert!(props.contains(&PropertyIdentifier::OBJECT_TYPE));
    assert!(props.contains(&PropertyIdentifier::VENDOR_NAME));
    assert!(props.contains(&PropertyIdentifier::OBJECT_LIST));
    assert!(props.contains(&PropertyIdentifier::PROPERTY_LIST));
    assert!(props.contains(&PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED));
    assert!(props.contains(&PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED));
}

#[test]
fn read_protocol_object_types_supported() {
    let dev = make_device();
    let val = dev
        .read_property(PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED, None)
        .unwrap();
    match val {
        PropertyValue::BitString { unused_bits, data } => {
            assert_eq!(unused_bits, 7);
            assert_eq!(data.len(), 9);
            // Byte 0 (types 0-7): all set
            assert_eq!(data[0], 0xFF);
            // Byte 1 (types 8-15): all set
            assert_eq!(data[1], 0xFF);
            // Byte 2 (types 16-23): all set
            assert_eq!(data[2], 0xFF);
            // Byte 3 (types 24-31): all set
            assert_eq!(data[3], 0xFF);
            // Byte 4 (types 32-39): 32-37,39 set; 38 (NetworkSecurity) unset
            assert_eq!(data[4], 0xFD);
            // Byte 5 (types 40-47): all set
            assert_eq!(data[5], 0xFF);
            // Byte 6 (types 48-55): all set
            assert_eq!(data[6], 0xFF);
            // Byte 7 (types 56-63): all set (56-62 + Color=63)
            assert_eq!(data[7], 0xFF);
            // Byte 8 (type 64): ColorTemperature set, 7 unused bits
            assert_eq!(data[8], 0x80);
        }
        _ => panic!("Expected BitString"),
    }
}

#[test]
fn read_protocol_services_supported() {
    let dev = make_device();
    let val = dev
        .read_property(PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED, None)
        .unwrap();
    match val {
        PropertyValue::BitString { unused_bits, data } => {
            assert_eq!(unused_bits, 7);
            assert_eq!(data.len(), 6);
            // Byte 0: services 0,2,5
            assert_eq!(data[0], 0xA4);
            // Byte 1: services 12,14,15
            assert_eq!(data[1], 0x0B);
            // Byte 4: service 32 (WhoIs)
            assert_eq!(data[4], 0x80);
        }
        _ => panic!("Expected BitString"),
    }
}

#[test]
fn active_cov_subscriptions_default_empty() {
    let dev = make_device();
    let val = dev
        .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::List(vec![]));
}

#[test]
fn active_cov_subscriptions_in_property_list() {
    let dev = make_device();
    assert!(dev
        .property_list()
        .contains(&PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS));
}

#[test]
fn active_cov_subscriptions_after_add() {
    use bacnet_types::constructed::{
        BACnetCOVSubscription, BACnetObjectPropertyReference, BACnetRecipient,
        BACnetRecipientProcess,
    };

    let mut dev = make_device();
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    dev.add_cov_subscription(BACnetCOVSubscription {
        recipient: BACnetRecipientProcess {
            recipient: BACnetRecipient::Device(dev_oid),
            process_identifier: 7,
        },
        monitored_property_reference: BACnetObjectPropertyReference::new(
            ai_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
        issue_confirmed_notifications: true,
        time_remaining: 300,
        cov_increment: Some(0.5),
    });

    let val = dev
        .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
        .unwrap();
    match val {
        PropertyValue::List(subs) => {
            assert_eq!(subs.len(), 1);
            match &subs[0] {
                PropertyValue::List(entry) => {
                    assert_eq!(entry.len(), 5); // includes cov_increment
                    assert_eq!(entry[0], PropertyValue::ObjectIdentifier(ai_oid));
                    assert_eq!(entry[1], PropertyValue::Unsigned(7));
                    assert_eq!(entry[2], PropertyValue::Boolean(true));
                    assert_eq!(entry[3], PropertyValue::Unsigned(300));
                    assert_eq!(entry[4], PropertyValue::Real(0.5));
                }
                _ => panic!("Expected List entry"),
            }
        }
        _ => panic!("Expected List"),
    }
}

#[test]
fn active_cov_subscriptions_without_increment() {
    use bacnet_types::constructed::{
        BACnetCOVSubscription, BACnetObjectPropertyReference, BACnetRecipient,
        BACnetRecipientProcess,
    };

    let mut dev = make_device();
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 50).unwrap();
    let bv_oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 3).unwrap();

    dev.add_cov_subscription(BACnetCOVSubscription {
        recipient: BACnetRecipientProcess {
            recipient: BACnetRecipient::Device(dev_oid),
            process_identifier: 1,
        },
        monitored_property_reference: BACnetObjectPropertyReference::new(
            bv_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
        issue_confirmed_notifications: false,
        time_remaining: 0,
        cov_increment: None,
    });

    let val = dev
        .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
        .unwrap();
    match val {
        PropertyValue::List(subs) => {
            assert_eq!(subs.len(), 1);
            match &subs[0] {
                PropertyValue::List(entry) => {
                    assert_eq!(entry.len(), 4); // no cov_increment
                    assert_eq!(entry[2], PropertyValue::Boolean(false));
                }
                _ => panic!("Expected List entry"),
            }
        }
        _ => panic!("Expected List"),
    }
}

#[test]
fn active_cov_subscriptions_write_denied() {
    let mut dev = make_device();
    let result = dev.write_property(
        PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
        None,
        PropertyValue::List(vec![]),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn set_active_cov_subscriptions_replaces() {
    use bacnet_types::constructed::{
        BACnetCOVSubscription, BACnetObjectPropertyReference, BACnetRecipient,
        BACnetRecipientProcess,
    };

    let mut dev = make_device();
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap();
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let ai2 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();

    // Add two subscriptions
    let sub1 = BACnetCOVSubscription {
        recipient: BACnetRecipientProcess {
            recipient: BACnetRecipient::Device(dev_oid),
            process_identifier: 1,
        },
        monitored_property_reference: BACnetObjectPropertyReference::new(
            ai1,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
        issue_confirmed_notifications: true,
        time_remaining: 100,
        cov_increment: None,
    };
    let sub2 = BACnetCOVSubscription {
        recipient: BACnetRecipientProcess {
            recipient: BACnetRecipient::Device(dev_oid),
            process_identifier: 2,
        },
        monitored_property_reference: BACnetObjectPropertyReference::new(
            ai2,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
        issue_confirmed_notifications: false,
        time_remaining: 200,
        cov_increment: Some(1.0),
    };
    dev.set_active_cov_subscriptions(vec![sub1, sub2]);

    let val = dev
        .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
        .unwrap();
    match val {
        PropertyValue::List(subs) => assert_eq!(subs.len(), 2),
        _ => panic!("Expected List"),
    }

    // Replace with empty
    dev.set_active_cov_subscriptions(vec![]);
    let val = dev
        .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::List(vec![]));
}

#[test]
fn compute_object_types_supported_known_inputs() {
    assert_eq!(compute_object_types_supported(&[0]), vec![0x80]);
    assert_eq!(compute_object_types_supported(&[8]), vec![0x00, 0x80]);
    assert_eq!(
        compute_object_types_supported(&[0, 1, 2, 3, 4, 5]),
        vec![0xFC]
    );
    assert_eq!(compute_object_types_supported(&[]), vec![0x00]);
}

#[test]
fn compute_object_types_supported_old_bits_preserved() {
    let old_types: Vec<u32> = vec![0, 1, 2, 3, 4, 5, 8, 13, 14, 19];
    let bs = compute_object_types_supported(&old_types);
    assert_eq!(bs[0], 0xFC);
    assert_eq!(bs[1], 0x86);
    assert_eq!(bs[2], 0x10);
}

#[test]
fn device_protocol_object_types_has_new_bits() {
    let dev = DeviceObject::new(DeviceConfig {
        instance: 1,
        name: "Test".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    let val = dev
        .read_property(PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED, None)
        .unwrap();
    let bits = match val {
        PropertyValue::BitString { data, .. } => data,
        _ => panic!("Expected BitString"),
    };
    assert!(bits.len() >= 8, "bitstring should cover types up to 62");
    assert_eq!(bits[0] & 0xFC, 0xFC, "AI/AO/AV/BI/BO/BV");
    assert_ne!(bits[1] & 0x80, 0, "Device (8)");
    assert_ne!(bits[1] & 0x04, 0, "MSI (13)");
    assert_ne!(bits[1] & 0x02, 0, "MSO (14)");
    assert_ne!(bits[2] & 0x10, 0, "MSV (19)");
    assert_ne!(bits[0] & 0x03, 0, "Calendar(6) and Command(7)");
    assert_ne!(bits[3] & 0x80, 0, "Accumulator (24)");
    assert_ne!(bits[7] & 0x80, 0, "NetworkPort (56)");
}
