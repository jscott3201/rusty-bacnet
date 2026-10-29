//! Standalone-client wire qualification on an explicitly isolated IPv6 link.
// The packet-info oracle (wire.rs) needs nix's decoded IPV6_PKTINFO, which
// it gives only on these targets.
#![cfg(all(
    feature = "ipv6",
    any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_vendor = "apple"
    )
))]
#![forbid(unsafe_code)]
#![allow(clippy::print_stderr)] // the qualification helpers log each observed wire frame
#[path = "ipv6_network_numbers/observer.rs"]
mod observer;
#[path = "../../bacnet-transport/tests/ipv6_selected_link/support.rs"]
#[allow(dead_code)] // Shared #887 fixture also serves its application-NPDU controls.
mod support;
#[path = "../../bacnet-transport/tests/ipv6_selected_link/wire.rs"]
mod wire;
// The closing rebinds can lose the freed port to another process (#1419).
#[path = "../../bacnet-endpoint/tests/support/port_retry.rs"]
mod port_retry;
use port_retry::{bind_eventually, rerun_on_lost_port};

use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_transport::bip6::{Bip6ForeignDeviceConfig, Bip6Transport};
use std::net::{Ipv6Addr, SocketAddrV6};
use support::{DEADLINE, GROUP, fixture, udp};
use tokio::net::UdpSocket;

const NODE: [u8; 3] = [0x12, 0x34, 0x56];
const PEER: [u8; 3] = [0x40, 0x87, 0x09];
const QUERY: &[u8] = &[1, 0x80, 0x12];
type Client = BACnetClient<Bip6Transport>;

async fn bounded<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("isolated IPv6 fixture made no progress")
}

fn frame(function: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x82, function];
    bytes.extend_from_slice(&((7 + payload.len()) as u16).to_be_bytes());
    bytes.extend_from_slice(&PEER);
    bytes.extend_from_slice(payload);
    bytes
}

fn number(n: u16, flag: u8) -> Vec<u8> {
    vec![1, 0x80, 0x13, (n >> 8) as u8, n as u8, flag]
}

async fn start(transport: Bip6Transport) -> (Client, SocketAddrV6) {
    let client = bounded(BACnetClient::start(ClientConfig::default(), transport))
        .await
        .unwrap();
    let address = client.transport().local_address().unwrap();
    (
        client,
        SocketAddrV6::new(*address.ip(), address.port(), 0, 0),
    )
}

async fn send(peer: &UdpSocket, target: SocketAddrV6, group: bool, npdu: &[u8]) {
    let mut payload = Vec::new();
    if !group {
        payload.extend_from_slice(&NODE);
    }
    payload.extend_from_slice(npdu);
    bounded(peer.send_to(&frame(if group { 2 } else { 1 }, &payload), target))
        .await
        .unwrap();
}

async fn local_number(socket: &UdpSocket) -> wire::Frame {
    loop {
        let p = wire::receive(socket).await.unwrap();
        if p.bytes.len() >= 7 && p.bytes[4..7] == NODE {
            return p;
        }
    }
}

async fn expect_number(
    next: impl std::future::Future<Output = wire::Frame>,
    client: SocketAddrV6,
    index: u32,
    foreign: bool,
    expected: u16,
) {
    let packet = bounded(next).await;
    eprintln!("number wire={packet:?}");
    let expected_npdu = number(expected, 0);
    let mut expected_wire = vec![0x82, if foreign { 0x0c } else { 2 }, 0, 13];
    expected_wire.extend_from_slice(&NODE);
    expected_wire.extend_from_slice(&expected_npdu);
    assert_eq!(packet.bytes, expected_wire);
    assert_eq!(packet.source, client);
    assert_eq!(
        packet.destination,
        if foreign { *client.ip() } else { GROUP }
    );
    assert_eq!(packet.index, index);
}

#[tokio::test]
#[ignore = "requires an explicitly supplied isolated IPv6 link and multicast wire observer"]
async fn normal_client_network_number_wire() {
    rerun_on_lost_port(async || {
        let (selected, index) = fixture();
        let (mut client, local) = start(Bip6Transport::new(selected, 0, Some(0x12_3456))).await;
        assert_eq!(*local.ip(), selected);
        // A separate network namespace avoids sender-local multicast reflection.
        let mut observer = observer::Observer::subscribe(local.port()).await;
        let observation_index = observer.index;
        let peer = udp(selected, 0, index, false);
        let group = SocketAddrV6::new(GROUP, local.port(), 0, index);
        send(&peer, local, false, &number(999, 1)).await;
        send(&peer, local, false, QUERY).await; // UNKNOWN must remain silent.
        receive_fence(&peer, local).await;
        send(&peer, group, true, QUERY).await; // UNKNOWN broadcast is silent too.
        send(&peer, group, true, &number(77, 0)).await;
        send(&peer, group, true, QUERY).await;
        expect_number(observer.number(), local, observation_index, false, 77).await;
        send(&peer, local, false, QUERY).await;
        expect_number(observer.number(), local, observation_index, false, 77).await;
        for (value, flag, expected) in [(78, 0, 78), (79, 1, 79), (80, 0, 79), (81, 1, 81)] {
            send(&peer, group, true, &number(value, flag)).await;
            send(&peer, group, true, QUERY).await;
            expect_number(observer.number(), local, observation_index, false, expected).await;
        }
        send(&peer, local, false, &number(200, 1)).await;
        receive_fence(&peer, local).await;
        for invalid in invalid_announcements() {
            send(&peer, group, true, &invalid).await;
            send(&peer, group, true, QUERY).await;
            expect_number(observer.number(), local, observation_index, false, 81).await;
        }
        for (i, invalid) in invalid_queries().iter().enumerate() {
            send(&peer, local, false, invalid).await;
            receive_fence(&peer, local).await;
            send(&peer, group, true, &number(82 + i as u16, 1)).await;
            send(&peer, group, true, QUERY).await;
            // A forbidden response to the preceding query would carry the old
            // number and fail here, before the positive marker on the control FIFO.
            expect_number(
                observer.number(),
                local,
                observation_index,
                false,
                82 + i as u16,
            )
            .await;
        }
        // OriginalBroadcast delivered to local unicast must not teach either.
        send(&peer, local, true, &number(200, 1)).await;
        receive_fence(&peer, local).await;
        send(&peer, group, true, QUERY).await;
        expect_number(observer.number(), local, observation_index, false, 84).await;
        let mut wrong_vmac_query = vec![0x12, 0x34, 0x57];
        wrong_vmac_query.extend_from_slice(QUERY);
        bounded(peer.send_to(&frame(1, &wrong_vmac_query), local))
            .await
            .unwrap();
        receive_fence(&peer, local).await;
        send(&peer, group, true, &number(85, 1)).await;
        send(&peer, group, true, QUERY).await;
        expect_number(observer.number(), local, observation_index, false, 85).await;
        drop(observer);
        stop_and_reuse(&mut client, local).await?;
        // A newly constructed runtime starts UNKNOWN, even on the identical port.
        let (restarted, local) =
            start(Bip6Transport::new(selected, local.port(), Some(0x12_3456))).await;
        let mut observer = observer::Observer::subscribe(local.port()).await;
        send(&peer, local, false, QUERY).await;
        receive_fence(&peer, local).await;
        send(&peer, group, true, &number(90, 0)).await;
        send(&peer, group, true, QUERY).await;
        expect_number(observer.number(), local, observation_index, false, 90).await;
        drop(observer);
        drop_and_reuse(restarted, local).await
    })
    .await;
}

fn invalid_announcements() -> Vec<Vec<u8>> {
    vec![
        number(65535, 1),
        number(200, 2),
        vec![1, 0x80, 0x13, 0, 200],
        vec![1, 0x88, 0, 4, 1, 9, 0x13, 0, 200, 1],
        vec![1, 0xa0, 0xff, 0xff, 0, 255, 0x13, 0, 200, 1],
    ]
}

fn invalid_queries() -> Vec<Vec<u8>> {
    vec![
        vec![1, 0x80, 0x12, 0],
        vec![1, 0x88, 0, 4, 1, 9, 0x12],
        vec![1, 0xa0, 0xff, 0xff, 0, 255, 0x12],
    ]
}

async fn stop_and_reuse(client: &mut Client, local: SocketAddrV6) -> std::io::Result<()> {
    bounded(client.stop()).await.unwrap();
    // No reuse option: graceful stop must release the owned socket before Drop.
    std::net::UdpSocket::bind(local).map(drop)
}

async fn drop_and_reuse(client: Client, local: SocketAddrV6) -> std::io::Result<()> {
    drop(client);
    // Drop requests abort; exclusive rebind is a positive eventual release
    // event. No quiet receive window or retraction of prior bytes is claimed.
    bind_eventually(DEADLINE, || std::net::UdpSocket::bind(local))
        .await
        .map(drop)
}

async fn receive_fence(peer: &UdpSocket, local: SocketAddrV6) {
    // Assuming ordered delivery on this controlled same-path UDP fixture, the
    // ACK shows receive-loop progress, not successful queue admission or Number
    // worker completion. The later exact-number response is the state oracle.
    bounded(peer.send_to(&frame(6, &[]), local)).await.unwrap();
    let ack = bounded(wire::receive(peer)).await.unwrap();
    assert_eq!(ack.bytes, [0x82, 7, 0, 10, 0x12, 0x34, 0x56, 0x40, 0x87, 9]);
}

async fn forward(bbmd: &UdpSocket, target: SocketAddrV6, npdu: &[u8]) {
    let mut payload = target.ip().octets().to_vec();
    payload.extend_from_slice(&bbmd.local_addr().unwrap().port().to_be_bytes());
    payload.extend_from_slice(npdu);
    bounded(bbmd.send_to(&frame(8, &payload), target))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires an explicitly supplied isolated IPv6 link and independent BBMD"]
async fn foreign_client_network_number_wire() {
    rerun_on_lost_port(async || {
        let (selected, index) = fixture();
        let bbmd = udp(selected, 0, index, false);
        let mut transport = Bip6Transport::new(Ipv6Addr::UNSPECIFIED, 0, Some(0x12_3456));
        transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
            bbmd_ip: selected,
            bbmd_port: bbmd.local_addr().unwrap().port(),
            ttl: 60,
        });
        let (mut client, local) = start(transport).await;
        let registration = bounded(wire::receive(&bbmd)).await.unwrap();
        assert_eq!(registration.bytes, [0x82, 9, 0, 9, 0x12, 0x34, 0x56, 0, 60]);
        assert_eq!(registration.source, local);
        send(&bbmd, local, false, QUERY).await; // No configured client number authority.
        forward(&bbmd, local, &number(77, 1)).await;
        send(&bbmd, local, false, QUERY).await;
        expect_number(local_number(&bbmd), local, index, true, 77).await;
        // Actual UDP unicast delivery of Forwarded-NPDU still represents a group.
        forward(&bbmd, local, QUERY).await;
        expect_number(local_number(&bbmd), local, index, true, 77).await;
        for (value, flag, expected) in [(78, 0, 77), (79, 1, 79)] {
            forward(&bbmd, local, &number(value, flag)).await;
            send(&bbmd, local, false, QUERY).await;
            expect_number(local_number(&bbmd), local, index, true, expected).await;
        }
        send(&bbmd, local, false, &number(200, 1)).await;
        for invalid in invalid_announcements() {
            forward(&bbmd, local, &invalid).await;
            send(&bbmd, local, false, QUERY).await;
            expect_number(local_number(&bbmd), local, index, true, 79).await;
        }
        let mut unusable = Ipv6Addr::LOCALHOST.octets().to_vec();
        unusable.extend_from_slice(&bbmd.local_addr().unwrap().port().to_be_bytes());
        unusable.extend_from_slice(&number(200, 1));
        bounded(bbmd.send_to(&frame(8, &unusable), local))
            .await
            .unwrap();
        send(&bbmd, local, false, QUERY).await;
        expect_number(local_number(&bbmd), local, index, true, 79).await;
        let impostor = udp(selected, 0, index, false);
        forward(&impostor, local, &number(200, 1)).await;
        // A transport-level response from this same sender fences receive progress
        // without claiming the untrusted Forwarded-NPDU was admitted to the owner.
        bounded(impostor.send_to(&frame(6, &[]), local))
            .await
            .unwrap();
        let ack = bounded(wire::receive(&impostor)).await.unwrap();
        assert_eq!(ack.bytes, [0x82, 7, 0, 10, 0x12, 0x34, 0x56, 0x40, 0x87, 9]);
        send(&bbmd, local, false, QUERY).await;
        expect_number(local_number(&bbmd), local, index, true, 79).await;
        for (i, invalid) in invalid_queries().iter().enumerate() {
            forward(&bbmd, local, invalid).await;
            forward(&bbmd, local, &number(82 + i as u16, 1)).await;
            send(&bbmd, local, false, QUERY).await;
            expect_number(local_number(&bbmd), local, index, true, 82 + i as u16).await;
        }
        stop_and_reuse(&mut client, local).await
    })
    .await;
}
