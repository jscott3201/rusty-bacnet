//! Standalone client Number wire evidence in isolated Linux BBMD/foreign modes.
#![cfg(target_os = "linux")]
#![allow(clippy::print_stderr)] // the evidence helpers log each observed wire frame

// Reuse only the independent UDP/byte helpers; no full server is constructed.
#[allow(dead_code)]
#[path = "bip_network_numbers/support.rs"]
mod support;
use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_transport::{
    bbmd::{BdtEntry, ForeignDevicePolicy},
    bip::{BipTransport, ForeignDeviceConfig},
    port::TransportPort,
};
use bacnet_types::{
    enums::{ObjectType, PropertyIdentifier},
    primitives::ObjectIdentifier,
};
use std::net::{Ipv4Addr, SocketAddrV4};
use support::*;
use tokio::net::UdpSocket;

type Client = BACnetClient<BipTransport>;
async fn client(transport: BipTransport) -> (Client, SocketAddrV4) {
    assert!(transport.supports_local_nonrouter_number_controls());
    // A BBMD or foreign device; no Network Port is registered here.
    assert_ne!(
        transport.bip_port().unwrap().mode.ip_mode(),
        bacnet_types::enums::IPMode::NORMAL
    );
    let client = bounded(BACnetClient::start(ClientConfig::default(), transport))
        .await
        .unwrap();
    let mac = client.local_mac();
    assert_eq!(&mac[..4], &[127, 0, 0, 1]);
    let local = SocketAddrV4::new(Ipv4Addr::LOCALHOST, u16::from_be_bytes([mac[4], mac[5]]));
    (client, local)
}
async fn forward(peer: &UdpSocket, local: SocketAddrV4, npdu: &[u8]) {
    let origin = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 9), address(peer).port());
    send(peer, local, &forwarded(origin, npdu)).await;
}
async fn own_number(peer: &UdpSocket, local: SocketAddrV4, function: u8, value: u16) {
    // The shared oracle inspects every own non-forwarding output. Function 4
    // is BBMD fanout, not an own Number reply; duplicate own replies are retained.
    expect_number(peer, local, function, value).await;
    eprintln!(
        "client={local} own BVLC={function:#04x} wire={:02x?}",
        frame(function, &number(value, 0))
    );
}
async fn stop(mut client: Client, local: SocketAddrV4) {
    bounded(client.stop()).await.unwrap();
    let _rebound = std::net::UdpSocket::bind(local)
        .expect("client stop releases actual UDP socket before client Drop");
}

#[tokio::test]
async fn client_number_bbmd_wire_admission_and_release() {
    let bdt = udp().await;
    let fd = udp().await;
    let peer = udp().await;
    let denied = udp().await;
    let (captured, port) = observer();
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, port, BROADCAST);
    transport.enable_bbmd(vec![BdtEntry {
        ip: Ipv4Addr::LOCALHOST.octets(),
        port: address(&bdt).port(),
        broadcast_mask: [255; 4],
    }]);
    transport.enable_foreign_device_registration(ForeignDevicePolicy::default());
    let (client, local) = client(transport).await;
    let group = SocketAddrV4::new(BROADCAST, local.port());
    // Prove actual wildcard production intake and broadcast observer coexistence.
    send(&peer, group, &frame(11, QUERY)).await;
    assert_eq!(receive(&captured).await, (frame(11, QUERY), address(&peer)));
    assert_eq!(
        receive(&bdt).await,
        (forwarded(address(&peer), QUERY), local)
    );
    // UNKNOWN and a unicast NNI cannot create an own response before the fence.
    send(&peer, local, &frame(10, &number(999, 1))).await;
    send(&peer, local, &frame(10, QUERY)).await;
    send(&peer, group, &frame(11, &number(77, 0))).await;
    send(&peer, local, &frame(10, QUERY)).await;
    own_number(&captured, local, 11, 77).await;

    forward(&denied, local, &number(200, 1)).await;
    fence(&denied, local).await; // receive loop only, not Number completion
    send(&peer, local, &frame(10, QUERY)).await;
    own_number(&captured, local, 11, 77).await;
    send(&fd, local, &frame(9, &number(200, 1))).await;
    assert_eq!(receive(&fd).await, (frame(0, &[0, 0x60]), local));
    send(&peer, local, &frame(10, QUERY)).await;
    own_number(&captured, local, 11, 77).await;

    // Same IP with a different port is an admitted BDT peer, not a self echo.
    forward(&bdt, local, &number(78, 1)).await;
    forward(&bdt, local, QUERY).await;
    own_number(&captured, local, 11, 78).await;
    send(&fd, local, &frame(5, &60u16.to_be_bytes())).await;
    assert_eq!(receive(&fd).await, (frame(0, &[0, 0]), local));
    send(&fd, local, &frame(9, &number(79, 1))).await;
    send(&fd, local, &frame(9, QUERY)).await;
    own_number(&captured, local, 11, 79).await;

    for invalid in [
        number(80, 0), // cannot replace flag-one learned quality
        number(200, 2),
        vec![1, 0x80, 0x13, 0, 200],
        vec![1, 0x88, 0, 4, 1, 9, 0x13, 0, 200, 1],
    ] {
        send(&peer, group, &frame(11, &invalid)).await;
        send(&peer, local, &frame(10, QUERY)).await;
        own_number(&captured, local, 11, 79).await;
    }
    // An incorrectly accepted malformed query would emit old 79 before new 81.
    send(&peer, group, &frame(11, &[1, 0x80, 0x12, 0])).await;
    send(&peer, group, &frame(11, &number(81, 1))).await;
    send(&peer, local, &frame(10, QUERY)).await;
    own_number(&captured, local, 11, 81).await;
    drop(captured);
    stop(client, local).await;
}

#[tokio::test]
async fn client_number_foreign_wire_alternate_sender_retry_and_progress() {
    let bbmd = udp().await;
    let alternate = udp().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, BROADCAST);
    transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: address(&bbmd).port(),
        renewal_interval: None,
        ttl: 60,
    });
    let (client, local) = client(transport).await;
    assert_eq!(receive(&bbmd).await, (frame(5, &[0, 60]), local));
    send(&bbmd, local, &frame(0, &[0, 0])).await;
    send(&alternate, local, &frame(10, QUERY)).await;
    send(&alternate, local, &frame(10, &number(999, 1))).await;
    send(&alternate, local, &frame(10, QUERY)).await;
    fence(&alternate, local).await;
    forward(&bbmd, local, &number(77, 0)).await;
    send(&bbmd, local, &frame(10, QUERY)).await;
    own_number(&bbmd, local, 9, 77).await;
    // Existing compatibility accepts another UDP sender; output still uses BBMD.
    forward(&alternate, local, &number(78, 1)).await;
    forward(&alternate, local, QUERY).await;
    own_number(&bbmd, local, 9, 78).await;
    for invalid in [
        number(79, 0),
        number(200, 2),
        vec![1, 0x80, 0x13, 0, 200],
        vec![1, 0x88, 0, 4, 1, 9, 0x13, 0, 200, 1],
    ] {
        forward(&alternate, local, &invalid).await;
        forward(&alternate, local, QUERY).await;
        own_number(&bbmd, local, 9, 78).await;
    }
    forward(&alternate, local, &[1, 0x80, 0x12, 0]).await;
    forward(&alternate, local, &number(79, 1)).await;
    forward(&alternate, local, QUERY).await;
    own_number(&bbmd, local, 9, 79).await;
    send(&bbmd, local, &frame(0, &[0, 0x30])).await;
    send(&bbmd, local, &frame(0, &[0, 0x60])).await;
    forward(&bbmd, local, QUERY).await;
    own_number(&bbmd, local, 9, 79).await;
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(31)).await;
    tokio::time::resume();
    assert_eq!(receive(&bbmd).await, (frame(5, &[0, 60]), local));
    send(&bbmd, local, &frame(0, &[0, 0])).await;
    forward(&bbmd, local, &number(80, 1)).await;
    forward(&bbmd, local, QUERY).await;
    own_number(&bbmd, local, 9, 80).await;

    // Exercise the client's requester while Number replies remain live. There is
    // no inbound full-server responder assumption or held physical writer here.
    let responder = udp().await;
    let mut mac = address(&responder).ip().octets().to_vec();
    mac.extend_from_slice(&address(&responder).port().to_be_bytes());
    {
        let request = client.read_property(
            &mac,
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        );
        tokio::pin!(request);
        let (wire, source) = bounded(async {
            tokio::select! {
                result = &mut request => panic!("request completed before ACK: {result:?}"),
                packet = receive(&responder) => packet,
            }
        })
        .await;
        assert_eq!(source, local);
        assert_eq!(wire[1], 10);
        let npdu = &wire[4..];
        assert_eq!(&npdu[..3], &[1, 4, 2]); // segmented-response capability
        assert_eq!(npdu[5], 12);
        assert_eq!(&npdu[6..], &[0x0c, 0, 0, 0, 1, 0x19, 85]);
        forward(&bbmd, local, QUERY).await;
        own_number(&bbmd, local, 9, 80).await;
        let ack = [
            1, 0, 0x30, npdu[4], 12, 0x0c, 0, 0, 0, 1, 0x19, 85, 0x3e, 0x44, 0x42, 0x28, 0, 0, 0x3f,
        ];
        send(&responder, local, &frame(10, &ack)).await;
        assert_eq!(
            bounded(&mut request).await.unwrap().property_value,
            [0x44, 0x42, 0x28, 0, 0]
        );
    }
    forward(&bbmd, local, &number(81, 1)).await;
    forward(&bbmd, local, QUERY).await;
    own_number(&bbmd, local, 9, 81).await;
    stop(client, local).await;
}
