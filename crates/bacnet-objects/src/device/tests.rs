mod description;

use super::*;
use crate::clock::{ClockFrame, ClockReader};
use bacnet_types::primitives::{Date, Time};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct FakeClock(Arc<Mutex<Option<ClockFrame>>>);

impl FakeClock {
    fn new(frame: ClockFrame) -> Self {
        Self(Arc::new(Mutex::new(Some(frame))))
    }

    fn set(&self, frame: ClockFrame) {
        *self.0.lock().unwrap() = Some(frame);
    }
}

impl ClockReader for FakeClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        *self.0.lock().ok()?
    }
}

fn clock_frame(hour: u8, minute: u8) -> ClockFrame {
    ClockFrame {
        local_date: Date {
            year: 126,
            month: 8,
            day: 26,
            day_of_week: 3,
        },
        local_time: Time {
            hour,
            minute,
            second: 30,
            hundredths: 25,
        },
        utc_offset: 300,
        daylight_savings_status: true,
    }
}

fn bind_clock(device: &mut DeviceObject, clock: FakeClock) {
    device.bind_clock_internal(Some(Arc::new(clock)));
}

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

    let mut dev = make_device();
    bind_clock(&mut dev, FakeClock::new(clock_frame(12, 0)));
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
            assert!(ss.contains(ServiceSupported::UNCONFIRMED_AUDIT_NOTIFICATION));
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
fn clockless_device_omits_clock_properties_and_sync_services() {
    use bacnet_types::bitstring::ServicesSupported;

    let dev = make_device();
    let props = dev.property_list();
    for property in [
        PropertyIdentifier::LOCAL_DATE,
        PropertyIdentifier::LOCAL_TIME,
        PropertyIdentifier::UTC_OFFSET,
        PropertyIdentifier::DAYLIGHT_SAVINGS_STATUS,
    ] {
        assert!(!props.contains(&property));
        assert!(matches!(
            dev.read_property(property, None),
            Err(Error::Protocol { class, code })
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32
        ));
    }

    let PropertyValue::BitString { data, .. } = dev
        .read_property(PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED, None)
        .unwrap()
    else {
        panic!("Expected BitString");
    };
    let services = ServicesSupported::from_bacnet(&data);
    assert!(!services.contains(ServiceSupported::TIME_SYNCHRONIZATION));
    assert!(!services.contains(ServiceSupported::UTC_TIME_SYNCHRONIZATION));
}

#[test]
fn bound_device_reads_one_advancing_clock_source() {
    let mut dev = make_device();
    let clock = FakeClock::new(clock_frame(9, 15));
    bind_clock(&mut dev, clock.clone());

    assert_eq!(
        dev.read_property(PropertyIdentifier::LOCAL_TIME, None)
            .unwrap(),
        PropertyValue::Time(clock_frame(9, 15).local_time)
    );
    assert_eq!(
        dev.read_property(PropertyIdentifier::UTC_OFFSET, None)
            .unwrap(),
        PropertyValue::Signed(300)
    );
    assert_eq!(
        dev.read_property(PropertyIdentifier::DAYLIGHT_SAVINGS_STATUS, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );

    clock.set(clock_frame(9, 16));
    assert_eq!(
        dev.read_property(PropertyIdentifier::LOCAL_TIME, None)
            .unwrap(),
        PropertyValue::Time(clock_frame(9, 16).local_time)
    );
    for property in [
        PropertyIdentifier::LOCAL_DATE,
        PropertyIdentifier::LOCAL_TIME,
        PropertyIdentifier::UTC_OFFSET,
        PropertyIdentifier::DAYLIGHT_SAVINGS_STATUS,
    ] {
        assert!(dev.property_list().contains(&property));
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

/// Standalone object data: the Device holds no subscription state. The live
/// list is the server COV table's projection (bacnet-server wire tests); the
/// constructed codec is covered by bacnet-encoding's golden vectors.
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

/// Table 12-13 footnote 18: the bundled Device executes
/// SubscribeCOVPropertyMultiple, so its standalone object lists
/// Active_COV_Multiple_Subscriptions as an optional, read-only, non-array
/// list that is empty without the server's live COV table.
#[test]
fn active_cov_multiple_subscriptions_standalone_optional_read_only_list() {
    let property = PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS;
    let mut dev = make_device();
    assert_eq!(
        dev.read_property(property, None).unwrap(),
        PropertyValue::ApplicationData(Vec::new())
    );
    assert!(dev.property_list().contains(&property));
    let metadata = dev.property_metadata();
    let row = metadata
        .iter()
        .find(|row| row.property_identifier == property)
        .unwrap();
    assert!(!row.is_required());
    assert!(!row.write_capability.is_writable());
    assert!(!dev.is_array_property(property));
    let denied = dev.write_property(
        property,
        None,
        PropertyValue::ApplicationData(Vec::new()),
        None,
    );
    assert!(matches!(denied, Err(Error::Protocol { class, code })
        if class == ErrorClass::PROPERTY.to_raw() as u32
            && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32));
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

#[test]
fn device_property_metadata_preserves_dynamic_list_and_write_dispatch() {
    use crate::property_metadata::PropertyWriteCapability;
    use PropertyIdentifier as P;

    for segmentation_supported in [
        Segmentation::NONE,
        Segmentation::TRANSMIT,
        Segmentation::RECEIVE,
        Segmentation::BOTH,
        Segmentation::from_raw(64),
    ] {
        let mut device = DeviceObject::new(DeviceConfig {
            segmentation_supported,
            ..DeviceConfig::default()
        })
        .unwrap();
        let clock = FakeClock::new(clock_frame(12, 0));
        // Bind, lose a sample, recover, and unbind on the same instance.
        for state in [0, 1, 2, 1, 0] {
            *clock.0.lock().unwrap() = (state == 1).then(|| clock_frame(12, 0));
            device.bind_clock_internal(
                (state != 0).then(|| Arc::new(clock.clone()) as Arc<dyn ClockReader>),
            );
            let mut expected: Vec<_> = device.properties.keys().copied().collect();
            expected.extend([
                P::OBJECT_LIST,
                P::PROPERTY_LIST,
                P::PROTOCOL_OBJECT_TYPES_SUPPORTED,
                P::PROTOCOL_SERVICES_SUPPORTED,
                P::ACTIVE_COV_SUBSCRIPTIONS,
                P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
            ]);
            if state == 1 {
                expected.extend([
                    P::LOCAL_DATE,
                    P::LOCAL_TIME,
                    P::UTC_OFFSET,
                    P::DAYLIGHT_SAVINGS_STATUS,
                ]);
            }
            expected.sort_by_key(|p| p.to_raw());
            let metadata = device.property_metadata();
            assert!(matches!(metadata, Cow::Borrowed(_)));
            assert_eq!(
                metadata
                    .iter()
                    .map(|row| row.property_identifier)
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(device.property_list().as_ref(), expected);
            let wire: Vec<_> = expected
                .iter()
                .copied()
                .filter(|p| {
                    !matches!(
                        *p,
                        P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE | P::PROPERTY_LIST
                    )
                })
                .map(|p| PropertyValue::Enumerated(p.to_raw()))
                .collect();
            assert_eq!(
                device.read_property(P::PROPERTY_LIST, None).unwrap(),
                PropertyValue::List(wire.clone())
            );
            assert_eq!(
                device.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
                PropertyValue::Unsigned(wire.len() as u64)
            );
            for (i, value) in wire.iter().enumerate() {
                assert_eq!(
                    device
                        .read_property(P::PROPERTY_LIST, Some(i as u32 + 1))
                        .unwrap(),
                    *value
                );
            }
            assert!(
                matches!(device.read_property(P::PROPERTY_LIST, Some(wire.len() as u32 + 1)),
                Err(Error::Protocol { class, code }) if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32)
            );
            let metadata = metadata.into_owned();
            for row in metadata {
                let p = row.property_identifier;
                let before = device.read_property(p, None).unwrap();
                assert_eq!(
                    row.write_capability,
                    if p == P::DESCRIPTION {
                        PropertyWriteCapability::Always
                    } else {
                        PropertyWriteCapability::ReadOnly
                    }
                );
                assert_eq!(device.is_writable_property(p), p == P::DESCRIPTION);
                let result = device.write_property(p, None, before.clone(), None);
                if p == P::DESCRIPTION {
                    result.unwrap();
                } else {
                    assert!(matches!(result, Err(Error::Protocol { class, code })
                        if class == ErrorClass::PROPERTY.to_raw() as u32
                            && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32));
                }
                assert_eq!(device.read_property(p, None).unwrap(), before);
            }
        }
    }
}
