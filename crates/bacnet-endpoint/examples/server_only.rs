//! Endpoint server-only example (B/IP, real loopback UDP).
//!
//! Old path: `BACnetServer::bip_builder()...build().await`.
//! New path: `BipEndpointBuilder::role(ServerOnly)` + identity + `start()`.
//!
//! Run with: `cargo run -p bacnet-endpoint --example server_only`

use std::net::Ipv4Addr;
use std::time::Duration;

use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu, ConfirmedRequest as ConfirmedPdu};
use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::identity::{build_database_with_extra, DeviceIdentity};
use bacnet_endpoint::session::SessionRole;
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{
    ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier,
};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use bytes::BytesMut;

const WAIT: Duration = Duration::from_secs(5);

fn oid(t: ObjectType, i: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(t, i).unwrap()
}

/// Probes one free loopback port so the example knows the endpoint MAC
/// (127.0.0.1 + port) without capturing a broadcast.
fn free_port() -> u16 {
    std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("probe bind")
        .local_addr()
        .expect("probe addr")
        .port()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Single Device identity: I-Am + Device readback + role limits agree.
    let port = free_port();
    let identity = DeviceIdentity::new(1001, 42)?.with_bip_port(1, 0, Ipv4Addr::LOCALHOST, port)?;
    let mut point = AnalogInputObject::new(1, "server-ai-1", 0)?;
    point.set_present_value(42.0);
    let db = build_database_with_extra(&identity, vec![Box::new(point)])?;

    let mut endpoint = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST)
        .role(SessionRole::ServerOnly)
        .database(db)
        .identity(identity.clone())
        .build_session()?;
    endpoint.start().await?;
    assert!(endpoint.server().is_some());
    assert!(endpoint.client().is_none());
    assert!(endpoint.is_running());

    // Endpoint MAC is the bound IP + port (single socket, known here).
    let endpoint_mac = MacAddr::from_slice(&[127, 0, 0, 1, (port >> 8) as u8, (port & 0xff) as u8]);

    // Controlled peer requests present-value from the endpoint server.
    let peer_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut peer_net = NetworkLayer::new(peer_transport);
    let mut peer_rx = peer_net.start().await?;

    let mut svc = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut svc);
    let mut enc = BytesMut::new();
    encode_apdu(
        &mut enc,
        &Apdu::ConfirmedRequest(ConfirmedPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 7,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: svc.freeze(),
        }),
    )?;
    peer_net
        .send_apdu(&enc, &endpoint_mac, true, NetworkPriority::NORMAL)
        .await?;
    let reply = tokio::time::timeout(WAIT, peer_rx.recv())
        .await?
        .ok_or("no reply")?;
    assert_eq!(reply.source_mac, endpoint_mac, "single socket, single MAC");
    match decode_apdu(reply.apdu)? {
        Apdu::ComplexAck(a) => {
            assert_eq!(a.invoke_id, 7);
            let decoded = ReadPropertyACK::decode(&a.service_ack)?;
            assert_eq!(decoded.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));
            println!("server-only read OK: {:?}", decoded.object_identifier);
        }
        other => panic!("expected ComplexAck, got {other:?}"),
    }

    // I-Am broadcast sends from the composed identity (local broadcast;
    // subnet delivery, not asserted here).
    endpoint.broadcast_i_am().await?;

    endpoint.stop().await?;
    peer_net.stop().await?;
    println!("done.");
    Ok(())
}
