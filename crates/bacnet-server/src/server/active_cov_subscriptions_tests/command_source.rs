//! Actual ingress, correlation, local authority and rollback for command sources.
use super::*;
use crate::LocalCommandSource;
use bacnet_encoding::constructed::{decode_value_source, encode_value_source};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetValueSource};
const SOURCE: PropertyIdentifier = PropertyIdentifier::VALUE_SOURCE;
const SOURCES: PropertyIdentifier = PropertyIdentifier::VALUE_SOURCE_ARRAY;
const TIME: PropertyIdentifier = PropertyIdentifier::LAST_COMMAND_TIME;

fn object_source(object: ObjectIdentifier) -> BACnetValueSource {
    BACnetValueSource::Object(BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: object,
    })
}
fn claimed(source: &BACnetValueSource) -> PropertyValue {
    let mut bytes = BytesMut::new();
    encode_value_source(&mut bytes, source).unwrap();
    PropertyValue::ApplicationData(bytes.to_vec())
}
fn wp(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
    priority: u8,
) -> (ConfirmedServiceChoice, BytesMut) {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut encoded, &value).unwrap();
    let mut bytes = BytesMut::new();
    WritePropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: None,
        property_value: encoded.to_vec(),
        priority: Some(priority),
    }
    .encode(&mut bytes)
    .unwrap();
    (ConfirmedServiceChoice::WRITE_PROPERTY, bytes)
}
async fn source(
    wire: &mut Wire,
    object: ObjectIdentifier,
    index: Option<u32>,
) -> BACnetValueSource {
    let bytes = wire
        .read(
            object,
            if index.is_some() { SOURCES } else { SOURCE },
            index,
        )
        .await
        .unwrap();
    let (source, end) = decode_value_source(&bytes, 0).unwrap();
    assert_eq!(end, bytes.len());
    source
}
async fn bind(wire: &Wire, peer: &Peer, instance: u32, ago: Duration) {
    let oid = ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap();
    wire.server.device_bindings.write().await.observe_i_am_at(
        oid,
        &peer.mac,
        peer.network.as_ref(),
        runtime_clock::now() - ago,
        |_| false,
    );
}
fn address_source(peer: &Peer) -> BACnetValueSource {
    BACnetValueSource::Address(BACnetAddress {
        network_number: peer.network.as_ref().map_or(0, |p| p.network),
        mac_address: peer
            .network
            .as_ref()
            .map_or_else(|| MacAddr::from_slice(&peer.mac), |p| p.mac_address.clone()),
    })
}

#[tokio::test]
async fn command_source_wire_origin_correlation_and_original_owner_matrix() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    let a = routed();
    let mut b = a.clone();
    b.mac = vec![99];
    let mut elsewhere = a.clone();
    elsewhere.network.as_mut().unwrap().mac_address = MacAddr::from_slice(&[77]);
    let device9 = ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap();
    // Audit reporting is absent. Unknown source is the original address, not router.
    simple_ack(
        wire.send(&a, wp(av(1), PV, PropertyValue::Real(31.0), 8))
            .await,
    );
    assert_eq!(source(&mut wire, av(1), None).await, address_source(&a));
    assert_eq!(wire.read(av(1), TIME, None).await.unwrap(), [0x19, 1]);
    let forwarded = BACnetValueSource::Object(BACnetDeviceObjectReference {
        device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap()),
        object_identifier: av(50),
    });
    simple_ack(
        wire.send(&b, wp(av(1), SOURCE, claimed(&forwarded), 8))
            .await,
    );
    assert_eq!(source(&mut wire, av(1), None).await, forwarded);
    // A later unique binding cannot grant the originally unknown writer cross-address rights.
    bind(&wire, &a, 9, Duration::ZERO).await;
    bind(&wire, &elsewhere, 9, Duration::ZERO).await;
    error(
        wire.send(
            &elsewhere,
            wp(av(1), SOURCE, claimed(&BACnetValueSource::None), 8),
        )
        .await,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    // New command replaces the original token with unique Device 9.
    simple_ack(
        wire.send(&elsewhere, wp(av(1), PV, PropertyValue::Real(32.0), 8))
            .await,
    );
    assert_eq!(source(&mut wire, av(1), None).await, object_source(device9));
    bind(&wire, &a, 9, Duration::ZERO).await;
    simple_ack(
        wire.send(&a, wp(av(1), SOURCE, claimed(&object_source(av(77))), 8))
            .await,
    );
    assert_eq!(wire.read(av(1), TIME, None).await.unwrap(), [0x19, 2]);
    // Expiry falls back to retained address, not the Device claim in the payload.
    bind(&wire, &a, 9, Duration::from_secs(601)).await;
    error(
        wire.send(&a, wp(av(1), SOURCE, claimed(&BACnetValueSource::None), 8))
            .await,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    simple_ack(
        wire.send(
            &elsewhere,
            wp(av(1), SOURCE, claimed(&BACnetValueSource::None), 8),
        )
        .await,
    );
    // Conflicting unique identity at retained address denies; ambiguity denies all correction.
    bind(&wire, &elsewhere, 10, Duration::ZERO).await;
    error(
        wire.send(&elsewhere, wp(av(1), SOURCE, claimed(&forwarded), 8))
            .await,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    bind(&wire, &elsewhere, 9, Duration::ZERO).await;
    error(
        wire.send(&elsewhere, wp(av(1), SOURCE, claimed(&forwarded), 8))
            .await,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    simple_ack(
        wire.send(&elsewhere, wp(av(1), PV, PropertyValue::Real(33.0), 8))
            .await,
    );
    assert_eq!(
        source(&mut wire, av(1), None).await,
        address_source(&elsewhere)
    );
    error(
        wire.send(&elsewhere, wp(av(1), SOURCE, claimed(&forwarded), 8))
            .await,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    // Direct source uses network 0; remote and local writers cannot correct one another.
    simple_ack(
        wire.send(&direct(), wp(av(2), PV, PropertyValue::Real(40.0), 16))
            .await,
    );
    assert_eq!(
        source(&mut wire, av(2), None).await,
        address_source(&direct())
    );
    assert!(wire
        .server
        .write_local(
            &av(2),
            SOURCE,
            None,
            claimed(&forwarded),
            None,
            LocalCommandSource::ServerDevice
        )
        .await
        .is_err());
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn command_source_local_required_identity_membership_and_owner_replacement() {
    for instances in [vec![], vec![ObjectIdentifier::WILDCARD_INSTANCE], vec![813]] {
        let mut wire = Wire::start_with_devices(ServerConfig::default(), &instances).await;
        let has_device = instances == [813];
        for initiating in [
            LocalCommandSource::ServerDevice,
            LocalCommandSource::Object(av(2)),
        ] {
            let result = wire
                .server
                .write_local(
                    &av(1),
                    PV,
                    None,
                    PropertyValue::Real(41.0),
                    None,
                    initiating,
                )
                .await;
            assert_eq!(result.is_ok(), has_device);
            if has_device {
                assert_eq!(
                    source(&mut wire, av(1), None).await,
                    object_source(match initiating {
                        LocalCommandSource::ServerDevice => device(),
                        LocalCommandSource::Object(oid) => oid,
                    })
                );
            }
        }
        assert!(wire
            .server
            .write_local(
                &av(1),
                PV,
                None,
                PropertyValue::Real(99.0),
                None,
                LocalCommandSource::Object(av(999))
            )
            .await
            .is_err());
        // Unrelated writes preserve no-Device behavior even when the selected source is unavailable.
        wire.server
            .write_local(
                &av(1),
                PropertyIdentifier::DESCRIPTION,
                None,
                PropertyValue::CharacterString("local".into()),
                None,
                LocalCommandSource::ServerDevice,
            )
            .await
            .unwrap();
        if has_device {
            wire.server
                .write_local(
                    &av(1),
                    SOURCE,
                    None,
                    claimed(&BACnetValueSource::None),
                    None,
                    LocalCommandSource::ServerDevice,
                )
                .await
                .unwrap();
            error(
                wire.send(
                    &direct(),
                    wp(av(1), SOURCE, claimed(&object_source(device())), 16),
                )
                .await,
                ErrorClass::PROPERTY,
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            wire.server
                .database()
                .write()
                .await
                .remove(&device())
                .unwrap();
            wire.server
                .database()
                .write()
                .await
                .add(Box::new(
                    DeviceObject::new(DeviceConfig {
                        instance: 900,
                        ..DeviceConfig::default()
                    })
                    .unwrap(),
                ))
                .unwrap();
            assert!(wire
                .server
                .write_local(
                    &av(1),
                    SOURCE,
                    None,
                    claimed(&object_source(device())),
                    None,
                    LocalCommandSource::ServerDevice
                )
                .await
                .is_err());
        }
        wire.server.stop().await.unwrap();
    }
}

fn initial(
    property: PropertyIdentifier,
    value: PropertyValue,
    priority: u8,
) -> BACnetPropertyValue {
    let mut bytes = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut bytes, &value).unwrap();
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: bytes.to_vec(),
        priority: Some(priority),
    }
}
#[tokio::test]
async fn command_source_wire_wpm_prefix_and_create_initial_values_rollback() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    let mut bytes = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: av(1),
            list_of_properties: vec![
                initial(PV, PropertyValue::Real(65.0), 8),
                initial(SOURCE, PropertyValue::ApplicationData(vec![0x08, 0x08]), 8),
                initial(PV, PropertyValue::Real(66.0), 8),
            ],
        }],
    }
    .encode(&mut bytes)
    .unwrap();
    let Apdu::Error(response) = wire
        .send(
            &routed(),
            (ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, bytes),
        )
        .await
    else {
        panic!("WPM must fail at trailing source data")
    };
    let failure =
        bacnet_services::wpm::WritePropertyMultipleError::decode(&response.error_data).unwrap();
    assert_eq!(
        failure.first_failed_write_attempt.property_identifier,
        SOURCE.to_raw()
    );
    assert_eq!(failure.first_failed_write_attempt.object_identifier, av(1));
    assert_eq!(
        wire.server.read_local(&av(1), PV, None).await.unwrap(),
        PropertyValue::Real(65.0)
    );
    assert_eq!(
        source(&mut wire, av(1), None).await,
        address_source(&routed())
    );
    assert_eq!(wire.read(av(1), TIME, None).await.unwrap(), [0x19, 1]);
    for (kind, value) in [
        (ObjectType::ANALOG_OUTPUT, PropertyValue::Real(2.0)),
        (ObjectType::BINARY_OUTPUT, PropertyValue::Enumerated(1)),
        (ObjectType::BINARY_VALUE, PropertyValue::Enumerated(1)),
        (ObjectType::MULTI_STATE_OUTPUT, PropertyValue::Unsigned(1)),
        (ObjectType::MULTI_STATE_VALUE, PropertyValue::Unsigned(1)),
    ] {
        for fail in [false, true] {
            let oid = ObjectIdentifier::new(kind, if fail { 825 } else { 824 }).unwrap();
            let mut values = vec![initial(PV, value.clone(), 8)];
            if fail {
                values.push(initial(
                    SOURCE,
                    PropertyValue::ApplicationData(vec![0x08, 0x08]),
                    8,
                ));
            }
            let mut bytes = BytesMut::new();
            CreateObjectRequest {
                object_specifier: ObjectSpecifier::Identifier(oid),
                list_of_initial_values: values,
            }
            .encode(&mut bytes);
            let response = wire
                .send(&routed(), (ConfirmedServiceChoice::CREATE_OBJECT, bytes))
                .await;
            if fail {
                assert!(matches!(response, Apdu::Error(_)), "{response:?}");
                assert!(wire.server.database().read().await.get(&oid).is_none());
            } else {
                assert!(matches!(response, Apdu::ComplexAck(_)), "{response:?}");
                assert_eq!(
                    source(&mut wire, oid, None).await,
                    address_source(&routed())
                );
                assert_eq!(
                    source(&mut wire, oid, Some(8)).await,
                    address_source(&routed())
                );
            }
        }
    }
    wire.server.stop().await.unwrap();
}
