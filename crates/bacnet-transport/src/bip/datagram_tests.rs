//! A datagram in parts is the encoder's frame, byte for byte, and the socket
//! sends it as one datagram on both of its send paths (#1978 item 5).

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::atomic::Ordering;

use bacnet_bip_codec::Bvll;
use bytes::BytesMut;
use tokio::net::UdpSocket;

use super::super::socket::BipSocket;
use super::*;
use crate::bvll::encode_bvll;

fn encoded(message: &Bvll<'_>) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_bvll(&mut buf, message).unwrap();
    buf.to_vec()
}

fn npdus() -> Vec<Vec<u8>> {
    vec![
        vec![1, 0],
        vec![
            1, 4, 0x00, 0x05, 0x01, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55,
        ],
        (0..1497).map(|index| index as u8).collect(),
    ]
}

#[test]
fn the_parts_are_the_encoded_frame() {
    let origin = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 9), 47808);
    for npdu in npdus() {
        for message in [
            Bvll::OriginalUnicastNpdu(&npdu),
            Bvll::OriginalBroadcastNpdu(&npdu),
            Bvll::DistributeBroadcastToNetwork(&npdu),
            Bvll::ForwardedNpdu {
                origin,
                npdu: &npdu,
            },
        ] {
            let datagram = OutgoingDatagram::split(&message, &npdu).expect("a header and the NPDU");
            assert_eq!(datagram.joined(), encoded(&message));
            assert_eq!(datagram.len(), encoded(&message).len());
            assert!(std::ptr::eq(datagram.payload(), npdu.as_slice()));
        }
    }
}

#[test]
fn a_message_not_ending_in_the_npdu_is_not_split() {
    let npdu = [1, 0];
    let copy = npdu;
    // The message carries a copy, not the slice the caller passed.
    assert!(OutgoingDatagram::split(&Bvll::OriginalUnicastNpdu(&copy), &npdu).is_none());
    // No NPDU at all.
    let table = Bvll::ReadBroadcastDistributionTable;
    assert!(OutgoingDatagram::split(&table, &[]).is_none());
}

async fn udp() -> UdpSocket {
    UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap()
}

fn address(socket: &UdpSocket) -> SocketAddrV4 {
    match socket.local_addr().unwrap() {
        std::net::SocketAddr::V4(address) => address,
        other => panic!("an IPv4 test socket, not {other}"),
    }
}

/// Sent at once, or after waiting for the socket, the parts arrive as one
/// datagram holding the encoded frame.
#[tokio::test]
async fn a_datagram_in_parts_arrives_whole_on_both_send_paths() {
    let sender = BipSocket::new(udp().await, None);
    let receiver = udp().await;
    let mut buf = vec![0u8; 2048];
    for npdu in npdus() {
        let message = Bvll::OriginalUnicastNpdu(&npdu);
        for would_block in [false, true] {
            sender.block_next_try.store(would_block, Ordering::Relaxed);
            let datagram = OutgoingDatagram::split(&message, &npdu).unwrap();
            let sent = sender
                .send_datagram(datagram, address(&receiver))
                .await
                .unwrap();
            assert_eq!(sent, npdu.len() + 4);
            assert!(!sender.block_next_try.load(Ordering::Relaxed));
            let (len, _) = receiver.recv_from(&mut buf).await.unwrap();
            assert_eq!(&buf[..len], encoded(&message).as_slice());
        }
    }
}
