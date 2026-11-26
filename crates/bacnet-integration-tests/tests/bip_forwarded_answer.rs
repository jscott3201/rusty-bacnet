//! A Forwarded-NPDU never answers a request (#2829).
//!
//! A B/IP node takes a Forwarded-NPDU's origin as its NPDU's source (Annex
//! J.2.5), and a node that is not a foreign device takes one from any
//! sender. A station on the link that cannot send from the device's UDP
//! address can still name it as a forward's origin. Here a client reads
//! from a device that never answers, and another socket on the link sends a
//! ComplexACK naming the device as origin, under every invoke ID, the
//! read's own first while it is open: the read must still time out, and the
//! B/IP transport counts each forward it dropped.
//!
//! Each forward goes once the client has counted the one before, so none is
//! lost to a full socket buffer and every invoke ID is provably seen.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_client::client::BACnetClient;
use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bvll::{decode_bip_mac, encode_bip_mac};
use bacnet_types::enums::{AbortReason, ObjectType, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::Bytes;
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

/// The invoke ID of the confirmed request in an Original-Unicast-NPDU.
fn request_invoke_id(frame: &[u8]) -> u8 {
    assert_eq!(frame[..2], [0x81, 0x0A], "an Original-Unicast-NPDU");
    let npdu = decode_npdu(Bytes::copy_from_slice(&frame[4..])).unwrap();
    match decode_apdu(npdu.payload).unwrap() {
        Apdu::ConfirmedRequest(request) => request.invoke_id,
        other => panic!("not a confirmed request: {other:?}"),
    }
}

/// Wait until `client` has dropped `count` forwards for their content.
async fn dropped(client: &BACnetClient<BipTransport>, count: u64) {
    let counted = async {
        while client
            .transport()
            .receive_counters()
            .forwarded_not_broadcast
            < count
        {
            tokio::task::yield_now().await;
        }
    };
    timeout(Duration::from_secs(5), counted)
        .await
        .unwrap_or_else(|_| panic!("forward {count} was not dropped"));
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
        // Room for the forwards on a loaded runner; the read ends by it.
        .apdu_timeout_ms(2_000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap();
    let (client_ip, client_port) = decode_bip_mac(client.local_mac()).unwrap();
    let client_address = SocketAddrV4::new(Ipv4Addr::from(client_ip), client_port);

    let device_mac = encode_bip_mac(device_address.ip().octets(), device_address.port());
    let open = AtomicBool::new(true);
    let read = async {
        let read = client
            .read_property(
                &device_mac[..],
                ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await;
        open.store(false, Ordering::SeqCst);
        read
    };
    let forge = async {
        // Once the request is out, so its transaction is open.
        let mut buf = [0u8; 1500];
        let (len, _) = timeout(Duration::from_secs(2), device.recv_from(&mut buf))
            .await
            .expect("the read reaches the device")
            .unwrap();
        let own = request_invoke_id(&buf[..len]);
        let others = (0..=u8::MAX).filter(|invoke_id| *invoke_id != own);
        for (count, invoke_id) in (1..).zip(std::iter::once(own).chain(others)) {
            stranger
                .send_to(&forwarded_ack(device_address, invoke_id), client_address)
                .await
                .unwrap();
            dropped(&client, count).await;
            if invoke_id == own {
                assert!(
                    open.load(Ordering::SeqCst),
                    "the read ended before the forward naming its invoke ID was dropped"
                );
            }
        }
    };
    let (read, ()) = tokio::join!(read, forge);
    // With no answer, it ends by its timeout after its one attempt.
    assert!(
        matches!(
            &read,
            Err(Error::Remote(RemoteError::Abort { reason })) if *reason == AbortReason::TSM_TIMEOUT
        ),
        "the read ended otherwise than by its timeout: {read:?}"
    );
    let counters = client.transport().receive_counters();
    assert_eq!(counters.forwarded_not_broadcast, 256);
    client.stop().await.unwrap();
}
