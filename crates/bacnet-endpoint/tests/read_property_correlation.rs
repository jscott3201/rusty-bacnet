//! Public endpoint correlation against the bundled server over real B/IP.
use bacnet_endpoint::{bip::BipEndpointBuilder, DeviceIdentity, SessionRole};
use bacnet_server::server::{BACnetServer, ServerConfig};
use bacnet_transport::bip::BipTransport;
use bacnet_types::{
    enums::{ObjectType, PropertyIdentifier},
    primitives::ObjectIdentifier,
};
use std::net::Ipv4Addr;

#[tokio::test]
async fn endpoint_device_wildcard_accepts_bundled_server_concrete_ack() {
    let database = DeviceIdentity::new(10, 42)
        .unwrap()
        .build_database()
        .unwrap();
    let mut server = BACnetServer::start_clockless(
        ServerConfig::default(),
        database,
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST),
    )
    .await
    .unwrap();
    let mut endpoint = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::ClientOnly)
        .build_session()
        .unwrap();
    endpoint.start().await.unwrap();
    let ack = endpoint
        .client()
        .unwrap()
        .read_property(
            server.local_mac(),
            ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::MAX_INSTANCE).unwrap(),
            PropertyIdentifier::OBJECT_IDENTIFIER,
            None,
        )
        .await
        .expect("valid concrete Device ACK for wildcard request");
    assert_eq!(
        ack.object_identifier,
        ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap()
    );
    assert_eq!(ack.property_value, [0xc4, 0x02, 0x00, 0x00, 0x0a]);
    endpoint.stop().await.unwrap();
    server.stop().await.unwrap();
}

use bacnet_client::client::BACnetClient;
use bacnet_encoding::{
    apdu::{decode_apdu, encode_apdu, Apdu, ComplexAck},
    npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress},
};
use bacnet_endpoint::{EndpointSession, SessionConfig};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::{loopback::LoopbackTransport, port::TransportPort};
use bacnet_types::{enums::ConfirmedServiceChoice, error::Error, MacAddr};
use bytes::BytesMut;
use tokio::time::{timeout, Duration};

fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

enum Reader {
    Standalone(BACnetClient<LoopbackTransport>),
    Endpoint(Box<EndpointSession<LoopbackTransport>>),
}
impl Reader {
    async fn read(
        &self,
        routed: bool,
        request: &ReadPropertyRequest,
    ) -> Result<ReadPropertyACK, Error> {
        let (object, property, index) = (
            request.object_identifier,
            request.property_identifier,
            request.property_array_index,
        );
        match self {
            Self::Standalone(client) if routed => {
                client
                    .read_property_routed(&[2], 100, &[3], object, property, index)
                    .await
            }
            Self::Standalone(client) => client.read_property(&[2], object, property, index).await,
            Self::Endpoint(session) if routed => {
                session
                    .client()
                    .unwrap()
                    .read_property_routed(&[2], 100, &[3], vec![], object, property, index)
                    .await
            }
            Self::Endpoint(session) => {
                session
                    .client()
                    .unwrap()
                    .read_property(&[2], object, property, index)
                    .await
            }
        }
    }
    async fn stop(&mut self) {
        match self {
            Self::Standalone(client) => client.stop().await.unwrap(),
            Self::Endpoint(session) => {
                assert_eq!(session.active_leases(), 0);
                session.stop().await.unwrap();
            }
        }
    }
}

async fn correlation_matrix(endpoint: bool, routed: bool) {
    let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut frames = peer.start().await.unwrap();
    let mut reader = if endpoint {
        let mut session =
            EndpointSession::new(transport, SessionRole::ClientOnly, SessionConfig::default())
                .unwrap();
        session.start().await.unwrap();
        Reader::Endpoint(Box::new(session))
    } else {
        Reader::Standalone(
            BACnetClient::generic_builder()
                .transport(transport)
                .build()
                .await
                .unwrap(),
        )
    };
    let concrete = oid(ObjectType::ANALOG_INPUT, 7);
    let device_alias = oid(ObjectType::DEVICE, ObjectIdentifier::MAX_INSTANCE);
    let port_alias = oid(ObjectType::NETWORK_PORT, ObjectIdentifier::MAX_INSTANCE);
    let mut cases = vec![
        (concrete, concrete, true),
        (concrete, oid(ObjectType::ANALOG_INPUT, 8), false),
        (concrete, oid(ObjectType::ANALOG_OUTPUT, 7), false),
        (device_alias, oid(ObjectType::DEVICE, 0), true),
        (
            device_alias,
            oid(
                ObjectType::DEVICE,
                ObjectIdentifier::MAX_ADDRESSABLE_INSTANCE,
            ),
            true,
        ),
        (device_alias, device_alias, false),
        (device_alias, oid(ObjectType::NETWORK_PORT, 10), false),
        (port_alias, oid(ObjectType::NETWORK_PORT, 2), true),
        (port_alias, port_alias, false),
        (port_alias, oid(ObjectType::DEVICE, 10), false),
        (
            oid(ObjectType::ANALOG_INPUT, ObjectIdentifier::MAX_INSTANCE),
            concrete,
            false,
        ),
    ];
    // Wrong property/index/malformed payload cases follow the valid object cases.
    cases.extend([(concrete, concrete, false); 4]);
    let last = cases.len();
    for (i, (requested, reported, valid)) in cases.into_iter().enumerate() {
        let request = ReadPropertyRequest {
            object_identifier: requested,
            property_identifier: PropertyIdentifier::OBJECT_LIST,
            property_array_index: Some(0),
        };
        let responding = async {
            let frame = timeout(Duration::from_secs(2), frames.recv())
                .await
                .unwrap()
                .unwrap();
            let npdu = decode_npdu(frame.npdu).unwrap();
            assert_eq!(
                npdu.destination
                    .as_ref()
                    .map(|d| (d.network, d.mac_address.as_slice())),
                routed.then_some((100, &[3][..]))
            );
            let Apdu::ConfirmedRequest(wire) = decode_apdu(npdu.payload).unwrap() else {
                panic!("request");
            };
            let received = ReadPropertyRequest::decode(&wire.service_request).unwrap();
            assert_eq!(received.object_identifier, requested);
            let ack = ReadPropertyACK {
                object_identifier: reported,
                property_identifier: if i == last - 4 {
                    PropertyIdentifier::OBJECT_NAME
                } else {
                    request.property_identifier
                },
                property_array_index: if i == last - 3 {
                    None
                } else if i == last - 2 {
                    Some(1)
                } else {
                    Some(0)
                },
                property_value: vec![0x21, 42],
            };
            let mut body = BytesMut::new();
            if i != last - 1 {
                ack.encode(&mut body);
            }
            let mut apdu = BytesMut::new();
            encode_apdu(
                &mut apdu,
                &Apdu::ComplexAck(ComplexAck {
                    segmented: false,
                    more_follows: false,
                    invoke_id: wire.invoke_id,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                    service_ack: body.freeze(),
                }),
            )
            .unwrap();
            let mut bytes = BytesMut::new();
            encode_npdu(
                &mut bytes,
                &Npdu {
                    source: routed.then_some(NpduAddress {
                        network: 100,
                        mac_address: MacAddr::from_slice(&[3]),
                    }),
                    payload: apdu.freeze(),
                    ..Npdu::default()
                },
            )
            .unwrap();
            peer.send_unicast(&bytes, &[1]).await.unwrap();
        };
        let (result, ()) = tokio::join!(reader.read(routed, &request), responding);
        if valid {
            let ack = result.unwrap();
            assert_eq!(ack.object_identifier, reported);
            assert_eq!(ack.property_value, [0x21, 42]);
        } else {
            assert!(
                matches!(result, Err(Error::Decoding { .. })),
                "case{i}: {result:?}"
            );
        }
        if let Reader::Endpoint(session) = &reader {
            assert_eq!(session.active_leases(), 0, "case{i} leaked lease");
        }
    }
    reader.stop().await;
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn standalone_direct_read_property_correlates_ack_identity() {
    correlation_matrix(false, false).await;
}
#[tokio::test]
async fn standalone_routed_read_property_correlates_ack_identity() {
    correlation_matrix(false, true).await;
}
#[tokio::test]
async fn endpoint_direct_read_property_correlates_ack_identity() {
    correlation_matrix(true, false).await;
}
#[tokio::test]
async fn endpoint_routed_read_property_correlates_ack_identity() {
    correlation_matrix(true, true).await;
}
