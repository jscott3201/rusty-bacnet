//! Full-server local Number ownership on existing BBMD/foreign B/IP modes.
//! Actual loopback UDP only. BBMD broadcast observation requires Linux sharing.
#[path = "bip_network_numbers/support.rs"]
mod support;
use bacnet_encoding::{
    apdu::{decode_apdu, Apdu},
    npdu::decode_npdu,
};
use bacnet_transport::bip::{BipTransport, ForeignDeviceConfig};
use bytes::Bytes;
use std::net::{Ipv4Addr, SocketAddrV4};
use support::*;
use tokio::net::UdpSocket;

async fn foreign(gates: Option<std::sync::Arc<Gates>>) -> (Server, SocketAddrV4, UdpSocket) {
    let bbmd = udp().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, BROADCAST);
    transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: address(&bbmd).port(),
        renewal_interval: None,
        ttl: 60,
    });
    let (server, local) = start(transport, gates).await;
    let (registration, source) = receive(&bbmd).await;
    assert_eq!(source, local);
    assert_eq!(registration, frame(5, &60u16.to_be_bytes()));
    (server, local, bbmd)
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
async fn bip_number_foreign_full_server_wire_and_alternate_sender() {
    let (server, local, bbmd) = foreign(None).await;
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
    stopped(server, local).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn bip_number_bbmd_full_server_wire_admission() {
    use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy};
    let bdt = udp().await;
    let foreign = udp().await;
    let peer = udp().await;
    let impostor = udp().await;
    let (observer, port) = observer();
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, port, BROADCAST);
    transport.enable_bbmd(vec![BdtEntry {
        ip: Ipv4Addr::LOCALHOST.octets(),
        port: address(&bdt).port(),
        broadcast_mask: [255; 4],
    }]);
    transport.enable_foreign_device_registration(ForeignDevicePolicy::default());
    let (server, local) = start(transport, None).await;
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
    stopped(server, local).await;
}

#[tokio::test]
async fn bip_number_foreign_registration_loss_and_retry_wire() {
    let (server, local, bbmd) = foreign(None).await;
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
    stopped(server, local).await;
}

async fn gated_progress_and_stop(bbmd_mode: bool, bare_drop: bool) {
    let gates = Gates::new();
    let peer = udp().await;
    #[cfg(target_os = "linux")]
    let observed = bbmd_mode.then(observer);
    #[cfg(target_os = "linux")]
    let port = observed.as_ref().map_or(0, |(_, port)| *port);
    #[cfg(not(target_os = "linux"))]
    let port = 0;
    let (mut server, local, bbmd) = if bbmd_mode {
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, port, BROADCAST);
        transport.enable_bbmd(vec![]);
        let (server, local) = start(transport, Some(gates.clone())).await;
        (server, local, None)
    } else {
        let (server, local, bbmd) = foreign(Some(gates.clone())).await;
        (server, local, Some(bbmd))
    };
    #[cfg(target_os = "linux")]
    let captured = observed.map(|(socket, _)| socket);
    if let Some(bbmd) = &bbmd {
        forward(bbmd, local, &number(77, 1)).await;
        forward(bbmd, local, QUERY).await;
    } else {
        let group = SocketAddrV4::new(BROADCAST, local.port());
        send(&peer, group, &frame(0x0b, &number(77, 1))).await;
        send(&peer, local, &frame(0x0a, QUERY)).await;
    }
    bounded(gates.entered.acquire()).await.unwrap().forget();
    let request = [1, 4, 0, 3, 42, 12, 0x0c, 0, 0, 0, 1, 0x19, 85];
    send(&peer, local, &frame(0x0a, &request)).await;
    let (wire, source) = receive(&peer).await;
    assert_eq!(source, local);
    assert_eq!(wire[1], 0x0a);
    let npdu = decode_npdu(Bytes::copy_from_slice(&wire[4..])).unwrap();
    let Apdu::ComplexAck(ack) = decode_apdu(npdu.payload).unwrap() else {
        panic!("ReadProperty ACK")
    };
    assert_eq!(ack.invoke_id, 42);
    assert_eq!(ack.service_choice.to_raw(), 12);
    assert_eq!(
        gates.dropped.available_permits(),
        0,
        "Number producer remains held"
    );
    if bare_drop {
        drop(server);
    } else {
        {
            let stop = server.stop();
            tokio::pin!(stop);
            bounded(async {
                tokio::select! {
                    _ = &mut stop => panic!("held transport stop"),
                    permit = gates.stop_entered.acquire() => { permit.unwrap().forget(); }
                }
            })
            .await;
        }
        bounded(gates.dropped.acquire()).await.unwrap().forget();
        gates.stop_release.add_permits(1);
        bounded(server.stop()).await.unwrap();
        drop(server);
    }
    if bare_drop {
        bounded(gates.dropped.acquire()).await.unwrap().forget();
    }
    #[cfg(target_os = "linux")]
    if let Some(captured) = captured {
        let mut bytes = [0; 2048];
        while let Ok((n, source)) = captured.try_recv_from(&mut bytes) {
            assert!(
                source != local.into() || bytes[..n] != frame(0x0b, &number(77, 0)),
                "held Number send never reached broadcast observer"
            );
        }
        drop(captured);
    }
    // Drop cleanup may finish asynchronously; successful exclusive bind is the
    // positive release event, with a watchdog rather than a fixed sleep.
    let rebound = bounded(async {
        loop {
            if let Ok(s) = std::net::UdpSocket::bind(local) {
                break s;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    drop(rebound);
    if let Some(bbmd) = bbmd {
        let mut bytes = [0; 2048];
        assert!(
            bbmd.try_recv_from(&mut bytes).is_err(),
            "held Number send never reached BBMD"
        );
    }
}
#[tokio::test]
async fn bip_number_foreign_held_producer_progress_stop_and_drop() {
    for drop in [false, true] {
        gated_progress_and_stop(false, drop).await;
    }
}
#[cfg(target_os = "linux")]
#[tokio::test]
async fn bip_number_bbmd_held_producer_progress_stop_and_drop() {
    for drop in [false, true] {
        gated_progress_and_stop(true, drop).await;
    }
}
