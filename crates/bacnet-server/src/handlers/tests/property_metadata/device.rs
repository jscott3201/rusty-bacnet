use super::*;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::ServicesSupported;
use bacnet_types::enums::{Segmentation, ServiceSupported};
use bacnet_types::primitives::{Date, PropertyValue, Time};
use std::sync::{Arc, Mutex};
use PropertyIdentifier as P;

struct TestClock(Mutex<Option<ClockFrame>>);

impl ClockReader for TestClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        *self.0.lock().unwrap()
    }
}

#[test]
fn rpm_device_property_metadata_pics_and_database_are_exact() {
    // Independent effective-set fixtures; no expected set comes from metadata.
    let required = [
        P::APDU_TIMEOUT,
        P::APPLICATION_SOFTWARE_VERSION,
        P::DEVICE_ADDRESS_BINDING,
        P::FIRMWARE_REVISION,
        P::MAX_APDU_LENGTH_ACCEPTED,
        P::MODEL_NAME,
        P::NUMBER_OF_APDU_RETRIES,
        P::OBJECT_IDENTIFIER,
        P::OBJECT_LIST,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PROTOCOL_OBJECT_TYPES_SUPPORTED,
        P::PROTOCOL_SERVICES_SUPPORTED,
        P::PROTOCOL_VERSION,
        P::SEGMENTATION_SUPPORTED,
        P::SYSTEM_STATUS,
        P::VENDOR_IDENTIFIER,
        P::VENDOR_NAME,
        P::PROTOCOL_REVISION,
        P::DATABASE_REVISION,
        P::PROPERTY_LIST,
    ];
    let frame = ClockFrame {
        local_date: Date {
            year: 126,
            month: 9,
            day: 14,
            day_of_week: 1,
        },
        local_time: Time {
            hour: 12,
            minute: 30,
            second: 15,
            hundredths: 25,
        },
        utc_offset: 300,
        daylight_savings_status: true,
    };
    for segmentation_supported in [
        Segmentation::NONE,
        Segmentation::TRANSMIT,
        Segmentation::RECEIVE,
        Segmentation::BOTH,
        Segmentation::from_raw(64),
    ] {
        let mut db = ObjectDatabase::new();
        let mut object = DeviceObject::new(DeviceConfig {
            instance: 77,
            name: "Device metadata".into(),
            segmentation_supported,
            ..DeviceConfig::default()
        })
        .unwrap();
        object.set_description("configured description".repeat(40));
        let oid = object.object_identifier();
        let other = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
        object.set_object_list(vec![oid, other]);
        db.add(Box::new(object)).unwrap();
        let clock = Arc::new(TestClock(Mutex::new(None)));
        for state in [0, 1, 2, 1, 0] {
            *clock.0.lock().unwrap() = (state == 1).then_some(frame);
            db.set_clock_reader((state != 0).then(|| clock.clone() as Arc<dyn ClockReader>));
            let mut optional = vec![
                P::DESCRIPTION,
                P::ACTIVE_COV_SUBSCRIPTIONS,
                P::LAST_RESTART_REASON,
                P::DEVICE_UUID,
            ];
            if segmentation_supported != Segmentation::NONE {
                optional.push(P::MAX_SEGMENTS_ACCEPTED);
            }
            if state == 1 {
                optional.extend([
                    P::LOCAL_DATE,
                    P::LOCAL_TIME,
                    P::UTC_OFFSET,
                    P::DAYLIGHT_SAVINGS_STATUS,
                ]);
            }
            optional.sort_by_key(|p| p.to_raw());
            let mut all = required.to_vec();
            all.extend(&optional);
            all.sort_by_key(|p| p.to_raw());
            assert_eq!(db.find_by_type(ObjectType::DEVICE), vec![oid]);
            assert_eq!(db.list_objects(), vec![oid]);
            let object = db.get(&oid).unwrap();
            assert_eq!(object.property_list().as_ref(), all);
            assert_eq!(object.required_properties().as_ref(), required);
            for property in [
                P::LOCAL_DATE,
                P::LOCAL_TIME,
                P::UTC_OFFSET,
                P::DAYLIGHT_SAVINGS_STATUS,
            ] {
                let read = object.read_property(property, None);
                if state == 1 {
                    read.unwrap();
                } else {
                    assert!(matches!(read, Err(Error::Protocol { class, code })
                        if class == bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32
                            && code == bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32));
                }
            }
            let PropertyValue::BitString { unused_bits, data } = object
                .read_property(P::PROTOCOL_SERVICES_SUPPORTED, None)
                .unwrap()
            else {
                panic!("expected supported-service bits");
            };
            assert_eq!((unused_bits, data.len()), (7, 7));
            let services = ServicesSupported::from_bacnet(&data);
            let expected_services: Vec<_> = bacnet_objects::device::EXECUTED_SERVICES
                .iter()
                .copied()
                .filter(|s| {
                    state == 1
                        || !matches!(
                            *s,
                            ServiceSupported::TIME_SYNCHRONIZATION
                                | ServiceSupported::UTC_TIME_SYNCHRONIZATION
                        )
                })
                .collect();
            assert_eq!(services.iter().count(), expected_services.len());
            assert!(expected_services.iter().all(|s| services.contains(*s)));
            let metadata = object.property_metadata();
            assert!(matches!(metadata, std::borrow::Cow::Borrowed(_)));
            assert_eq!(
                metadata
                    .iter()
                    .map(|r| r.property_identifier)
                    .collect::<Vec<_>>(),
                all
            );
            for row in metadata.iter() {
                assert_eq!(
                    row.conformance.is_required(),
                    required.contains(&row.property_identifier)
                );
                assert_eq!(
                    row.write_capability.is_writable(),
                    row.property_identifier == P::DESCRIPTION
                );
            }
            let pics = crate::pics::generate_pics(
                &db,
                &crate::server::ServerConfig::default(),
                &crate::pics::PicsConfig::default(),
            );
            assert_eq!(pics.supported_object_types.len(), 1);
            let support = &pics.supported_object_types[0];
            assert_eq!(support.object_type, ObjectType::DEVICE);
            assert!(!support.createable);
            assert!(!support.deleteable);
            assert_eq!(
                support
                    .supported_properties
                    .iter()
                    .map(|r| {
                        assert!(r.access.readable);
                        (r.property_id, r.access.optional, r.access.writable)
                    })
                    .collect::<Vec<_>>(),
                all.iter()
                    .map(|&p| (p, optional.contains(&p), p == P::DESCRIPTION))
                    .collect::<Vec<_>>()
            );
            let mut all_rpm = all.clone();
            all_rpm.retain(|&p| p != P::PROPERTY_LIST);
            let required_rpm: Vec<_> = required
                .iter()
                .copied()
                .filter(|&p| p != P::PROPERTY_LIST)
                .collect();
            for (selector, expected) in [
                (P::ALL, all_rpm.as_slice()),
                (P::REQUIRED, required_rpm.as_slice()),
                (P::OPTIONAL, optional.as_slice()),
                (P::PROPERTY_LIST, &[P::PROPERTY_LIST]),
            ] {
                assert_rpm_selector_bytes(&db, oid, selector, expected);
            }
            assert_eq!(
                object.read_property(P::OBJECT_LIST, None).unwrap(),
                PropertyValue::List(vec![
                    PropertyValue::ObjectIdentifier(oid),
                    PropertyValue::ObjectIdentifier(other)
                ])
            );
            assert_eq!(
                object.read_property(P::OBJECT_LIST, Some(0)).unwrap(),
                PropertyValue::Unsigned(2)
            );
            assert_eq!(
                object
                    .read_property(P::ACTIVE_COV_SUBSCRIPTIONS, None)
                    .unwrap(),
                PropertyValue::ApplicationData(vec![])
            );
        }
    }
}
