//! A B/IP transport bound to `0.0.0.0` accepts unicast only to one of the
//! host's listed IPv4 addresses, on every OS, with the destination the OS
//! reports for each datagram (#952).

use super::*;
use std::net::IpAddr;
use tokio::time::timeout;

const UNICAST_NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];
const BROADCAST_NPDU: &[u8] = &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

/// The host list, and the addresses in it that a local sender can surely
/// reach: loopback and the default-route address.
fn listed_and_reachable() -> (Vec<Ipv4Addr>, Vec<Ipv4Addr>) {
    let listed = crate::local_addresses::ipv4().unwrap();
    let mut reachable = vec![Ipv4Addr::LOCALHOST];
    reachable.extend(crate::local_addresses::route_ipv4());
    for ip in &reachable {
        assert!(
            listed.contains(ip),
            "{ip} is not in the host list {listed:?}"
        );
    }
    (listed, reachable)
}

fn frame(function: BvlcFunction, npdu: &[u8]) -> BytesMut {
    raw_frame(function, npdu)
}

async fn next_npdu(rx: &mut mpsc::Receiver<ReceivedNpdu>) -> ReceivedNpdu {
    timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for an NPDU")
        .expect("receive channel closed")
}

#[tokio::test]
async fn the_os_reports_a_listed_destination_for_unicast_to_a_wildcard_socket() {
    let (listed, reachable) = listed_and_reachable();
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap();
    socket.set_nonblocking(true).unwrap();
    let receiver = DestinationReceiver::configure(&socket, IpVersion::V4).unwrap();
    socket
        .bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0).into())
        .unwrap();
    let socket = UdpSocket::from_std(socket.into()).unwrap();
    let port = socket.local_addr().unwrap().port();
    let sender = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        .await
        .unwrap();
    let mut buf = [0u8; 64];
    let accepts = |function, received: &crate::udp_metadata::ReceivedDatagram| {
        admitted_delivery(
            function,
            received.destination,
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::BROADCAST,
            &listed,
            true,
            received.os_group_delivery,
        )
        .is_some()
    };

    for target in reachable {
        sender
            .send_to(b"unicast", SocketAddrV4::new(target, port))
            .await
            .unwrap();
        let received = timeout(
            Duration::from_secs(2),
            receiver.recv_from(&socket, &mut buf),
        )
        .await
        .expect("timed out waiting for the unicast datagram")
        .unwrap();
        assert_eq!(received.destination, IpAddr::from(target), "{received:?}");
        assert_ne!(received.os_group_delivery, Some(true), "{received:?}");
        assert!(
            accepts(BvlcFunction::ORIGINAL_UNICAST_NPDU, &received),
            "{received:?}"
        );
        assert!(
            !accepts(BvlcFunction::ORIGINAL_BROADCAST_NPDU, &received),
            "{received:?}"
        );
    }

    // Windows delivers a limited broadcast back to local sockets, flagged as
    // group delivery. (GitHub's macOS runners cannot send one at all.)
    #[cfg(windows)]
    {
        sender.set_broadcast(true).unwrap();
        sender
            .send_to(b"broadcast", SocketAddrV4::new(Ipv4Addr::BROADCAST, port))
            .await
            .unwrap();
        let received = timeout(
            Duration::from_secs(2),
            receiver.recv_from(&socket, &mut buf),
        )
        .await
        .expect("timed out waiting for the limited broadcast")
        .unwrap();
        use bacnet_bip_core::node::{Addressed, LocalAddresses};
        let IpAddr::V4(destination) = received.destination else {
            panic!("an IPv4 destination: {received:?}");
        };
        let mut local = LocalAddresses::new(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST);
        local.wildcard = Some(&listed);
        let to = Addressed::classify(destination, received.os_group_delivery, &local);
        assert!(to.broadcast && !to.node, "{received:?}");
        assert!(
            accepts(BvlcFunction::ORIGINAL_BROADCAST_NPDU, &received),
            "{received:?}"
        );
        assert!(
            !accepts(BvlcFunction::ORIGINAL_UNICAST_NPDU, &received),
            "{received:?}"
        );
    }
}

#[tokio::test]
async fn a_wildcard_transport_takes_unicast_to_listed_addresses_and_drops_mismatches() {
    let (_, reachable) = listed_and_reachable();
    let mut transport = BipTransport::new(Ipv4Addr::UNSPECIFIED, 0, Ipv4Addr::BROADCAST);
    let mut rx = transport.start().await.unwrap();
    let port = transport.port;
    let sender = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        .await
        .unwrap();
    let unicast = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, UNICAST_NPDU);
    let broadcast = frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, BROADCAST_NPDU);

    for target in reachable {
        let to = SocketAddrV4::new(target, port);
        // An Original-Broadcast-NPDU sent by unicast is dropped, so the first
        // NPDU delivered is the Original-Unicast-NPDU sent after it.
        sender.send_to(&broadcast, to).await.unwrap();
        sender.send_to(&unicast, to).await.unwrap();
        let npdu = next_npdu(&mut rx).await;
        assert_eq!(npdu.npdu.as_ref(), UNICAST_NPDU, "to {target}");
        assert!(!npdu.link_layer_group, "to {target}");
    }

    #[cfg(windows)]
    {
        // And the other way round for a limited broadcast.
        sender.set_broadcast(true).unwrap();
        let to = SocketAddrV4::new(Ipv4Addr::BROADCAST, port);
        sender.send_to(&unicast, to).await.unwrap();
        sender.send_to(&broadcast, to).await.unwrap();
        let npdu = next_npdu(&mut rx).await;
        assert_eq!(npdu.npdu.as_ref(), BROADCAST_NPDU);
        assert!(npdu.link_layer_group);
    }

    transport.stop().await.unwrap();
}
