//! Normal-mode resolution through the BACnet multicast group, on real
//! sockets (U.5, #1637): a send to an unknown VMAC multicasts an
//! Address-Resolution, and the answer releases the send.
//!
//! The test multicasts to FF02::BAC0 on the loopback interface. macOS loops
//! that back to a local socket; a Linux loopback without the multicast flag
//! does not, and there the ignored isolated-link fixtures carry the
//! evidence. On macOS the test fails if the group isn't reachable;
//! elsewhere it skips.

use std::io;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;

use super::*;
use crate::port::TransportPort;

/// The OS whose loopback carries FF02::BAC0, where the test has to run.
const EVIDENCE_OS: bool = cfg!(target_os = "macos");
const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];

/// The loopback interface's index, looked up by its usual names.
fn loopback_index() -> Option<u32> {
    ["lo0", "lo"]
        .into_iter()
        .find_map(|name| nix::net::if_::if_nametoindex(name).ok())
}

/// A socket bound to FF02::BAC0 at `port` that joined it on interface
/// `index`. Bound to the group, it gets the group's datagrams at that port
/// but not the unicasts the transport's wildcard socket there is for.
fn group_listener(port: u16, index: u32) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_only_v6(true)?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    let group = SocketAddrV6::new(BACNET_IPV6_MULTICAST_LINK_LOCAL, port, 0, index);
    socket.bind(&group.into())?;
    socket.join_multicast_v6(&BACNET_IPV6_MULTICAST_LINK_LOCAL, index)?;
    UdpSocket::from_std(socket.into())
}

#[allow(clippy::print_stderr)] // a skipped setup is noted on the console
fn skip(why: &str) {
    if EVIDENCE_OS {
        panic!("macOS loops FF02::BAC0 back on lo0, and this test needs it: {why}");
    }
    eprintln!("skipping: {why}");
}

/// A node at device instance 10 sends to VMAC 00 04 D2, which it doesn't
/// know. The group gets `82 03 00 0A 00 00 0A 00 04 D2`; the peer answers
/// from its own address with `82 05 00 0A 00 04 D2 00 00 0A`; the held NPDU
/// then reaches that address as an Original-Unicast-NPDU.
#[tokio::test]
async fn a_normal_mode_send_resolves_through_the_multicast_group() {
    let Some(index) = loopback_index() else {
        return skip("no loopback interface index");
    };
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(10));
    transport.set_broadcast_scope(Bip6BroadcastScope::LinkLocal);
    let _rx = transport.start().await.unwrap();
    let local = transport.local_address().unwrap();
    let listener = match group_listener(local.port(), index) {
        Ok(listener) => listener,
        Err(error) => return skip(&format!("cannot listen on the group: {error}")),
    };
    let peer = UdpSocket::bind("[::1]:0").await.unwrap();
    let SocketAddr::V6(peer_address) = peer.local_addr().unwrap() else {
        unreachable!("bound to ::1");
    };
    let answer = async {
        let mut buf = [0u8; 64];
        loop {
            let (len, _) = listener.recv_from(&mut buf).await.unwrap();
            // The start's own collision probe may come first.
            if buf[..len] == [0x82, 0x03, 0x00, 0x0A, 0x00, 0x00, 0x0A, 0x00, 0x04, 0xD2] {
                break;
            }
        }
        let ack = [0x82, 0x05, 0x00, 0x0A, 0x00, 0x04, 0xD2, 0x00, 0x00, 0x0A];
        peer.send_to(&ack, local).await.unwrap();
        let (len, from) = peer.recv_from(&mut buf).await.unwrap();
        assert_eq!(from, SocketAddr::V6(local));
        assert_eq!(
            buf[..len],
            [0x82, 0x01, 0x00, 0x0E, 0x00, 0x00, 0x0A, 0x00, 0x04, 0xD2, 0x01, 0x00, 0x10, 0x08]
        );
    };
    let exchange =
        async { tokio::join!(transport.send_unicast(NPDU, &[0x00, 0x04, 0xD2]), answer) };
    match tokio::time::timeout(Duration::from_secs(5), exchange).await {
        Ok((sent, ())) => sent.unwrap(),
        Err(_) => return skip("the Address-Resolution never reached the group"),
    }
    assert_eq!(
        transport.lookup_vmac(Bip6Vmac::new([0x00, 0x04, 0xD2])),
        Some(peer_address)
    );
    let counters = transport.resolution_counters();
    assert_eq!(
        (counters.resolutions_started, counters.resolutions_resolved),
        (1, 1)
    );
    transport.stop().await.unwrap();
}
