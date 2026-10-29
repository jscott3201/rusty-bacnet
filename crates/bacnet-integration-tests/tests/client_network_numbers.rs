//! Actual NORMAL-B/IP client wire proof on isolated Linux loopback.
#![cfg(target_os = "linux")]
use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_transport::bip::BipTransport;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use tokio::{
    net::UdpSocket,
    time::{Duration, timeout},
};

// The closing rebinds can lose the freed port to another process (#1419).
#[path = "../../bacnet-endpoint/tests/support/port_retry.rs"]
mod port_retry;

const BROADCAST: Ipv4Addr = Ipv4Addr::new(127, 255, 255, 255);
fn frame(function: u8, npdu: &[u8]) -> Vec<u8> {
    let mut wire = vec![0x81, function];
    wire.extend_from_slice(&((4 + npdu.len()) as u16).to_be_bytes());
    wire.extend_from_slice(npdu);
    wire
}
async fn expect(observer: &UdpSocket, source: SocketAddrV4, number: u8) {
    timeout(Duration::from_secs(3), async {
        let mut wire = [0; 256];
        loop {
            let (n, from) = observer.recv_from(&mut wire).await.unwrap();
            if from != SocketAddr::V4(source) {
                continue;
            }
            // Independent BVLC and NPDU bytes, not the production decoder.
            assert_eq!(&wire[..n], frame(11, &[1, 0x80, 0x13, 0, number, 0]));
            break;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn client_number_normal_bip_actual_broadcast_wire_and_release() {
    port_retry::rerun_on_lost_port(async || {
        // This exact SO_REUSEADDR wildcard/broadcast-specific pattern was separately
        // qualified with actual Linux UDP in the preceding B/IP full-server slice.
        // The observer binds first: only an explicitly requested client port sets
        // SO_REUSEADDR (#892), and both sockets need it to share the port.
        let raw = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )
        .unwrap();
        raw.set_reuse_address(true).unwrap();
        raw.set_nonblocking(true).unwrap();
        raw.bind(&SocketAddrV4::new(BROADCAST, 0).into()).unwrap();
        let observer = UdpSocket::from_std(raw.into()).unwrap();
        let port = observer.local_addr().unwrap().port();
        let transport = BipTransport::new(Ipv4Addr::LOCALHOST, port, BROADCAST);
        let mut client = BACnetClient::start(ClientConfig::default(), transport)
            .await
            .unwrap();
        let mac = client.local_mac();
        assert_eq!(&mac[..4], &[127, 0, 0, 1]);
        assert_eq!(u16::from_be_bytes([mac[4], mac[5]]), port);
        let local = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        let group = SocketAddrV4::new(BROADCAST, port);
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        peer.set_broadcast(true).unwrap();
        peer.send_to(&frame(10, &[1, 0x80, 0x12]), local)
            .await
            .unwrap();
        peer.send_to(&frame(11, &[1, 0x80, 0x12]), group)
            .await
            .unwrap();
        peer.send_to(&frame(11, &[1, 0x80, 0x13, 0, 77, 1]), group)
            .await
            .unwrap();
        peer.send_to(&frame(10, &[1, 0x80, 0x12]), local)
            .await
            .unwrap();
        expect(&observer, local, 77).await;
        peer.send_to(&frame(11, &[1, 0x80, 0x12]), group)
            .await
            .unwrap();
        expect(&observer, local, 77).await;
        // Unicast NNI cannot replace the learned number; the next reply is its fence.
        peer.send_to(&frame(10, &[1, 0x80, 0x13, 0, 99, 1]), local)
            .await
            .unwrap();
        peer.send_to(&frame(10, &[1, 0x80, 0x12]), local)
            .await
            .unwrap();
        expect(&observer, local, 77).await;
        client.stop().await.unwrap();
        drop(observer);
        // Stop releases the B/IP socket before the client's Drop.
        std::net::UdpSocket::bind(local).map(drop)
    })
    .await;
}
