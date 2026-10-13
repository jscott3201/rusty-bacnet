//! Optional rows on the actual selected Device, alongside live executor state.
use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::object_profile::ObjectProfile;
use bacnet_types::constructed::BACnetNameValue;

const TAGS: PropertyIdentifier = PropertyIdentifier::TAGS;
const LOCATION: PropertyIdentifier = PropertyIdentifier::PROFILE_LOCATION;
const NAME: PropertyIdentifier = PropertyIdentifier::PROFILE_NAME;
const EXHAUST: &[u8] = &[0x0d, 8, 0, b'e', b'x', b'h', b'a', b'u', b's', b't'];
const FLOOR: &[u8] = &[0x0d, 6, 0, b'f', b'l', b'o', b'o', b'r', 0x21, 3];
fn text(value: &str) -> Vec<u8> {
    let mut bytes = vec![0x75, value.len() as u8 + 1, 0];
    bytes.extend(value.as_bytes());
    bytes
}
fn database(mask: u8) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    // Higher identity is inserted first; the profiled Device must still own views.
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 900,
            name: "Higher Device".into(),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let mut object = DeviceObject::new(DeviceConfig {
        instance: device().instance_number(),
        ..Default::default()
    })
    .unwrap();
    object.set_services_supported(&[ServiceSupported::READ_PROPERTY]);
    object
        .set_profile(ObjectProfile {
            tags: (mask & 1 != 0).then(|| vec![BACnetNameValue::semantic("exhaust")]),
            profile_location: (mask & 2 != 0).then(|| "https://example.com/p.xdd".into()),
            profile_name: (mask & 4 != 0).then(|| "555-device".into()),
        })
        .unwrap();
    db.add(Box::new(object)).unwrap();
    db.add(Box::new(AnalogValueObject::new(1, "AV-1", 62).unwrap()))
        .unwrap();
    assert_eq!(db.selected_device(), Some(device()));
    db
}
async fn local_bytes(wire: &Wire, target: ObjectIdentifier, p: PropertyIdentifier) -> Vec<u8> {
    let value = wire.server.read_local(&target, p, None).await.unwrap();
    let mut bytes = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut bytes, &value).unwrap();
    bytes.to_vec()
}
#[tokio::test(start_paused = true)]
async fn selected_device_profiles_preserve_live_cov_bindings_services_and_pics() {
    for mask in 0..8 {
        let mut wire = Wire::start_with_database(ServerConfig::default(), database(mask)).await;
        simple_ack(
            wire.send(&direct(), subscribe_cov(77, av(1), Some(false), Some(300)))
                .await,
        );
        let active = wire.active().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].recipient.process_identifier, 77);
        assert_eq!(
            active[0].monitored_property_reference.object_identifier,
            av(1)
        );
        assert_eq!(active[0].time_remaining, 300);
        let active_bytes = wire.active_bytes().await;
        assert!(!active_bytes.is_empty());
        wire.server
            .device_bindings
            .write()
            .await
            .insert_configured(
                DeviceBinding::local(
                    ObjectIdentifier::new(ObjectType::DEVICE, 21).unwrap(),
                    [10, 0, 0, 21, 0xba, 0xc0],
                )
                .unwrap(),
                |_| false,
            )
            .unwrap();
        // Independent BACnetAddressBinding: Device21, local network0, six-byte B/IP MAC.
        let binding = vec![
            0xc4, 2, 0, 0, 21, 0x21, 0, 0x65, 6, 10, 0, 0, 21, 0xba, 0xc0,
        ];
        let services = wire.read(device(), SERVICES, None).await.unwrap();
        assert_eq!(service_bits(&value(&services)), expected_services(true));
        for written in [false, true] {
            if written && mask & 1 != 0 {
                let mut request = BytesMut::new();
                WritePropertyRequest {
                    object_identifier: device(),
                    property_identifier: TAGS,
                    property_array_index: None,
                    property_value: FLOOR.to_vec(),
                    priority: None,
                }
                .encode(&mut request)
                .unwrap();
                simple_ack(
                    wire.send(&direct(), (ConfirmedServiceChoice::WRITE_PROPERTY, request))
                        .await,
                );
            }
            for target in [device(), wildcard()] {
                let mut expected = vec![
                    (ACTIVE, active_bytes.clone()),
                    (BINDINGS, binding.clone()),
                    (SERVICES, services.clone()),
                ];
                for (bit, p, bytes) in [
                    (
                        1,
                        TAGS,
                        if written {
                            FLOOR.to_vec()
                        } else {
                            EXHAUST.to_vec()
                        },
                    ),
                    (2, LOCATION, text("https://example.com/p.xdd")),
                    (4, NAME, text("555-device")),
                ] {
                    if mask & bit == 0 {
                        assert_eq!(
                            wire.read(target, p, None).await,
                            Err((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
                        );
                    } else {
                        expected.push((p, bytes));
                    }
                }
                for (p, bytes) in &expected {
                    assert_eq!(wire.read(target, *p, None).await.unwrap(), *bytes);
                    assert_eq!(local_bytes(&wire, target, *p).await, *bytes);
                }
                let ack = wire
                    .rpm(vec![(
                        target,
                        expected.iter().map(|(p, _)| (*p, None)).collect(),
                    )])
                    .await;
                let rows = &ack.list_of_read_access_results[0].list_of_results;
                assert_eq!(rows.len(), expected.len());
                for (row, (p, bytes)) in rows.iter().zip(expected) {
                    assert_eq!(row.property_identifier, p);
                    assert_eq!(row.error, None);
                    assert_eq!(row.property_value, Some(bytes));
                }
                let listed = property_list(&wire.read(target, LIST, None).await.unwrap());
                assert!(listed.windows(2).all(|pair| pair[0] < pair[1]));
                assert!(!listed.contains(&LIST.to_raw()));
                assert_eq!(
                    value(&wire.read(target, LIST, Some(0)).await.unwrap()),
                    PropertyValue::Unsigned(listed.len() as u64)
                );
                for (index, p) in listed.iter().enumerate() {
                    assert_eq!(
                        value(
                            &wire
                                .read(target, LIST, Some(index as u32 + 1))
                                .await
                                .unwrap()
                        ),
                        PropertyValue::Enumerated(*p)
                    );
                }
                for (bit, p) in [(1, TAGS), (2, LOCATION), (4, NAME)] {
                    assert_eq!(listed.contains(&p.to_raw()), mask & bit != 0);
                }
            }
        }
        let pics = wire
            .server
            .generate_pics(&crate::pics::PicsConfig::default())
            .await;
        let device = pics
            .supported_object_types
            .iter()
            .find(|o| o.object_type == ObjectType::DEVICE)
            .unwrap();
        for (bit, p) in [(1, TAGS), (2, LOCATION), (4, NAME)] {
            let row = device
                .supported_properties
                .iter()
                .find(|r| r.property_id == p);
            assert_eq!(row.is_some(), mask & bit != 0);
            if let Some(row) = row {
                assert!(row.access.readable && row.access.optional);
                assert_eq!(row.access.writable, p == TAGS);
            }
        }
        for p in [ACTIVE, MULTIPLE, BINDINGS, SERVICES] {
            let row = device
                .supported_properties
                .iter()
                .find(|r| r.property_id == p)
                .unwrap();
            assert!(row.access.readable && !row.access.writable);
        }
        wire.server.stop().await.unwrap();
    }
}
