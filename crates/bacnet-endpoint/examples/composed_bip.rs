//! Endpoint composed example (B/IP, both roles, real loopback UDP).
//!
//! One device owns client + server above one socket: the role initiates
//! confirmed requests out while independent peers request in, with I-Am
//! identical to Device ReadProperty.
//!
//! Run with: `cargo run -p bacnet-endpoint --example composed_bip`

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
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;
use bytes::BytesMut;

const WAIT: Duration = Duration::from_secs(5);

fn oid(t: ObjectType, i: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(t, i).unwrap()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let identity = DeviceIdentity::new(1001, 42)?.with_bip_port(1, 0, Ipv4Addr::LOCALHOST, 0)?;
    let mut point = AnalogInputObject::new(1, "composed-ai-1", 0)?;
    point.set_present_value(11.0);
    let db = build_database_with_extra(&identity, vec![Box::new(point)])?;

    let mut endpoint = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::Both)
        .database(db)
        .identity(identity.clone())
        .build_session()?;
    endpoint.start().await?;
    assert!(endpoint.is_running());
    assert_eq!(endpoint.session_role(), SessionRole::Both);

    // Controlled peer over a second real socket.
    let peer_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut peer_net = NetworkLayer::new(peer_transport);
    let mut peer_rx = peer_net.start().await?;
    let peer_mac = MacAddr::from_slice(peer_net.local_mac());

    // Outbound: endpoint role initiates; peer answers 22.0.
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
        .ok_or("no request")?;
    let endpoint_mac = req.source_mac.clone();
    assert_eq!(endpoint_mac.len(), 6, "one socket => one 6-byte B/IP MAC");
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
        .send_apdu(&enc, &endpoint_mac, false, NetworkPriority::NORMAL)
        .await?;
    let ack = tokio::time::timeout(WAIT, pending).await???;
    assert_eq!(ack.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));
    println!("outbound read OK.");

    // Inbound: peer requests present-value 11.0 from the endpoint server.
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
            invoke_id: 9,
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
        Apdu::ComplexAck(a) => assert_eq!(a.invoke_id, 9),
        other => panic!("expected ComplexAck, got {other:?}"),
    }
    println!("inbound read OK.");

    // Narrow admin while roles run + I-Am broadcast.
    endpoint.broadcast_i_am().await?;
    let counters = endpoint.policy_counters().await;
    assert_eq!(endpoint.active_leases(), 0);
    println!("counters: {counters:?}");

    // Owner shutdown; use-after-stop fails closed.
    endpoint.stop().await?;
    assert!(endpoint.stop().await.is_err(), "stop-once");
    assert!(endpoint.broadcast_i_am().await.is_err());
    peer_net.stop().await?;
    println!("done.");
    Ok(())
}
