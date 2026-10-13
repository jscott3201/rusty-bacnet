//! Checking a quiet peer on real sockets on `::1` (U.5, #1637): the check is
//! a Virtual-Address-Resolution sent by unicast to the address on file, so it
//! reaches a peer however it was learned, here one with no multicast domain
//! or BBMD in common. An answer keeps the peer; silence removes it, holds its
//! VMAC unreachable for a while, and the next send resolves it afresh.
//!
//! The test runs in real time, since macOS hands a loopback datagram over
//! after `send_to` returns, and a paused clock would jump ahead meanwhile.
//! It moves the table's times back instead of waiting minutes out.

use std::net::{Ipv6Addr, SocketAddr};
use std::time::Duration;

use bacnet_types::error::Error;
use tokio::net::UdpSocket;
use tokio::time::Instant;

use super::vmac_table::{REVALIDATE_AFTER, UNREACHABLE_FOR};
use super::*;
use crate::port::TransportPort;

/// The peer's VMAC: device instance 1234.
const PEER: [u8; 3] = [0x00, 0x04, 0xD2];
const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];
/// Original-Unicast-NPDU from VMAC 00 00 0A (this node) to [`PEER`].
const TO_PEER: [u8; 14] = [
    0x82, 0x01, 0x00, 0x0E, 0x00, 0x00, 0x0A, 0x00, 0x04, 0xD2, 0x01, 0x00, 0x10, 0x08,
];
/// Virtual-Address-Resolution from VMAC 00 00 0A.
const CHECK: [u8; 7] = [0x82, 0x06, 0x00, 0x07, 0x00, 0x00, 0x0A];

/// The next datagram `socket` reads.
async fn next(socket: &UdpSocket) -> Vec<u8> {
    let mut buf = [0u8; 1500];
    let (len, _) = tokio::time::timeout(Duration::from_secs(10), socket.recv_from(&mut buf))
        .await
        .expect("a datagram arrives")
        .unwrap();
    buf[..len].to_vec()
}

/// The next datagram the BBMD reads that isn't a Register-Foreign-Device.
async fn next_request(bbmd: &UdpSocket) -> Vec<u8> {
    loop {
        let datagram = next(bbmd).await;
        if datagram[1] != 0x09 {
            return datagram;
        }
    }
}

/// Wait until no lookup is running.
async fn settled(transport: &Bip6Transport) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while transport.resolver.with_state(|state, _| state.resolving()) > 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the lookup ends");
}

/// Let `by` go by in the table.
fn backdate(transport: &Bip6Transport, by: Duration) {
    transport
        .resolver
        .with_state(|state, _| state.table.backdate(by));
}

fn kind(result: &Result<(), Error>) -> Option<std::io::ErrorKind> {
    match result {
        Err(Error::Transport(error)) => Some(error.kind()),
        _ => None,
    }
}

#[tokio::test]
async fn a_quiet_peer_is_checked_by_unicast_and_removed_when_silent() {
    let bbmd = UdpSocket::bind("[::1]:0").await.unwrap();
    let peer = UdpSocket::bind("[::1]:0").await.unwrap();
    let (SocketAddr::V6(bbmd_address), SocketAddr::V6(peer_address)) =
        (bbmd.local_addr().unwrap(), peer.local_addr().unwrap())
    else {
        unreachable!("bound to ::1");
    };
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(10));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: *bbmd_address.ip(),
        bbmd_port: bbmd_address.port(),
        ttl: 60,
    });
    let mut rx = transport.start().await.unwrap();
    let local = transport.local_address().unwrap();
    let peer_vmac = Bip6Vmac::new(PEER);

    // The peer makes itself known with a frame of its own.
    let hello = [
        0x82, 0x01, 0x00, 0x0E, 0x00, 0x04, 0xD2, 0x00, 0x00, 0x0A, 0x01, 0x00, 0x10, 0x08,
    ];
    peer.send_to(&hello, local).await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.source_mac.as_slice(), PEER);
    assert_eq!(transport.lookup_vmac(peer_vmac), Some(peer_address));

    // Quiet for five minutes: the send still goes to the address on file,
    // and the check goes there too, at once.
    backdate(&transport, REVALIDATE_AFTER);
    let start = Instant::now();
    transport.send_unicast(NPDU, &PEER).await.unwrap();
    let mut got = vec![next(&peer).await, next(&peer).await];
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "the check waited {:?}",
        start.elapsed()
    );
    got.sort();
    assert_eq!(got, [TO_PEER.to_vec(), CHECK.to_vec()]);

    // The peer answers: it stays, and nothing more is asked.
    let answer = [0x82, 0x07, 0x00, 0x0A, 0x00, 0x04, 0xD2, 0x00, 0x00, 0x0A];
    peer.send_to(&answer, local).await.unwrap();
    settled(&transport).await;
    assert_eq!(transport.lookup_vmac(peer_vmac), Some(peer_address));
    let counters = transport.resolution_counters();
    assert_eq!(
        (counters.revalidations_sent, counters.revalidations_failed),
        (1, 0)
    );

    // Quiet again and silent: three checks a second apart, then the peer
    // is gone and its VMAC held unreachable.
    backdate(&transport, REVALIDATE_AFTER);
    transport.send_unicast(NPDU, &PEER).await.unwrap();
    settled(&transport).await;
    let mut got = Vec::new();
    for _ in 0..4 {
        got.push(next(&peer).await);
    }
    got.sort();
    let checks = vec![CHECK.to_vec(); 3];
    assert_eq!(got, [vec![TO_PEER.to_vec()], checks].concat());
    assert_eq!(transport.lookup_vmac(peer_vmac), None);
    let counters = transport.resolution_counters();
    assert_eq!(
        (counters.revalidations_sent, counters.revalidations_failed),
        (4, 1)
    );
    let held = transport.send_unicast(NPDU, &PEER).await;
    assert_eq!(kind(&held), Some(std::io::ErrorKind::HostUnreachable));

    // Once no longer held, a send resolves the VMAC afresh, through the
    // BBMD, and the peer's answer releases it.
    backdate(&transport, UNREACHABLE_FOR);
    let resolve = async {
        let request = next_request(&bbmd).await;
        let expected = [0x82, 0x03, 0x00, 0x0A, 0x00, 0x00, 0x0A, 0x00, 0x04, 0xD2];
        assert_eq!(request, expected);
        let ack = [0x82, 0x05, 0x00, 0x0A, 0x00, 0x04, 0xD2, 0x00, 0x00, 0x0A];
        peer.send_to(&ack, local).await.unwrap();
        assert_eq!(next(&peer).await, TO_PEER);
    };
    let (sent, ()) = tokio::join!(transport.send_unicast(NPDU, &PEER), resolve);
    sent.unwrap();
    let counters = transport.resolution_counters();
    assert_eq!(
        (counters.resolutions_started, counters.resolutions_resolved),
        (1, 1)
    );
    assert_eq!(transport.lookup_vmac(peer_vmac), Some(peer_address));
    transport.stop().await.unwrap();
}
