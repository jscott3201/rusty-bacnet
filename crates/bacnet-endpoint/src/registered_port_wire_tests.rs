//! Independent UDP oracle for the full-server and bounded endpoint owners.
use super::*;
use bacnet_encoding::{
    apdu::{Apdu, ComplexAck, ConfirmedRequest, decode_apdu, encode_apdu},
    primitives::{decode_application_value, encode_property_value},
};
use bacnet_services::{
    read_property::{ReadPropertyACK, ReadPropertyRequest},
    write_property::WritePropertyRequest,
};
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{ConfirmedServiceChoice as S, PropertyIdentifier as P};
use bytes::{Bytes, BytesMut};
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    time::Duration,
};
use tokio::net::UdpSocket;

fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}
fn port() -> ObjectIdentifier {
    oid(ObjectType::NETWORK_PORT, 2)
}
fn wildcard() -> ObjectIdentifier {
    oid(ObjectType::NETWORK_PORT, 4194303)
}
fn device() -> ObjectIdentifier {
    oid(ObjectType::DEVICE, 785)
}
fn identity() -> crate::DeviceIdentity {
    crate::DeviceIdentity::new(785, 555)
        .unwrap()
        .with_max_apdu(1476)
        .unwrap()
        .with_bip_port(1, 91, Ipv4Addr::LOCALHOST, 11)
        .unwrap()
        .with_bip_port(2, 17, Ipv4Addr::LOCALHOST, 0)
        .unwrap()
}
fn request(service: S, body: BytesMut) -> Bytes {
    static NEXT_INVOKE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);
    let invoke_id = NEXT_INVOKE.fetch_add(1, Ordering::Relaxed);
    let mut data = BytesMut::new();
    encode_apdu(
        &mut data,
        &Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_request: body.freeze(),
        }),
    )
    .unwrap();
    data.freeze()
}
fn rp(target: ObjectIdentifier, property: P) -> Bytes {
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: target,
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut body);
    request(S::READ_PROPERTY, body)
}
fn wp(text: &str) -> Bytes {
    let mut value = BytesMut::new();
    encode_property_value(&mut value, &PropertyValue::CharacterString(text.into())).unwrap();
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: device(),
        property_identifier: P::DESCRIPTION,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    request(S::WRITE_PROPERTY, body)
}
async fn exchange(peer: &UdpSocket, address: SocketAddrV4, apdu: &[u8]) -> Bytes {
    // Independent local BVLL/NPDU framing; no production encoder on this path.
    let length = apdu.len() + 6;
    let mut frame = vec![0x81, 0x0a, (length >> 8) as u8, length as u8, 1, 4];
    frame.extend_from_slice(apdu);
    peer.send_to(&frame, address).await.unwrap();
    let mut response = vec![0; 4096];
    let (size, source) =
        tokio::time::timeout(Duration::from_secs(3), peer.recv_from(&mut response))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "no response to APDU len={} header={:?}",
                    apdu.len(),
                    &apdu[..4.min(apdu.len())]
                )
            })
            .unwrap();
    assert_eq!(source, std::net::SocketAddr::V4(address));
    assert_eq!(&response[..2], &[0x81, 0x0a]);
    assert_eq!(
        u16::from_be_bytes([response[2], response[3]]) as usize,
        size
    );
    assert_eq!(response[4], 1);
    assert_eq!(response[5] & 0xf8, 0, "local NPDU has no routed fields");
    Bytes::copy_from_slice(&response[6..size])
}
async fn read(
    peer: &UdpSocket,
    address: SocketAddrV4,
    target: ObjectIdentifier,
    property: P,
) -> (ObjectIdentifier, PropertyValue) {
    let response = exchange(peer, address, &rp(target, property)).await;
    let Apdu::ComplexAck(ack) = decode_apdu(response).unwrap() else {
        panic!("expected RP ACK");
    };
    let ack = ReadPropertyACK::decode(&ack.service_ack).unwrap();
    (
        ack.object_identifier,
        decode_application_value(&ack.property_value, 0).unwrap().0,
    )
}

enum Owner {
    Server(Box<bacnet_server::server::BACnetServer<BipTransport>>),
    Endpoint(Box<EndpointSession<BipTransport>>),
}
impl Owner {
    async fn start(full: bool, registered: bool) -> Self {
        let identity = identity();
        let db = identity.build_database().unwrap();
        if full {
            let mut config = crate::identity::server_config(&identity);
            config.registered_network_port = registered.then_some(port());
            Self::Server(Box::new(
                bacnet_server::server::BACnetServer::start(
                    config,
                    db,
                    BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST),
                )
                .await
                .unwrap(),
            ))
        } else {
            let mut builder =
                crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
                    .role(SessionRole::ServerOnly)
                    .device(identity)
                    .database(db)
                    .device_writes(Arc::new(|_| true));
            if registered {
                builder = builder.registered_network_port(port());
            }
            let mut endpoint = builder.build_session().unwrap();
            endpoint.start().await.unwrap();
            Self::Endpoint(Box::new(endpoint))
        }
    }
    fn address(&self) -> SocketAddrV4 {
        match self {
            Self::Server(server) => {
                let (ip, port) =
                    bacnet_transport::bvll::decode_bip_mac(server.local_mac()).unwrap();
                SocketAddrV4::new(ip.into(), port)
            }
            Self::Endpoint(endpoint) => endpoint.bip_local_address().unwrap(),
        }
    }
    fn database(&self) -> Arc<RwLock<ObjectDatabase>> {
        match self {
            Self::Server(server) => server.database().clone(),
            Self::Endpoint(endpoint) => endpoint.database.as_ref().unwrap().clone(),
        }
    }
    async fn stop(&mut self) {
        match self {
            Self::Server(server) => server.stop().await.unwrap(),
            Self::Endpoint(endpoint) => {
                endpoint.stop().await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn registered_port_real_owners_snapshot_wildcard_and_release() {
    for full in [true, false] {
        port_retry::rerun_on_lost_port(async || {
            let mut owner = Owner::start(full, true).await;
            let address = owner.address();
            let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            assert_ne!(address.port(), 0);
            for (property, value) in [
                (
                    P::OBJECT_IDENTIFIER,
                    PropertyValue::ObjectIdentifier(port()),
                ),
                (
                    P::BACNET_IP_UDP_PORT,
                    PropertyValue::Unsigned(address.port().into()),
                ),
                (
                    P::MAC_ADDRESS,
                    PropertyValue::OctetString(
                        bacnet_transport::bvll::encode_bip_mac([127, 0, 0, 1], address.port())
                            .to_vec(),
                    ),
                ),
                (P::APDU_LENGTH, PropertyValue::Unsigned(1476)),
                (P::NETWORK_NUMBER, PropertyValue::Unsigned(17)),
            ] {
                assert_eq!(
                    read(&peer, address, wildcard(), property).await,
                    (port(), value)
                );
            }
            assert_eq!(
                read(
                    &peer,
                    address,
                    oid(ObjectType::NETWORK_PORT, 1),
                    P::NETWORK_NUMBER
                )
                .await
                .1,
                PropertyValue::Unsigned(91)
            );
            assert_eq!(
                read(
                    &peer,
                    address,
                    oid(ObjectType::DEVICE, 4194303),
                    P::OBJECT_IDENTIFIER
                )
                .await,
                (device(), PropertyValue::ObjectIdentifier(device()))
            );
            if let Owner::Endpoint(endpoint) = &owner {
                let selected = endpoint
                    .identity()
                    .unwrap()
                    .network_ports()
                    .iter()
                    .find(|entry| entry.instance == 2)
                    .unwrap();
                assert_eq!(selected.udp_port, address.port());
                assert_eq!(
                    selected.mac.as_slice(),
                    bacnet_transport::bvll::encode_bip_mac([127, 0, 0, 1], address.port())
                );
            }
            let db = owner.database();
            assert!(db.write().await.remove(&port()).is_err());
            assert!(
                db.write()
                    .await
                    .get_mut(&port())
                    .unwrap()
                    .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
                    .is_err()
            );
            owner.stop().await;
            assert!(db.write().await.remove(&port()).unwrap().is_some());
            std::net::UdpSocket::bind(address).map(drop)
        })
        .await;
    }
}

#[tokio::test]
async fn registered_port_none_does_not_choose_declared_object() {
    for full in [true, false] {
        let mut owner = Owner::start(full, false).await;
        let address = owner.address();
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let result = exchange(&peer, address, &rp(wildcard(), P::OBJECT_IDENTIFIER)).await;
        let Apdu::Error(error) = decode_apdu(result).unwrap() else {
            panic!("unregistered wildcard must fail");
        };
        assert_eq!(
            error.error_code,
            bacnet_types::enums::ErrorCode::UNKNOWN_OBJECT
        );
        assert_eq!(
            read(&peer, address, port(), P::BACNET_IP_UDP_PORT).await.1,
            PropertyValue::Unsigned(0)
        );
        owner.stop().await;
    }
}

#[tokio::test]
async fn registered_port_capacity_1476_valid_wp_and_rp_through_both_owners() {
    for full in [true, false] {
        let mut owner = Owner::start(full, true).await;
        let address = owner.address();
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let text = (1400..1476)
            .map(|n| "W".repeat(n))
            .find(|text| wp(text).len() == 1476)
            .unwrap();
        let encoded = wp(&text);
        assert_eq!(encoded.len(), 1476);
        assert!(matches!(
            decode_apdu(exchange(&peer, address, &encoded).await).unwrap(),
            Apdu::SimpleAck(_)
        ));
        assert_eq!(
            read(&peer, address, device(), P::DESCRIPTION).await.1,
            PropertyValue::CharacterString(text)
        );
        // Independently size the distinct RP ACK rather than padding an APDU.
        let text = (1400..1476)
            .map(|n| "R".repeat(n))
            .find(|text| {
                let mut value = BytesMut::new();
                encode_property_value(&mut value, &PropertyValue::CharacterString(text.clone()))
                    .unwrap();
                let mut body = BytesMut::new();
                ReadPropertyACK {
                    object_identifier: device(),
                    property_identifier: P::DESCRIPTION,
                    property_array_index: None,
                    property_value: value.to_vec(),
                }
                .encode(&mut body);
                let mut apdu = BytesMut::new();
                encode_apdu(
                    &mut apdu,
                    &Apdu::ComplexAck(ComplexAck {
                        segmented: false,
                        more_follows: false,
                        invoke_id: 1,
                        sequence_number: None,
                        proposed_window_size: None,
                        service_choice: S::READ_PROPERTY,
                        service_ack: body.freeze(),
                    }),
                )
                .unwrap();
                apdu.len() == 1476
            })
            .unwrap();
        owner
            .database()
            .write()
            .await
            .get_mut(&device())
            .unwrap()
            .device_authority_internal()
            .unwrap()
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString(text.clone()),
                None,
            )
            .unwrap();
        let ack = exchange(&peer, address, &rp(device(), P::DESCRIPTION)).await;
        assert_eq!(ack.len(), 1476); // exchange also asserts BVLL length = 1482.
        let Apdu::ComplexAck(ack) = decode_apdu(ack).unwrap() else {
            panic!("capacity RP ACK");
        };
        let value = ReadPropertyACK::decode(&ack.service_ack).unwrap();
        assert_eq!(
            decode_application_value(&value.property_value, 0)
                .unwrap()
                .0,
            PropertyValue::CharacterString(text)
        );
        owner.stop().await;
    }
}

#[tokio::test]
async fn registered_port_rpm_mixed_targets_and_missing_registration() {
    use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
    use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
    for registered in [true, false] {
        let mut owner = Owner::start(true, registered).await;
        let address = owner.address();
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut body = BytesMut::new();
        let objects = [
            oid(ObjectType::NETWORK_PORT, 1),
            wildcard(),
            oid(ObjectType::DEVICE, 4194303),
        ];
        ReadPropertyMultipleRequest {
            list_of_read_access_specs: objects
                .into_iter()
                .map(|object_identifier| ReadAccessSpecification {
                    object_identifier,
                    list_of_property_references: vec![PropertyReference {
                        property_identifier: P::OBJECT_IDENTIFIER,
                        property_array_index: None,
                    }],
                })
                .collect(),
        }
        .encode(&mut body)
        .unwrap();
        let result = exchange(&peer, address, &request(S::READ_PROPERTY_MULTIPLE, body)).await;
        let Apdu::ComplexAck(result) = decode_apdu(result).unwrap() else {
            panic!("mixed RPM must succeed overall");
        };
        let ack = ReadPropertyMultipleACK::decode(&result.service_ack).unwrap();
        assert_eq!(ack.list_of_read_access_results.len(), 3);
        for (index, result) in ack.list_of_read_access_results.iter().enumerate() {
            if index == 1 && !registered {
                assert_eq!(result.object_identifier, wildcard());
                assert_eq!(
                    result.list_of_results[0].error,
                    Some((
                        bacnet_types::enums::ErrorClass::OBJECT,
                        bacnet_types::enums::ErrorCode::UNKNOWN_OBJECT
                    ))
                );
            } else {
                let expected = [objects[0], port(), device()][index];
                assert_eq!(result.object_identifier, expected);
                assert_eq!(
                    decode_application_value(
                        result.list_of_results[0].property_value.as_ref().unwrap(),
                        0
                    )
                    .unwrap()
                    .0,
                    PropertyValue::ObjectIdentifier(expected)
                );
            }
        }
        owner.stop().await;
    }
}

#[tokio::test]
async fn registered_port_shared_database_does_not_register_other_ingress() {
    let mut owner = Owner::start(false, true).await;
    let mut other = EndpointIngress::new(
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST),
        8,
    );
    let mut receivers = other.start().await.unwrap();
    let address = receivers.bip_local_address.unwrap();
    let responder = EndpointResponder::new(owner.database(), receivers.egress);
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let request = rp(wildcard(), P::OBJECT_IDENTIFIER);
    let (reply, handled) = tokio::join!(exchange(&peer, address, &request), async {
        responder
            .handle(receivers.inbound_requests.recv().await.unwrap())
            .await
            .unwrap()
    });
    assert!(handled);
    let Apdu::Error(error) = decode_apdu(reply).unwrap() else {
        panic!("other ingress must not inherit DB registration");
    };
    assert_eq!(
        error.error_code,
        bacnet_types::enums::ErrorCode::UNKNOWN_OBJECT
    );
    assert_eq!(
        read(&peer, owner.address(), wildcard(), P::OBJECT_IDENTIFIER).await,
        (port(), PropertyValue::ObjectIdentifier(port()))
    );
    other.stop().await.unwrap();
    owner.stop().await;
}

#[tokio::test]
async fn registered_port_successful_audit_uses_concrete_targets_per_property() {
    use bacnet_objects::audit::AuditReporterObject;
    use bacnet_services::{audit::AuditNotificationRequest, rpm::ReadPropertyMultipleRequest};
    use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
    use bacnet_types::{
        bitstring::AuditOperationFlags,
        constructed::{BACnetAddress, BACnetRecipient},
        enums::{AuditLevel, AuditOperation},
    };
    let audit_peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let audit_port = audit_peer.local_addr().unwrap().port();
    let id = identity();
    let mut db = id.build_database().unwrap();
    db.get_mut(&device())
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: bacnet_types::MacAddr::from_slice(
                &bacnet_transport::bvll::encode_bip_mac([127, 0, 0, 1], audit_port),
            ),
        }))
        .unwrap();
    let mut reporter = AuditReporterObject::new(1, "read reporter").unwrap();
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    reporter.set_issue_confirmed_notifications(false).unwrap();
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::READ);
    reporter.set_auditable_operations(operations).unwrap();
    let reporter_oid = reporter.object_identifier();
    db.add(Box::new(reporter)).unwrap();
    let mut config = crate::identity::server_config(&id);
    config.registered_network_port = Some(port());
    config.audit_reporters = Some(bacnet_server::server::AuditReportersConfig {
        reporters: vec![reporter_oid],
    });
    let mut server = bacnet_server::server::BACnetServer::start(
        config,
        db,
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST),
    )
    .await
    .unwrap();
    let (_, udp) = bacnet_transport::bvll::decode_bip_mac(server.local_mac()).unwrap();
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, udp);
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    assert_eq!(
        read(&peer, address, wildcard(), P::OBJECT_IDENTIFIER)
            .await
            .0,
        port()
    );
    async fn notification(
        peer: &UdpSocket,
    ) -> Vec<bacnet_types::constructed::BACnetAuditNotification> {
        let mut frame = [0; 2048];
        let (length, _) = tokio::time::timeout(Duration::from_secs(3), peer.recv_from(&mut frame))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&frame[..2], &[0x81, 0x0a]);
        let Apdu::UnconfirmedRequest(request) =
            decode_apdu(Bytes::copy_from_slice(&frame[6..length])).unwrap()
        else {
            panic!("unconfirmed Audit wire request");
        };
        AuditNotificationRequest::decode(&request.service_request)
            .unwrap()
            .notifications
    }
    let audit = notification(&audit_peer).await;
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].target_object, Some(port()));
    let mut body = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: [wildcard(), device()]
            .into_iter()
            .map(|object_identifier| ReadAccessSpecification {
                object_identifier,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: P::OBJECT_IDENTIFIER,
                    property_array_index: None,
                }],
            })
            .collect(),
    }
    .encode(&mut body)
    .unwrap();
    assert!(matches!(
        decode_apdu(exchange(&peer, address, &request(S::READ_PROPERTY_MULTIPLE, body)).await)
            .unwrap(),
        Apdu::ComplexAck(_)
    ));
    let mut targets = vec![];
    while targets.len() < 2 {
        targets.extend(
            notification(&audit_peer)
                .await
                .into_iter()
                .map(|record| record.target_object.unwrap()),
        );
    }
    targets.sort_by_key(|oid| (oid.object_type().to_raw(), oid.instance_number()));
    let mut expected = vec![port(), device()];
    expected.sort_by_key(|oid| (oid.object_type().to_raw(), oid.instance_number()));
    assert_eq!(targets, expected);
    server.stop().await.unwrap();
}

#[path = "registered_port_boundary_tests.rs"]
mod boundaries;

#[tokio::test]
async fn registered_port_capacity_independent_local_bvll_framing() {
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let std::net::SocketAddr::V4(address) = peer.local_addr().unwrap() else {
        panic!()
    };
    let mut network = bacnet_network::layer::NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    ));
    let _received = network.start().await.unwrap();
    let apdu = vec![0x55; 1476];
    let mac = bacnet_transport::bvll::encode_bip_mac(address.ip().octets(), address.port());
    network
        .send_apdu(&apdu, &mac, false, bacnet_npdu::NetworkPriority::Normal)
        .await
        .unwrap();
    let mut frame = [0; 1500];
    let (size, _) = tokio::time::timeout(Duration::from_secs(3), peer.recv_from(&mut frame))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(size, 1482);
    assert_eq!(&frame[..6], &[0x81, 0x0a, 0x05, 0xca, 1, 0]);
    assert_eq!(&frame[6..size], &apdu);
    network.stop().await.unwrap();
}

#[path = "registered_port_routed_tests.rs"]
mod routed;
