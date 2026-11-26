//! A Forwarded-NPDU never answers a request (#2829).
//!
//! A B/IP node takes a Forwarded-NPDU's origin as its NPDU's source (Annex
//! J.2.5), and a node that is not a foreign device takes one from any
//! sender. A station on the link that cannot send from the device's UDP
//! address can still name it as a forward's origin. Here a client reads
//! from a device that never answers, and another socket on the link sends a
//! ComplexACK naming the device as origin, under every invoke ID: the read
//! must still time out, and the B/IP transport counts each forward it
//! dropped.

use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_client::client::BACnetClient;
use bacnet_transport::bvll::{decode_bip_mac, encode_bip_mac};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use tokio::net::UdpSocket;
use tokio::time::{Duration, timeout};

/// A ReadProperty-ACK for Analog Input 1's Present_Value, 666.0.
const READ_ACK: [u8; 14] = [
    0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x3E, 0x44, 0x44, 0x26, 0x80, 0x00, 0x3F,
];

/// A Forwarded-NPDU naming `origin`, carrying a local NPDU with a
/// ComplexACK for ReadProperty under `invoke_id`.
fn forwarded_ack(origin: SocketAddrV4, invoke_id: u8) -> Vec<u8> {
    let mut npdu = vec![0x01, 0x00, 0x30, invoke_id, 0x0C];
    npdu.extend_from_slice(&READ_ACK);
    let length = 10 + npdu.len();
    let mut frame = vec![0x81, 0x04, (length >> 8) as u8, length as u8];
    frame.extend_from_slice(&origin.ip().octets());
    frame.extend_from_slice(&origin.port().to_be_bytes());
    frame.extend_from_slice(&npdu);
    frame
}

fn v4(socket: &UdpSocket) -> SocketAddrV4 {
    match socket.local_addr().unwrap() {
        std::net::SocketAddr::V4(address) => address,
        other => panic!("{other} is not IPv4"),
    }
}

#[tokio::test]
async fn a_forwarded_answer_naming_the_device_never_completes_its_read() {
    let device = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let stranger = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let device_address = v4(&device);
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .apdu_timeout_ms(500)
        .apdu_retries(0)
        .build()
        .await
        .unwrap();
    let (client_ip, client_port) = decode_bip_mac(client.local_mac()).unwrap();
    let client_address = SocketAddrV4::new(Ipv4Addr::from(client_ip), client_port);

    let device_mac = encode_bip_mac(device_address.ip().octets(), device_address.port());
    let read = client.read_property(
        &device_mac[..],
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let forge = async {
        // Once the request is out, so its transaction is open.
        let mut buf = [0u8; 1500];
        timeout(Duration::from_secs(2), device.recv_from(&mut buf))
            .await
            .expect("the read reaches the device")
            .unwrap();
        for invoke_id in 0..=u8::MAX {
            stranger
                .send_to(&forwarded_ack(device_address, invoke_id), client_address)
                .await
                .unwrap();
        }
    };
    let (read, ()) = tokio::join!(read, forge);
    // With no answer, it ends unanswered after its one attempt.
    assert!(
        read.is_err(),
        "a forwarded ComplexACK answered the read: {read:?}"
    );
    let counters = client.transport().receive_counters();
    assert_eq!(counters.forwarded_not_broadcast, 256);
    client.stop().await.unwrap();
}
