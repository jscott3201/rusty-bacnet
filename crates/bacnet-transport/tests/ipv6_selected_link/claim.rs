//! A requested port on the isolated link (#2385): claimed by default, with
//! the node's one socket still taking its groups, its own echo and its
//! unicast; and what sharing it costs on Linux.

use super::*;
use crate::port_retry::{bind_error, rerun_on_lost_port};
use std::io;

/// A UDP port no socket holds now; its users rerun on a lost port.
fn free_port() -> u16 {
    std::net::UdpSocket::bind((Ipv6Addr::UNSPECIFIED, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Another program's socket on `[address]:port` that asks to share it.
fn squatter(address: Ipv6Addr, port: u16) -> io::Result<socket2::Socket> {
    let socket = socket2::Socket::new(socket2::Domain::IPV6, socket2::Type::DGRAM, None)?;
    socket.set_only_v6(true)?;
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddrV6::new(address, port, 0, 0).into())?;
    Ok(socket)
}

/// `function` (1 unicast, 2 broadcast) from VMAC 40 88 `tag`, carrying
/// `npdu`, with `to` as a unicast's destination VMAC.
fn frame(function: u8, tag: u8, to: &[u8], npdu: &[u8]) -> Vec<u8> {
    let length = 7 + to.len() + npdu.len();
    let mut frame = vec![0x82, function, 0, length as u8, 0x40, 0x88, tag];
    frame.extend_from_slice(to);
    frame.extend_from_slice(npdu);
    frame
}

/// A transport that only names its port claims it: a second socket that
/// asks to share is refused, on `[::]` everywhere and on Linux on the
/// node's own address and on its group too. The node's one socket still
/// takes a group's datagram, its own broadcast coming back, and its
/// unicast.
#[tokio::test]
#[ignore = "requires an isolated IPv6 link or explicitly selected loopback-only fixture"]
async fn explicit_claimed_port_refuses_a_second_bind_and_keeps_group_echo_and_unicast() {
    rerun_on_lost_port(async || {
        let (selected, index) = fixture();
        let port = free_port();
        let mut transport = Bip6Transport::new(selected, port, None);
        let mut incoming = transport.start().await.map_err(bind_error)?;
        let local = transport.local_address().unwrap();
        assert_eq!((*local.ip(), local.port()), (selected, port));
        for address in [Ipv6Addr::UNSPECIFIED, selected, GROUP] {
            let refused =
                address.is_unspecified() || cfg!(any(target_os = "linux", target_os = "android"));
            let second = squatter(address, port);
            assert_eq!(second.is_err(), refused, "{address}: {:?}", second.err());
        }

        let peer = udp(selected, 0, index, false);
        peer.send_to(
            &frame(2, 0x23, &[], NPDU),
            SocketAddrV6::new(GROUP, port, 0, index),
        )
        .await
        .unwrap();
        let heard = tokio::time::timeout(DEADLINE, incoming.recv())
            .await
            .expect("a group datagram reaches the claimed socket")
            .unwrap();
        assert_eq!(heard.npdu.as_ref(), NPDU);
        assert!(heard.link_layer_group);
        assert_eq!(heard.source_mac.as_slice(), [0x40, 0x88, 0x23]);

        assert_eq!(transport.receive_counters().own_echo, 0);
        transport.send_broadcast(NPDU).await.unwrap();
        let echoed = async {
            while transport.receive_counters().own_echo == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        };
        tokio::time::timeout(DEADLINE, echoed)
            .await
            .expect("the node's own broadcast comes back to the claimed socket");

        let vmac = transport.local_mac().to_vec();
        peer.send_to(
            &frame(1, 0x23, &vmac, NPDU),
            SocketAddrV6::new(selected, port, 0, 0),
        )
        .await
        .unwrap();
        let directed = tokio::time::timeout(DEADLINE, incoming.recv())
            .await
            .expect("a unicast reaches the claimed socket")
            .unwrap();
        assert_eq!(directed.npdu.as_ref(), NPDU);
        assert!(!directed.link_layer_group);
        transport.stop().await.unwrap();
        Ok(())
    })
    .await;
}

/// What sharing costs on Linux: a socket that binds the shared port after
/// the transport gets the unicast sent to the node, and the transport gets
/// none of it. A group's datagram still reaches both.
#[cfg(any(target_os = "linux", target_os = "android"))]
#[tokio::test]
#[ignore = "requires an explicitly supplied isolated IPv6 multicast link"]
async fn explicit_shared_port_gives_its_unicast_to_the_socket_bound_last() {
    rerun_on_lost_port(async || {
        let (selected, index) = fixture();
        let port = free_port();
        let mut transport = sharing(selected, port, None);
        let mut incoming = transport.start().await.map_err(bind_error)?;
        // Another stack's socket on the port, joined to the group.
        let later = udp(Ipv6Addr::UNSPECIFIED, port, index, true);
        let peer = udp(selected, 0, index, false);

        let vmac = transport.local_mac().to_vec();
        let unicast = frame(1, 0x24, &vmac, NPDU);
        peer.send_to(&unicast, SocketAddrV6::new(selected, port, 0, 0))
            .await
            .unwrap();
        let taken = tokio::time::timeout(DEADLINE, wire::receive(&later))
            .await
            .expect("the later socket takes the node's unicast")
            .unwrap();
        assert_eq!(taken.bytes, unicast);

        // Had the unicast reached the transport too, it would come up
        // before this group datagram, sent after it.
        let marker = [1, 0, 0x10, 8, 0x09, 9, 0x19, 9];
        peer.send_to(
            &frame(2, 0x24, &[], &marker),
            SocketAddrV6::new(GROUP, port, 0, index),
        )
        .await
        .unwrap();
        wire_broadcast(&later, &marker).await.unwrap();
        let first = tokio::time::timeout(DEADLINE, incoming.recv())
            .await
            .expect("a group datagram reaches every socket on the port")
            .unwrap();
        assert_eq!(first.npdu.as_ref(), marker);
        assert!(first.link_layer_group);
        transport.stop().await.unwrap();
        Ok(())
    })
    .await;
}
