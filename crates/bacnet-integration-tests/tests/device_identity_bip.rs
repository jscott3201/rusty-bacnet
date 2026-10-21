//! A frozen server identity across real B/IP I-Am, RP, RPM and local reads.
use bacnet_client::client::BACnetClient;
use bacnet_encoding::{
    apdu::{decode_apdu, Apdu},
    npdu::decode_npdu,
    primitives::decode_application_value,
};
use bacnet_objects::{
    database::ObjectDatabase,
    device::{DeviceConfig, DeviceObject},
    object_profile::ObjectProfile,
    DeviceIdentity,
};
use bacnet_server::server::{BACnetServer, DiscoveryPolicy, ServerConfig};
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::{
    bbmd::BdtEntry,
    bip::BipTransport,
    bvll::{decode_bvll, BvlcFunction},
};
use bacnet_types::{
    constructed::{PropertyReference, ReadAccessSpecification},
    enums::{
        BackupAndRestoreState, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P,
        Segmentation,
    },
    error::Error,
    primitives::{ObjectIdentifier, PropertyValue},
};
use std::{net::Ipv4Addr, time::Duration};
use tokio::net::UdpSocket;

async fn receive_iam(socket: &UdpSocket) -> IAmRequest {
    let mut bytes = [0; 2048];
    let (len, _) = tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    let frame = decode_bvll(&bytes[..len]).unwrap();
    assert!(matches!(
        frame.function,
        BvlcFunction::FORWARDED_NPDU | BvlcFunction::ORIGINAL_UNICAST_NPDU
    ));
    let Apdu::UnconfirmedRequest(request) =
        decode_apdu(decode_npdu(frame.npdu().unwrap()).unwrap().payload).unwrap()
    else {
        panic!("I-Am expected")
    };
    assert_eq!(request.service_choice.to_raw(), 0);
    IAmRequest::decode(&request.service_request).unwrap()
}

fn declaration(identity: &DeviceIdentity) -> Vec<(P, PropertyValue)> {
    let mut rows = vec![
        (
            P::OBJECT_IDENTIFIER,
            PropertyValue::ObjectIdentifier(identity.device_oid()),
        ),
        (P::VENDOR_IDENTIFIER, PropertyValue::Unsigned(42)),
        (P::MAX_APDU_LENGTH_ACCEPTED, PropertyValue::Unsigned(1474)),
        (
            P::SEGMENTATION_SUPPORTED,
            PropertyValue::Enumerated(identity.segmentation().to_raw().into()),
        ),
    ];
    if identity.segmentation() != Segmentation::NONE {
        rows.push((P::APDU_SEGMENT_TIMEOUT, PropertyValue::Unsigned(700)));
        rows.push((
            P::MAX_SEGMENTS_ACCEPTED,
            PropertyValue::Unsigned(if identity.segmentation() == Segmentation::TRANSMIT {
                1
            } else {
                65
            }),
        ));
    }
    rows
}

async fn assert_views(
    server: &BACnetServer<BipTransport>,
    client: &BACnetClient<BipTransport>,
    identity: &DeviceIdentity,
    target: ObjectIdentifier,
) {
    let rows = declaration(identity);
    let oid = identity.device_oid();
    for (property, expected) in &rows {
        assert_eq!(
            server.read_local(&target, *property, None).await.unwrap(),
            *expected
        );
        let ack = client
            .read_property(server.local_mac(), target, *property, None)
            .await
            .unwrap();
        assert_eq!(ack.object_identifier, oid);
        assert_eq!(
            decode_application_value(&ack.property_value, 0).unwrap().0,
            *expected
        );
    }
    let ack = client
        .read_property_multiple(
            server.local_mac(),
            vec![ReadAccessSpecification {
                object_identifier: target,
                list_of_property_references: rows
                    .iter()
                    .map(|(p, _)| PropertyReference {
                        property_identifier: *p,
                        property_array_index: None,
                    })
                    .collect(),
            }],
        )
        .await
        .unwrap();
    assert_eq!(ack.list_of_read_access_results[0].object_identifier, oid);
    for (result, (property, expected)) in ack.list_of_read_access_results[0]
        .list_of_results
        .iter()
        .zip(rows)
    {
        assert_eq!(result.property_identifier, property);
        assert_eq!(
            decode_application_value(result.property_value.as_ref().unwrap(), 0)
                .unwrap()
                .0,
            expected
        );
    }
    let PropertyValue::List(list) = server
        .read_local(&target, P::PROPERTY_LIST, None)
        .await
        .unwrap()
    else {
        panic!("list")
    };
    for property in [P::APDU_SEGMENT_TIMEOUT, P::MAX_SEGMENTS_ACCEPTED] {
        assert_eq!(
            list.contains(&PropertyValue::Enumerated(property.to_raw())),
            identity.segmentation() != Segmentation::NONE
        );
        if identity.segmentation() == Segmentation::NONE {
            assert!(client
                .read_property(server.local_mac(), target, property, None)
                .await
                .is_err());
        }
    }
}

#[tokio::test]
async fn bip_identity_matches_discovery_reads_and_preserves_supplied_device_state() {
    for mode in [
        Segmentation::NONE,
        Segmentation::TRANSMIT,
        Segmentation::RECEIVE,
        Segmentation::BOTH,
    ] {
        let identity = DeviceIdentity::new(1720, 42)
            .unwrap()
            .with_max_apdu(1474)
            .unwrap()
            .with_segmentation(mode)
            .with_apdu_segment_timeout_ms(700);
        let oid = identity.device_oid();
        let mut device = DeviceObject::new(DeviceConfig {
            identity: identity.clone(),
            backup_and_restore: true,
            backup_failure_timeout: 91,
            ..Default::default()
        })
        .unwrap();
        device.set_configuration_files(vec![ObjectIdentifier::new(ObjectType::FILE, 7).unwrap()]);
        device.set_backup_and_restore_state(BackupAndRestoreState::PERFORMING_A_BACKUP);
        device
            .set_profile(ObjectProfile {
                profile_name: Some("42-preserved".into()),
                ..Default::default()
            })
            .unwrap();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(device)).unwrap();
        let sink = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
        transport.enable_bbmd(vec![BdtEntry {
            ip: Ipv4Addr::LOCALHOST.octets(),
            port: sink.local_addr().unwrap().port(),
            broadcast_mask: [255; 4],
        }]);
        let mut server = BACnetServer::start(
            ServerConfig {
                device: identity.clone(),
                discovery_policy: DiscoveryPolicy::unlimited(),
                ..Default::default()
            },
            db,
            transport,
        )
        .await
        .unwrap();
        let mut client = BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .broadcast_address(Ipv4Addr::LOCALHOST)
            .apdu_timeout_ms(2000)
            .build()
            .await
            .unwrap();
        for (property, expected) in [
            (P::BACKUP_FAILURE_TIMEOUT, PropertyValue::Unsigned(91)),
            (
                P::BACKUP_AND_RESTORE_STATE,
                PropertyValue::Enumerated(BackupAndRestoreState::PERFORMING_A_BACKUP.to_raw()),
            ),
            (
                P::PROFILE_NAME,
                PropertyValue::CharacterString("42-preserved".into()),
            ),
            (
                P::CONFIGURATION_FILES,
                PropertyValue::List(vec![PropertyValue::ObjectIdentifier(
                    ObjectIdentifier::new(ObjectType::FILE, 7).unwrap(),
                )]),
            ),
        ] {
            assert_eq!(
                server.read_local(&oid, property, None).await.unwrap(),
                expected
            );
        }
        for phase in 0..3 {
            if phase == 1 {
                let mut db = server.database().write().await;
                db.remove(&oid).unwrap();
                db.add(Box::new(
                    DeviceObject::new(DeviceConfig {
                        identity: DeviceIdentity::new(1720, 99)
                            .unwrap()
                            .with_max_apdu(480)
                            .unwrap()
                            .with_segmentation(if mode == Segmentation::NONE {
                                Segmentation::BOTH
                            } else {
                                Segmentation::NONE
                            }),
                        ..Default::default()
                    })
                    .unwrap(),
                ))
                .unwrap();
            } else if phase == 2 {
                let mut db = server.database().write().await;
                db.add(Box::new(
                    DeviceObject::new(DeviceConfig {
                        identity: DeviceIdentity::new(1, 17)
                            .unwrap()
                            .with_name("lower-device"),
                        ..Default::default()
                    })
                    .unwrap(),
                ))
                .unwrap();
                assert_eq!(db.selected_device().unwrap().instance_number(), 1);
            }
            server.broadcast_i_am().await.unwrap();
            let manual = receive_iam(&sink).await;
            let (ip, port) = bacnet_transport::bvll::decode_bip_mac(server.local_mac()).unwrap();
            // Independent unicast Who-Is: BVLL, two-byte NPDU, service choice 8.
            sink.send_to(
                &[0x81, 0x0a, 0, 8, 1, 0, 0x10, 8],
                (Ipv4Addr::from(ip), port),
            )
            .await
            .unwrap();
            let requested = receive_iam(&sink).await;
            for iam in [manual, requested] {
                assert_eq!(iam.object_identifier, oid);
                assert_eq!(iam.vendor_id, 42);
                assert_eq!(iam.max_apdu_length, 1474);
                assert_eq!(iam.segmentation_supported, mode);
            }
            for target in [
                oid,
                ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::MAX_INSTANCE).unwrap(),
            ] {
                assert_views(&server, &client, &identity, target).await;
            }
        }
        server.database().write().await.remove(&oid).unwrap();
        let sent = server.discovery_counters().i_am_sent;
        assert!(server.broadcast_i_am().await.is_err());
        assert_eq!(server.discovery_counters().i_am_sent, sent);
        for target in [
            oid,
            ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::MAX_INSTANCE).unwrap(),
        ] {
            for result in [
                server
                    .read_local(&target, P::VENDOR_IDENTIFIER, None)
                    .await
                    .map(|_| ()),
                client
                    .read_property(server.local_mac(), target, P::VENDOR_IDENTIFIER, None)
                    .await
                    .map(|_| ()),
            ] {
                assert!(
                    matches!(result, Err(Error::Protocol { class, code })
                    if class == ErrorClass::OBJECT && code == ErrorCode::UNKNOWN_OBJECT),
                    "{result:?}"
                );
            }
            let ack = client
                .read_property_multiple(
                    server.local_mac(),
                    vec![ReadAccessSpecification {
                        object_identifier: target,
                        list_of_property_references: vec![PropertyReference {
                            property_identifier: P::VENDOR_IDENTIFIER,
                            property_array_index: None,
                        }],
                    }],
                )
                .await
                .unwrap();
            assert_eq!(
                ack.list_of_read_access_results[0].list_of_results[0].error,
                Some((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT))
            );
        }
        server
            .database()
            .write()
            .await
            .add(Box::new(
                DeviceObject::new(DeviceConfig {
                    identity: DeviceIdentity::new(1720, 99).unwrap(),
                    ..Default::default()
                })
                .unwrap(),
            ))
            .unwrap();
        assert_views(
            &server,
            &client,
            &identity,
            ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::MAX_INSTANCE).unwrap(),
        )
        .await;
        server.broadcast_i_am().await.unwrap();
        assert_eq!(receive_iam(&sink).await.object_identifier, oid);
        client.stop().await.unwrap();
        server.stop().await.unwrap();
    }
}
