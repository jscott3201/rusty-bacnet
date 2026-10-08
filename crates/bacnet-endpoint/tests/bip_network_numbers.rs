//! Shared endpoint Number ownership over actual BBMD/foreign loopback UDP.
//! Actual loopback UDP only. BBMD broadcast observation requires Linux sharing.
#[cfg(target_os = "linux")]
#[path = "bip_network_numbers/composed.rs"]
mod composed;
#[path = "support/port_retry.rs"]
mod port_retry;
#[path = "bip_network_numbers/support.rs"]
mod support;
use bacnet_endpoint::session::SessionRole;
use bacnet_transport::bip::ForeignDeviceConfig;
use port_retry::rerun_on_lost_port;
use std::net::{Ipv4Addr, SocketAddrV4};
use support::*;
use tokio::net::UdpSocket;

async fn foreign() -> (Endpoint, SocketAddrV4, UdpSocket) {
    let bbmd = udp().await;
    let (endpoint, local) = start(
        builder()
            .role(SessionRole::ClientOnly)
            .register_as_foreign_device(ForeignDeviceConfig {
                bbmd_ip: Ipv4Addr::LOCALHOST,
                bbmd_port: address(&bbmd).port(),
                renewal_interval: None,
                ttl: 60,
            }),
    )
    .await;
    assert!(endpoint.client().is_some());
    assert!(endpoint.server().is_none());
    let (registration, source) = receive(&bbmd).await;
    assert_eq!(source, local);
    assert_eq!(registration, frame(5, &60u16.to_be_bytes()));
    (endpoint, local, bbmd)
}
async fn forward(socket: &UdpSocket, target: SocketAddrV4, npdu: &[u8]) {
    let origin = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 9), address(socket).port());
    send(socket, target, &forwarded(origin, npdu)).await;
}
async fn invalid_controls(
    peer: &UdpSocket,
    observer: &UdpSocket,
    local: SocketAddrV4,
    function: u8,
    group_function: u8,
) {
    let send_group = |npdu: Vec<u8>| async move {
        let bytes = if group_function == 4 {
            forwarded(address(peer), &npdu)
        } else {
            frame(group_function, &npdu)
        };
        let target = if group_function == 0x0b {
            SocketAddrV4::new(BROADCAST, local.port())
        } else {
            local
        };
        send(peer, target, &bytes).await;
    };
    send(peer, local, &frame(0x0a, &number(200, 1))).await;
    fence(peer, local).await;
    for npdu in [
        number(0, 1),
        number(65535, 1),
        number(200, 2),
        vec![1, 0x80, 0x13, 0, 200],
        vec![1, 0x80, 0x13, 0, 200, 1, 0],
        vec![1, 0x88, 0, 4, 1, 9, 0x13, 0, 200, 1],
        vec![1, 0xa0, 255, 255, 0, 255, 0x13, 0, 200, 1],
    ] {
        send_group(npdu).await;
        send_group(QUERY.to_vec()).await;
        expect_number(observer, local, function, 81).await;
    }
    for (i, npdu) in [
        vec![1, 0x80, 0x12, 0],
        vec![1, 0x88, 0, 4, 1, 9, 0x12],
        vec![1, 0xa0, 255, 255, 0, 255, 0x12],
    ]
    .into_iter()
    .enumerate()
    {
        send_group(npdu).await;
        send_group(number(82 + i as u16, 1)).await;
        send_group(QUERY.to_vec()).await;
        expect_number(observer, local, function, 82 + i as u16).await;
    }
}

#[tokio::test]
async fn endpoint_number_foreign_client_wire_and_alternate_sender() {
    // The closing release probe can lose the port to another process (#1070).
    rerun_on_lost_port(async || {
        let (server, local, bbmd) = foreign().await;
        let alternate = udp().await;
        send(&bbmd, local, &frame(0, &[0, 0])).await;
        // No configured authority from unrelated Network Port999. An unknown query
        // and unicast announcement must not cause a response before the valid fence.
        send(&alternate, local, &frame(0x0a, QUERY)).await;
        send(&alternate, local, &frame(0x0a, &number(999, 1))).await;
        send(&alternate, local, &frame(0x0a, QUERY)).await;
        forward(&alternate, local, QUERY).await;
        fence(&alternate, local).await;
        forward(&bbmd, local, &number(77, 0)).await;
        forward(&bbmd, local, QUERY).await;
        expect_number(&bbmd, local, 9, 77).await;
        send(&alternate, local, &frame(0x0a, QUERY)).await;
        expect_number(&bbmd, local, 9, 77).await;
        for (value, flag, expected) in [(78, 0, 78), (79, 1, 79), (80, 0, 79), (81, 1, 81)] {
            // Existing compatibility: valid forwarded traffic is not restricted to
            // the configured BBMD UDP sender. Its logical group bit is not trust.
            forward(&alternate, local, &number(value, flag)).await;
            forward(&alternate, local, QUERY).await;
            expect_number(&bbmd, local, 9, expected).await;
        }
        // Too-short forwarded BVLC and a length larger than the datagram are invalid.
        send(&alternate, local, &[0x81, 4, 0, 9, 127, 0, 0, 1, 0]).await;
        send(&alternate, local, &[0x81, 4, 0, 30, 127, 0, 0, 1, 0, 1]).await;
        forward(&alternate, local, QUERY).await;
        expect_number(&bbmd, local, 9, 81).await;
        invalid_controls(&alternate, &bbmd, local, 9, 4).await;
        // Model peer registration responses, without inventing a transport status bit.
        send(&bbmd, local, &frame(0, &[0, 0])).await;
        forward(&bbmd, local, &number(77, 1)).await;
        forward(&bbmd, local, QUERY).await;
        expect_number(&bbmd, local, 9, 77).await;
        send(&bbmd, local, &frame(0, &[0, 0x30])).await; // registration now refused
        send(&bbmd, local, &frame(0, &[0, 0x60])).await; // distribution refused after loss
        forward(&bbmd, local, QUERY).await;
        // DBTN is still attempted after both registration and distribution NAKs.
        expect_number(&bbmd, local, 9, 77).await;
        // The completed exchanges gave the spawned registration timer a turn.
        // Advance only the local test runtime; resume before awaiting real UDP.
        tokio::time::pause();
        tokio::time::advance(std::time::Duration::from_secs(31)).await;
        tokio::time::resume();
        let (retry, source) = receive(&bbmd).await;
        assert_eq!(source, local);
        assert_eq!(retry, frame(5, &60u16.to_be_bytes()));
        send(&bbmd, local, &frame(0, &[0, 0])).await;
        forward(&bbmd, local, &number(78, 1)).await;
        forward(&bbmd, local, QUERY).await;
        expect_number(&bbmd, local, 9, 78).await;
        stopped(server, local).await
    })
    .await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn endpoint_number_bbmd_server_wire_admission() {
    // The closing release probe can lose the port to another process (#1070).
    rerun_on_lost_port(async || {
        use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy};
        let bdt = udp().await;
        let foreign = udp().await;
        let peer = udp().await;
        let impostor = udp().await;
        let (observer, port) = observer();
        let configured = builder_on(port)
            .role(SessionRole::ServerOnly)
            .enable_bbmd(vec![BdtEntry {
                ip: Ipv4Addr::LOCALHOST.octets(),
                port: address(&bdt).port(),
                broadcast_mask: [255; 4],
            }])
            .foreign_device_policy(ForeignDevicePolicy::default());
        let (server, local) = start(configured).await;
        assert!(server.server().is_some());
        assert!(server.client().is_none());
        let group = SocketAddrV4::new(BROADCAST, local.port());
        // Before relying on shared capture, prove the real production wildcard
        // receives this broadcast (BDT fanout) and the independent observer sees it.
        send(&peer, group, &frame(0x0b, QUERY)).await;
        let (seen, source) = receive(&observer).await;
        assert_eq!(source, address(&peer));
        assert_eq!(seen, frame(0x0b, QUERY));
        let (fanout, source) = receive(&bdt).await;
        assert_eq!(source, local);
        assert_eq!(fanout, forwarded(address(&peer), QUERY));
        send(&peer, local, &frame(0x0a, &number(999, 1))).await;
        send(&peer, local, &frame(0x0a, QUERY)).await;
        fence(&peer, local).await;
        send(&peer, group, &frame(0x0b, &number(77, 0))).await;
        send(&peer, group, &frame(0x0b, QUERY)).await;
        expect_number(&observer, local, 0x0b, 77).await;
        send(&peer, local, &frame(0x0a, QUERY)).await;
        expect_number(&observer, local, 0x0b, 77).await;
        forward(&impostor, local, &number(200, 1)).await;
        fence(&impostor, local).await;
        send(&peer, local, &frame(0x0a, QUERY)).await;
        expect_number(&observer, local, 0x0b, 77).await;
        forward(&bdt, local, &number(78, 0)).await;
        forward(&bdt, local, QUERY).await;
        expect_number(&observer, local, 0x0b, 78).await;
        // Unregistered DBTN is NAKed and cannot teach.
        send(&foreign, local, &frame(9, &number(200, 1))).await;
        let (nak, source) = receive(&foreign).await;
        assert_eq!(source, local);
        assert_eq!(nak, frame(0, &[0, 0x60]));
        send(&peer, local, &frame(0x0a, QUERY)).await;
        expect_number(&observer, local, 0x0b, 78).await;
        send(&foreign, local, &frame(5, &60u16.to_be_bytes())).await;
        let (registered, source) = receive(&foreign).await;
        assert_eq!(source, local);
        assert_eq!(registered, frame(0, &[0, 0]));
        for (value, flag, expected) in [(79, 1, 79), (80, 0, 79), (81, 1, 81)] {
            send(&foreign, local, &frame(9, &number(value, flag))).await;
            send(&foreign, local, &frame(9, QUERY)).await;
            expect_number(&observer, local, 0x0b, expected).await;
        }
        invalid_controls(&peer, &observer, local, 0x0b, 0x0b).await;
        drop(observer); // release the independently shared bind before reuse proof
        stopped(server, local).await
    })
    .await;
}
