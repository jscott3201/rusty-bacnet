use super::*;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::ServiceSupported;
use std::borrow::Cow;

const SERVICES: PropertyIdentifier = PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED;
const LIST: PropertyIdentifier = PropertyIdentifier::PROPERTY_LIST;
const BINDINGS: PropertyIdentifier = PropertyIdentifier::DEVICE_ADDRESS_BINDING;

fn value(bytes: &[u8]) -> PropertyValue {
    let (value, end) = bacnet_encoding::primitives::decode_application_value(bytes, 0).unwrap();
    assert_eq!(end, bytes.len());
    value
}
fn service_bits(value: &PropertyValue) -> Vec<u8> {
    let PropertyValue::BitString { unused_bits, data } = value else {
        panic!("services bitstring");
    };
    assert_eq!((*unused_bits, data.len()), (7, 7));
    bacnet_types::bitstring::ServicesSupported::from_bacnet(data)
        .iter()
        .map(|service| service.to_raw())
        .collect()
}
fn expected_services(clock: bool) -> Vec<u8> {
    let mut services: Vec<_> = bacnet_objects::device::EXECUTED_SERVICES
        .iter()
        .filter(|service| {
            clock
                || !matches!(
                    **service,
                    ServiceSupported::TIME_SYNCHRONIZATION
                        | ServiceSupported::UTC_TIME_SYNCHRONIZATION
                )
        })
        .map(|service| service.to_raw())
        .collect();
    services.sort_unstable();
    services
}

/// Opaque custom reader: executor-owned rows must never delegate into it.
struct CustomDevice {
    oid: ObjectIdentifier,
    metadata: bool,
}
impl BACnetObject for CustomDevice {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        "custom-device"
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        _: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        assert!(
            ![SERVICES, LIST, ACTIVE, MULTIPLE, BINDINGS].contains(&property),
            "served owned property reached custom reader: {property:?}"
        );
        Ok(PropertyValue::CharacterString("custom-description".into()))
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        panic!("read-only fixture")
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Owned(vec![PropertyIdentifier::DESCRIPTION, ACTIVE, MULTIPLE])
    }
    fn property_metadata(&self) -> Cow<'_, [bacnet_objects::property_metadata::PropertyMetadata]> {
        use bacnet_objects::property_metadata::{
            PropertyConformance, PropertyMetadata, PropertyWriteCapability,
        };
        if !self.metadata {
            return Cow::Borrowed(&[]);
        }
        Cow::Owned(
            self.property_list()
                .iter()
                .map(|property| {
                    PropertyMetadata::new(
                        *property,
                        PropertyConformance::RequiredWrite,
                        None,
                        PropertyWriteCapability::Always,
                    )
                })
                .collect(),
        )
    }
    fn is_array_property(&self, property: PropertyIdentifier) -> bool {
        property != LIST
    }
}

async fn assert_profile(wire: &mut Wire, target: ObjectIdentifier, clock: bool) {
    let bytes = wire.read(target, SERVICES, None).await.unwrap();
    let expected = expected_services(clock);
    assert_eq!(service_bits(&value(&bytes)), expected);
    let local = wire
        .server
        .read_local(&target, SERVICES, None)
        .await
        .unwrap();
    assert_eq!(service_bits(&local), expected);
    let listed = property_list(&wire.read(target, LIST, None).await.unwrap());
    assert!(
        listed.contains(&ACTIVE.to_raw())
            && listed.contains(&MULTIPLE.to_raw())
            && listed.contains(&SERVICES.to_raw())
    );
    assert!(!listed.contains(&LIST.to_raw()));
    assert_eq!(
        value(&wire.read(target, LIST, Some(0)).await.unwrap()),
        PropertyValue::Unsigned(listed.len() as u64)
    );
    assert_eq!(
        value(&wire.read(target, LIST, Some(1)).await.unwrap()),
        PropertyValue::Enumerated(listed[0])
    );
    assert_eq!(
        wire.read(target, LIST, Some(u32::MAX)).await,
        Err((ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX))
    );
    for property in [ACTIVE, MULTIPLE, SERVICES] {
        assert_eq!(
            wire.read(target, property, Some(0)).await,
            Err((ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY))
        );
    }
    let ack = wire
        .rpm(vec![(
            target,
            vec![
                (PropertyIdentifier::ALL, None),
                (PropertyIdentifier::OPTIONAL, None),
                (PropertyIdentifier::REQUIRED, None),
                (SERVICES, Some(0)),
                (ACTIVE, Some(0)),
                (MULTIPLE, Some(0)),
                (LIST, Some(0)),
            ],
        )])
        .await;
    let elements = &ack.list_of_read_access_results[0].list_of_results;
    for property in [ACTIVE, MULTIPLE] {
        let rows: Vec<_> = elements
            .iter()
            .filter(|row| row.property_identifier == property)
            .collect();
        assert_eq!(
            rows.len(),
            3,
            "ALL, OPTIONAL, explicit indexed; never REQUIRED"
        );
        assert_eq!(rows[2].property_array_index, None);
        assert_eq!(
            rows[2].error,
            Some((ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY))
        );
        for row in &rows[..2] {
            assert_eq!(row.error, None);
        }
    }
    let rows: Vec<_> = elements
        .iter()
        .filter(|row| row.property_identifier == SERVICES)
        .collect();
    assert_eq!(
        rows.len(),
        3,
        "ALL, REQUIRED, explicit indexed; never OPTIONAL"
    );
    assert_eq!(
        service_bits(&value(rows[0].property_value.as_ref().unwrap())),
        expected
    );
    assert_eq!(
        service_bits(&value(rows[1].property_value.as_ref().unwrap())),
        expected
    );
    assert_eq!(rows[2].property_array_index, None);
    let list = elements.last().unwrap();
    assert_eq!(list.property_array_index, Some(0));
    assert_eq!(
        value(list.property_value.as_ref().unwrap()),
        PropertyValue::Unsigned(listed.len() as u64)
    );
    let pics = wire
        .server
        .generate_pics(&crate::pics::PicsConfig::default())
        .await;
    let device = pics
        .supported_object_types
        .iter()
        .find(|row| row.object_type == ObjectType::DEVICE)
        .unwrap();
    for property in [ACTIVE, MULTIPLE, SERVICES, LIST] {
        let rows: Vec<_> = device
            .supported_properties
            .iter()
            .filter(|row| row.property_id == property)
            .collect();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].access.readable && !rows[0].access.writable);
        assert_eq!(
            rows[0].access.optional,
            [ACTIVE, MULTIPLE].contains(&property)
        );
    }
    let executors: Vec<_> = pics
        .supported_services
        .iter()
        .filter(|row| row.executor)
        .collect();
    assert_eq!(executors.len(), expected.len());
    for name in [
        "SubscribeCOV",
        "SubscribeCOVProperty",
        "SubscribeCOVPropertyMultiple",
    ] {
        assert!(executors.iter().any(|row| row.service_name == name));
    }
    for name in ["TimeSynchronization", "UTCTimeSynchronization"] {
        assert_eq!(executors.iter().any(|row| row.service_name == name), clock);
    }
}

#[tokio::test]
async fn device_execution_view_survives_public_profile_mutation_and_same_oid_replacement() {
    for clock in [true, false] {
        let mut wire = Wire::start(ServerConfig::default()).await;
        if !clock {
            wire.server.database().write().await.set_clock_reader(None);
        }
        wire.server
            .database()
            .write()
            .await
            .get_mut(&device())
            .unwrap()
            .device_authority_internal()
            .unwrap()
            .set_services_supported(&[ServiceSupported::VT_OPEN]);
        assert_profile(&mut wire, device(), clock).await;
        let mut replacement = DeviceObject::new(DeviceConfig {
            instance: device().instance_number(),
            ..Default::default()
        })
        .unwrap();
        replacement.set_services_supported(&[]);
        wire.server
            .database()
            .write()
            .await
            .add(Box::new(replacement))
            .unwrap();
        assert_profile(&mut wire, wildcard(), clock).await;
        for metadata in [false, true] {
            wire.server
                .database()
                .write()
                .await
                .add(Box::new(CustomDevice {
                    oid: device(),
                    metadata,
                }))
                .unwrap();
            assert_profile(&mut wire, device(), clock).await;
            assert_eq!(
                value(
                    &wire
                        .read(device(), PropertyIdentifier::DESCRIPTION, None)
                        .await
                        .unwrap()
                ),
                PropertyValue::CharacterString("custom-description".into())
            );
        }
        wire.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn device_execution_view_owns_selected_live_and_other_empty_cov_values() {
    let mut wire = Wire::start_with_devices(ServerConfig::default(), &[813, 900]).await;
    let other = ObjectIdentifier::new(ObjectType::DEVICE, 900).unwrap();
    for target in [device(), other] {
        let mut replacement = DeviceObject::new(DeviceConfig {
            instance: target.instance_number(),
            name: format!("declared-empty-{}", target.instance_number()),
            ..Default::default()
        })
        .unwrap();
        replacement.set_services_supported(&[]);
        wire.server
            .database()
            .write()
            .await
            .add(Box::new(replacement))
            .unwrap();
    }
    wire.server
        .database()
        .write()
        .await
        .add(Box::new(CustomDevice {
            oid: other,
            metadata: true,
        }))
        .unwrap();
    simple_ack(
        wire.send(&direct(), subscribe_cov(20, av(1), Some(false), Some(300)))
            .await,
    );
    simple_ack(
        wire.send(
            &direct(),
            subscribe_cov_property_multiple(
                21,
                false,
                Some((300, 1)),
                vec![(av(1), vec![plain(PV)])],
            ),
        )
        .await,
    );
    for property in [ACTIVE, MULTIPLE] {
        assert!(!wire
            .read(device(), property, None)
            .await
            .unwrap()
            .is_empty());
        assert!(wire.read(other, property, None).await.unwrap().is_empty());
        assert_eq!(
            wire.server
                .read_local(&other, property, None)
                .await
                .unwrap(),
            PropertyValue::ApplicationData(vec![])
        );
        let ack = wire
            .rpm(vec![
                (wildcard(), vec![(property, None)]),
                (other, vec![(property, None)]),
            ])
            .await;
        assert_eq!(
            ack.list_of_read_access_results[0].object_identifier,
            device()
        );
        assert!(!ack.list_of_read_access_results[0].list_of_results[0]
            .property_value
            .as_ref()
            .unwrap()
            .is_empty());
        assert!(ack.list_of_read_access_results[1].list_of_results[0]
            .property_value
            .as_ref()
            .unwrap()
            .is_empty());
    }
    assert_profile(&mut wire, other, true).await;
    wire.server.stop().await.unwrap();
}

#[test]
fn device_execution_view_rpm_budgets_count_canonical_rows_and_preserve_output() {
    use crate::device_view::{DeviceExecution, DeviceReadContext};
    use bacnet_services::rpm::ReadPropertyMultipleRequest;
    use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
    let mut db = ObjectDatabase::new();
    db.add(Box::new(CustomDevice {
        oid: device(),
        metadata: true,
    }))
    .unwrap();
    let view = DeviceReadContext::new(&db, DeviceExecution::FullServer);
    let request = |property| ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: device(),
            list_of_property_references: vec![PropertyReference {
                property_identifier: property,
                property_array_index: None,
            }],
        }],
    };
    for (request, budget, work_failure) in [
        (
            request(PropertyIdentifier::ALL),
            ReadPropertyMultipleBudget {
                max_result_elements: 3,
                max_service_ack_bytes: 4096,
            },
            true,
        ),
        (
            request(ACTIVE),
            ReadPropertyMultipleBudget {
                max_result_elements: 1,
                max_service_ack_bytes: 1,
            },
            false,
        ),
    ] {
        let mut out = BytesMut::from(&b"unchanged"[..]);
        let mut observations = 0;
        let result =
            crate::handlers::RpmPlan::new(&db, &request, budget.max_result_elements, Some(&view))
                .and_then(|plan| {
                    plan.read_observed(
                        &db,
                        Some(&view),
                        &mut out,
                        budget.max_service_ack_bytes,
                        |_, _, _, _| observations += 1,
                    )
                });
        assert!(matches!(
            (&result, work_failure),
            (Err(crate::handlers::ReadFailure::Work), true)
                | (Err(crate::handlers::ReadFailure::Bytes), false)
        ));
        assert_eq!(out.as_ref(), b"unchanged");
        assert_eq!(observations, 0);
    }
    let mut out = BytesMut::new();
    crate::handlers::RpmPlan::new(&db, &request(PropertyIdentifier::ALL), 5, Some(&view))
        .unwrap()
        .read_observed(&db, Some(&view), &mut out, 4096, |_, _, _, _| {})
        .unwrap();
    let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&out).unwrap();
    let properties: Vec<_> = ack.list_of_read_access_results[0]
        .list_of_results
        .iter()
        .map(|row| row.property_identifier)
        .collect();
    assert_eq!(
        properties,
        vec![
            PropertyIdentifier::DESCRIPTION,
            BINDINGS,
            SERVICES,
            ACTIVE,
            MULTIPLE
        ]
    );
}

#[test]
fn device_execution_view_keeps_context_free_helpers_on_raw_declared_profile() {
    let mut db = ObjectDatabase::new();
    let mut object = DeviceObject::new(DeviceConfig {
        instance: device().instance_number(),
        ..Default::default()
    })
    .unwrap();
    object.set_services_supported(&[ServiceSupported::READ_PROPERTY]);
    db.add(Box::new(object)).unwrap();
    let mut request = BytesMut::new();
    bacnet_services::read_property::ReadPropertyRequest {
        object_identifier: device(),
        property_identifier: ACTIVE,
        property_array_index: None,
    }
    .encode(&mut request);
    assert!(
        matches!(crate::handlers::handle_read_property(&db, &request, &mut BytesMut::new()), Err(Error::Protocol { class, code }) if class == ErrorClass::PROPERTY.to_raw() as u32 && code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32)
    );
    let mut request = BytesMut::new();
    bacnet_services::rpm::ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![bacnet_types::constructed::ReadAccessSpecification {
            object_identifier: device(),
            list_of_property_references: vec![bacnet_types::constructed::PropertyReference {
                property_identifier: PropertyIdentifier::ALL,
                property_array_index: None,
            }],
        }],
    }
    .encode(&mut request)
    .unwrap();
    let mut raw = BytesMut::new();
    crate::handlers::handle_read_property_multiple(&db, &request, &mut raw).unwrap();
    let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&raw).unwrap();
    let rows = &ack.list_of_read_access_results[0].list_of_results;
    assert!(!rows
        .iter()
        .any(|row| [ACTIVE, MULTIPLE].contains(&row.property_identifier)));
    let services = rows
        .iter()
        .find(|row| row.property_identifier == SERVICES)
        .unwrap();
    assert_eq!(
        service_bits(&value(services.property_value.as_ref().unwrap())),
        vec![12]
    );
    let raw_pics = crate::pics::PicsGenerator::new(
        &db,
        &ServerConfig::default(),
        &crate::pics::PicsConfig::default(),
    )
    .generate();
    assert_eq!(
        raw_pics
            .supported_services
            .iter()
            .filter(|row| row.executor)
            .map(|row| row.service_name.as_str())
            .collect::<Vec<_>>(),
        vec!["ReadProperty"]
    );
}

#[path = "device_execution_writes.rs"]
mod writes;

#[path = "device_profiles.rs"]
mod device_profiles;
