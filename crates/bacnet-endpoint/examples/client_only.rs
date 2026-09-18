//! Endpoint client-only example (B/IP, real loopback UDP).
//!
//! Old path: `BACnetClient::bip_builder()...build().await`.
//! New path: `BipEndpointBuilder::role(ClientOnly)` + `start()`.
//!
//! Run with: `cargo run -p bacnet-endpoint --example client_only`

use std::time::Duration;

use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu};
use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::session::SessionRole;
use bacnet_network::layer::NetworkLayer;
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{
    ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier,
};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;
use bytes::BytesMut;
use std::net::Ipv4Addr;

const WAIT: Duration = Duration::from_secs(5);

fn oid(t: ObjectType, i: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(t, i).unwrap()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Client-only endpoint: no database, no server handle.
    let mut endpoint = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::ClientOnly)
        .queue_capacity(16)
        .client_timers(2_000, 0)
        .build_session()?;
    endpoint.start().await?;
    assert!(endpoint.client().is_some());
    assert!(endpoint.server().is_none());

    // Controlled peer: real B/IP socket + network layer.
    let peer_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut peer_net = NetworkLayer::new(peer_transport);
    let mut peer_rx = peer_net.start().await?;
    let peer_mac = MacAddr::from_slice(peer_net.local_mac());

    // Endpoint initiates; peer answers with present-value 22.0.
    let client = endpoint.cloned_client_handle().unwrap();
    let peer_mac_c = peer_mac.clone();
    let pending = tokio::spawn(async move {
        client
            .read_property(
                &peer_mac_c,
                oid(ObjectType::ANALOG_INPUT, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    let req = tokio::time::timeout(WAIT, peer_rx.recv())
        .await?
        .ok_or("peer closed")?;
    let iid = match decode_apdu(req.apdu.clone())? {
        Apdu::ConfirmedRequest(r) => r.invoke_id,
        other => panic!("peer expected ConfirmedRequest, got {other:?}"),
    };
    let mut val = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut val, &PropertyValue::Real(22.0))?;
    let mut svc = BytesMut::new();
    ReadPropertyACK {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: val.to_vec(),
    }
    .encode(&mut svc);
    let mut enc = BytesMut::new();
    encode_apdu(
        &mut enc,
        &Apdu::ComplexAck(bacnet_encoding::apdu::ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: iid,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: svc.freeze(),
        }),
    )?;
    peer_net
        .send_apdu(&enc, &req.source_mac, false, NetworkPriority::NORMAL)
        .await?;
    let ack = tokio::time::timeout(WAIT, pending).await???;
    assert_eq!(ack.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));
    println!("client-only read OK: {:?}", ack.object_identifier);

    // Owner shutdown with the cloned handle alive, then use-after-stop fails closed.
    endpoint.stop().await?;
    peer_net.stop().await?;
    println!("done.");
    Ok(())
}
