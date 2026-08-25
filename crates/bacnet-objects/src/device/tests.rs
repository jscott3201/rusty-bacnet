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
fn mode_derived_max_segments_accepted() {
    let cases = [
        ("none", Segmentation::NONE, None),
        ("transmit", Segmentation::TRANSMIT, Some(1)),
        ("receive", Segmentation::RECEIVE, Some(65)),
        ("both", Segmentation::BOTH, Some(65)),
        ("unknown", Segmentation::from_raw(64), Some(65)),
    ];

    for (name, segmentation, expected_max_segments) in cases {
        let dev = DeviceObject::new(DeviceConfig {
            segmentation_supported: segmentation,
            ..DeviceConfig::default()
        })
        .unwrap();

        assert_eq!(
            dev.read_property(PropertyIdentifier::SEGMENTATION_SUPPORTED, None)
                .unwrap(),
            PropertyValue::Enumerated(segmentation.to_raw() as u32),
            "{name} segmentation readback"
        );

        let PropertyValue::List(property_list) = dev
            .read_property(PropertyIdentifier::PROPERTY_LIST, None)
            .unwrap()
        else {
            panic!("{name} Property_List was not a list");
        };
        assert_eq!(
            property_list.contains(&PropertyValue::Enumerated(
                PropertyIdentifier::MAX_SEGMENTS_ACCEPTED.to_raw(),
            )),
            expected_max_segments.is_some(),
            "{name} Property_List presence"
        );

        let max_segments = dev.read_property(PropertyIdentifier::MAX_SEGMENTS_ACCEPTED, None);
        match expected_max_segments {
            Some(expected) => assert_eq!(
                max_segments.unwrap(),
                PropertyValue::Unsigned(expected),
                "{name} Max_Segments_Accepted"
            ),
            None => assert!(matches!(
                max_segments,
                Err(Error::Protocol { class, code })
                    if class == ErrorClass::PROPERTY.to_raw() as u32
                        && code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32
            )),
        }
    }
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
            assert_eq!(
                data,
                vec![0xFF, 0xFF, 0xFF, 0xFF, 0xFD, 0xFF, 0xEB, 0xFF, 0x80],
                "types 51 and 53 must stay clear while types 50, 52, 54, and 64 remain set"
            );
        }
        _ => panic!("Expected BitString"),
    }
}

#[test]
fn read_protocol_services_supported() {
    use bacnet_types::bitstring::ServicesSupported;
    use bacnet_types::enums::ServiceSupported;

    let dev = make_device();
    let val = dev
        .read_property(PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED, None)
        .unwrap();
    match val {
        PropertyValue::BitString { unused_bits, data } => {
            // Full Clause 21 production: bits 0..=48 → 7 octets, 7 unused.
            assert_eq!(unused_bits, 7);
            assert_eq!(data.len(), 7);

            let ss = ServicesSupported::from_bacnet(&data);
            for service in EXECUTED_SERVICES {
                assert!(ss.contains(*service), "missing {service}");
            }
            assert_eq!(
                ss.iter().count(),
                EXECUTED_SERVICES.len(),
                "no bits beyond EXECUTED_SERVICES may be set"
            );

            // Semantic pins from #192: divergent-numbering services land on
            // their bit-35+ positions (impossible in the old 41-bit string)…
            assert!(ss.contains(ServiceSupported::WHO_IS));
            assert!(ss.contains(ServiceSupported::READ_RANGE));
            assert!(ss.contains(ServiceSupported::SUBSCRIBE_COV_PROPERTY_MULTIPLE));
            assert!(!ss.contains(ServiceSupported::WRITE_GROUP));
            // …and initiate-only services are not declared as executed.
            assert!(!ss.contains(ServiceSupported::I_AM));
            assert!(!ss.contains(ServiceSupported::I_HAVE));
            assert!(!ss.contains(ServiceSupported::CONFIRMED_EVENT_NOTIFICATION));
            assert!(!ss.contains(ServiceSupported::UNCONFIRMED_COV_NOTIFICATION));
        }
        _ => panic!("Expected BitString"),
    }
}

#[test]
fn set_services_supported_overrides_default() {
    use bacnet_types::bitstring::ServicesSupported;
    use bacnet_types::enums::ServiceSupported;

    let mut dev = make_device();
    dev.set_services_supported(&[ServiceSupported::READ_PROPERTY]);
    let val = dev
        .read_property(PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED, None)
        .unwrap();
    let PropertyValue::BitString { unused_bits, data } = val else {
        panic!("Expected BitString");
    };
    assert_eq!((unused_bits, data.len()), (7, 7));
    let ss = ServicesSupported::from_bacnet(&data);
    assert!(ss.contains(ServiceSupported::READ_PROPERTY));
    assert_eq!(ss.iter().count(), 1);
}

#[test]
fn active_cov_subscriptions_default_empty() {
    let dev = make_device();
    let val = dev
        .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::ApplicationData(Vec::new()));
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
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 7).unwrap();
    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 3).unwrap();

    dev.add_cov_subscription(BACnetCOVSubscription {
        recipient: BACnetRecipientProcess {
            recipient: BACnetRecipient::Device(dev_oid),
            process_identifier: 7,
        },
        monitored_property_reference: BACnetObjectPropertyReference::new_indexed(ao_oid, 87, 2),
        issue_confirmed_notifications: true,
        time_remaining: 300,
        cov_increment: Some(0.5),
    });

    let val = dev
        .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::ApplicationData(vec![
            0x0E, 0x0E, 0x0C, 0x02, 0x00, 0x00, 0x07, 0x0F, 0x19, 0x07, 0x0F, 0x1E, 0x0C, 0x00,
            0x40, 0x00, 0x03, 0x19, 0x57, 0x29, 0x02, 0x1F, 0x29, 0x01, 0x3A, 0x01, 0x2C, 0x4C,
            0x3F, 0x00, 0x00, 0x00,
        ])
    );
}

#[test]
fn active_cov_subscriptions_without_increment() {
    use bacnet_types::constructed::{
        BACnetCOVSubscription, BACnetObjectPropertyReference, BACnetRecipient,
        BACnetRecipientProcess,
    };

    let mut dev = make_device();
    let bv_oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 3).unwrap();

    dev.add_cov_subscription(BACnetCOVSubscription {
        recipient: BACnetRecipientProcess {
            recipient: BACnetRecipient::Address(bacnet_types::constructed::BACnetAddress {
                network_number: 0x1234,
                mac_address: bacnet_types::MacAddr::from_slice(&[0xAA, 0xBB]),
            }),
            process_identifier: 9,
        },
        monitored_property_reference: BACnetObjectPropertyReference::new(
            bv_oid,
            PropertyIdentifier::STATUS_FLAGS.to_raw(),
        ),
        issue_confirmed_notifications: false,
        time_remaining: 0,
        cov_increment: None,
    });

    let val = dev
        .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::ApplicationData(vec![
            0x0E, 0x0E, 0x1E, 0x22, 0x12, 0x34, 0x62, 0xAA, 0xBB, 0x1F, 0x0F, 0x19, 0x09, 0x0F,
            0x1E, 0x0C, 0x01, 0x40, 0x00, 0x03, 0x19, 0x6F, 0x1F, 0x29, 0x00, 0x39, 0x00,
        ])
    );
}

#[test]
fn active_cov_subscriptions_write_denied() {
    let mut dev = make_device();
    let result = dev.write_property(
        PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
        None,
        PropertyValue::ApplicationData(Vec::new()),
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
    let subscriptions = vec![sub1, sub2];
    let mut expected = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_cov_subscription_list(&mut expected, &subscriptions);
    dev.set_active_cov_subscriptions(subscriptions);

    let val = dev
        .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::ApplicationData(expected.to_vec()));

    // Replace with empty
    dev.set_active_cov_subscriptions(vec![]);
    let val = dev
        .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::ApplicationData(Vec::new()));
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
