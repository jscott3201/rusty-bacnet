use super::*;
use tokio::time::{timeout, Duration};

async fn recv_bvll(socket: &UdpSocket) -> BvllMessage {
    let mut recv_buf = [0u8; 2048];
    let (len, _addr) = timeout(Duration::from_secs(2), socket.recv_from(&mut recv_buf))
        .await
        .expect("timed out waiting for BVLL frame")
        .unwrap();
    decode_bvll(&recv_buf[..len]).unwrap()
}

async fn assert_no_bvll(socket: &UdpSocket, label: &str) {
    let mut recv_buf = [0u8; 2048];
    assert!(
        timeout(Duration::from_millis(100), socket.recv_from(&mut recv_buf))
            .await
            .is_err(),
        "{label} received an unexpected BVLL frame"
    );
}

#[tokio::test]
async fn forwarded_npdu_from_bdt_peer_uses_originating_source_mac() {
    let socket = Arc::new(super::BipSocket::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
        None,
    ));
    let local_port = socket.local_addr().unwrap().port();
    let local_broadcast_sink = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let local_broadcast_port = local_broadcast_sink.local_addr().unwrap().port();
    let bdt_peer_sink = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let bdt_peer_port = bdt_peer_sink.local_addr().unwrap().port();
    let (npdu_tx, mut npdu_rx) = mpsc::channel(1);
    let peer = ([192, 0, 2, 10], 0xBAC0);
    let origin = ([192, 0, 2, 20], 0xBAC1);
    let mut state = BbmdState::new(Ipv4Addr::LOCALHOST.octets(), local_port);
    state
        .set_bdt(vec![
            BdtEntry {
                ip: peer.0,
                port: peer.1,
                broadcast_mask: [255, 255, 255, 255],
            },
            BdtEntry {
                ip: Ipv4Addr::LOCALHOST.octets(),
                port: bdt_peer_port,
                broadcast_mask: [255, 255, 255, 255],
            },
        ])
        .unwrap();

    let ctx = RecvContext {
        ..RecvContext::for_test(
            socket,
            npdu_tx,
            SocketAddrV4::new(Ipv4Addr::from(Ipv4Addr::LOCALHOST.octets()), local_port),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, local_broadcast_port),
            Some(Arc::new(std::sync::Mutex::new(state))),
        )
    };
    let msg = forwarded_message((origin.0, origin.1), &[0x01, 0x00, 0xAA, 0xBB]);

    handle_bvll_message(&msg, peer, Delivery::Unicast, &ctx).await;

    let received = timeout(Duration::from_secs(2), npdu_rx.recv())
        .await
        .expect("timed out waiting for received NPDU")
        .expect("NPDU channel closed");
    assert_eq!(received.npdu, msg.npdu().unwrap());
    assert_eq!(
        received.source_mac.as_slice(),
        &encode_bip_mac(origin.0, origin.1)
    );
    assert!(received.link_layer_group);

    let local_frame = recv_bvll(&local_broadcast_sink).await;
    assert_eq!(local_frame.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(origin_of(&local_frame), Some(origin));
    assert_eq!(local_frame.payload.as_ref(), msg.payload.as_ref());

    let mut bdt_peer_buf = [0u8; 2048];
    assert!(
        timeout(
            Duration::from_millis(100),
            bdt_peer_sink.recv_from(&mut bdt_peer_buf)
        )
        .await
        .is_err(),
        "Forwarded-NPDU from a BDT peer must not be re-forwarded to BDT peers"
    );
}

#[tokio::test]
async fn forwarded_npdu_from_non_bdt_sender_is_rejected_without_delivery() {
    let bbmd_socket = Arc::new(super::BipSocket::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
        None,
    ));
    let local_port = bbmd_socket.local_addr().unwrap().port();
    let local_broadcast_sink = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let local_broadcast_port = local_broadcast_sink.local_addr().unwrap().port();
    let bdt_peer_sink = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let bdt_peer_port = bdt_peer_sink.local_addr().unwrap().port();
    let fdt_socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let fdt_port = fdt_socket.local_addr().unwrap().port();
    let (npdu_tx, mut npdu_rx) = mpsc::channel(1);
    let sender = ([192, 0, 2, 250], 0xBAC4);
    let origin = ([192, 0, 2, 20], 0xBAC1);

    let mut state = BbmdState::new(Ipv4Addr::LOCALHOST.octets(), local_port);
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    state
        .set_bdt(vec![BdtEntry {
            ip: Ipv4Addr::LOCALHOST.octets(),
            port: bdt_peer_port,
            broadcast_mask: [255, 255, 255, 255],
        }])
        .unwrap();
    assert_eq!(
        state.register_foreign_device(Ipv4Addr::LOCALHOST.octets(), fdt_port, 60),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );

    let ctx = RecvContext {
        ..RecvContext::for_test(
            bbmd_socket,
            npdu_tx,
            SocketAddrV4::new(Ipv4Addr::from(Ipv4Addr::LOCALHOST.octets()), local_port),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, local_broadcast_port),
            Some(Arc::new(std::sync::Mutex::new(state))),
        )
    };
    let msg = forwarded_message((origin.0, origin.1), &[0x01, 0x00, 0xAA, 0xBB]);

    handle_bvll_message(&msg, sender, Delivery::Unicast, &ctx).await;

    assert!(
        timeout(Duration::from_millis(100), npdu_rx.recv())
            .await
            .is_err(),
        "Forwarded-NPDU from a non-BDT sender must not be delivered locally"
    );
    assert_no_bvll(&local_broadcast_sink, "local broadcast").await;
    assert_no_bvll(&bdt_peer_sink, "BDT peer").await;
    assert_no_bvll(&fdt_socket, "foreign device").await;
}

/// The case the #1623 review gave: this BBMD's own row is /32 (appended)
/// and the peer's /24. Peers send to this BBMD alone, so a unicast forward
/// from the peer goes to the local subnet as well as the foreign device
/// (Annex J.4.5). Before #1623 the peer's /24 row skipped the rebroadcast.
#[tokio::test]
async fn forwarded_npdu_from_one_hop_peer_is_rebroadcast_when_own_row_is_two_hop() {
    let bbmd_socket = Arc::new(super::BipSocket::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
        None,
    ));
    let local_port = bbmd_socket.local_addr().unwrap().port();
    let fdt_socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let fdt_port = fdt_socket.local_addr().unwrap().port();
    let local_broadcast_sink = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let local_broadcast_port = local_broadcast_sink.local_addr().unwrap().port();
    let bdt_peer_sink = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let bdt_peer_port = bdt_peer_sink.local_addr().unwrap().port();

    let (npdu_tx, _npdu_rx) = mpsc::channel(1);
    let peer = ([192, 0, 2, 10], 0xBAC0);
    let origin = ([192, 0, 2, 20], 0xBAC1);
    let mut state = BbmdState::new(Ipv4Addr::LOCALHOST.octets(), local_port);
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    state
        .set_bdt(vec![
            BdtEntry {
                ip: peer.0,
                port: peer.1,
                broadcast_mask: [255, 255, 255, 0],
            },
            BdtEntry {
                ip: Ipv4Addr::LOCALHOST.octets(),
                port: bdt_peer_port,
                broadcast_mask: [255, 255, 255, 255],
            },
        ])
        .unwrap();
    assert_eq!(
        state.register_foreign_device(Ipv4Addr::LOCALHOST.octets(), fdt_port, 60),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );

    let ctx = RecvContext {
        ..RecvContext::for_test(
            bbmd_socket,
            npdu_tx,
            SocketAddrV4::new(Ipv4Addr::from(Ipv4Addr::LOCALHOST.octets()), local_port),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, local_broadcast_port),
            Some(Arc::new(std::sync::Mutex::new(state))),
        )
    };
    let msg = forwarded_message((origin.0, origin.1), &[0x01, 0x00, 0xAA, 0xBB]);

    handle_bvll_message(&msg, peer, Delivery::Unicast, &ctx).await;

    let local_frame = recv_bvll(&local_broadcast_sink).await;
    assert_eq!(local_frame.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(origin_of(&local_frame), Some(origin));
    assert_eq!(local_frame.payload.as_ref(), msg.payload.as_ref());

    let fdt_frame = recv_bvll(&fdt_socket).await;
    assert_eq!(fdt_frame.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(origin_of(&fdt_frame), Some(origin));
    assert_eq!(fdt_frame.payload.as_ref(), msg.payload.as_ref());

    let mut bdt_peer_buf = [0u8; 2048];
    assert!(
        timeout(
            Duration::from_millis(100),
            bdt_peer_sink.recv_from(&mut bdt_peer_buf)
        )
        .await
        .is_err(),
        "Forwarded-NPDU from a BDT peer must not be re-forwarded to BDT peers"
    );
}

#[tokio::test]
async fn forwarded_npdu_fdt_fanout_respects_budget_and_increments_counter() {
    let bbmd_socket = Arc::new(super::BipSocket::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
        None,
    ));
    let local_port = bbmd_socket.local_addr().unwrap().port();
    let local_broadcast_sink = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let local_broadcast_port = local_broadcast_sink.local_addr().unwrap().port();

    let mut fd_sockets = Vec::new();
    let mut fd_ports = Vec::new();
    for _ in 0..3 {
        let sock = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        fd_ports.push(sock.local_addr().unwrap().port());
        fd_sockets.push(sock);
    }

    let (npdu_tx, _npdu_rx) = mpsc::channel(1);
    let peer = ([192, 0, 2, 10], 0xBAC0);
    let origin = ([192, 0, 2, 20], 0xBAC1);

    let mut state = BbmdState::new(Ipv4Addr::LOCALHOST.octets(), local_port);
    state.enable_foreign_device_registration(ForeignDevicePolicy {
        max_fdt_fanout: 2,
        ..ForeignDevicePolicy::default()
    });
    state
        .set_bdt(vec![BdtEntry {
            ip: peer.0,
            port: peer.1,
            broadcast_mask: [255, 255, 255, 255],
        }])
        .unwrap();

    for &port in &fd_ports {
        assert_eq!(
            state.register_foreign_device(Ipv4Addr::LOCALHOST.octets(), port, 60),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }

    let bbmd_state = Arc::new(std::sync::Mutex::new(state));
    let ctx = RecvContext {
        ..RecvContext::for_test(
            bbmd_socket,
            npdu_tx,
            SocketAddrV4::new(Ipv4Addr::from(Ipv4Addr::LOCALHOST.octets()), local_port),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, local_broadcast_port),
            Some(bbmd_state.clone()),
        )
    };
    let msg = forwarded_message((origin.0, origin.1), &[0x01, 0x00, 0xAA, 0xBB]);

    handle_bvll_message(&msg, peer, Delivery::Unicast, &ctx).await;

    // First two foreign devices received the frame
    let frame1 = recv_bvll(&fd_sockets[0]).await;
    assert_eq!(frame1.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(origin_of(&frame1), Some(origin));
    assert_eq!(frame1.payload.as_ref(), msg.payload.as_ref());

    let frame2 = recv_bvll(&fd_sockets[1]).await;
    assert_eq!(frame2.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(origin_of(&frame2), Some(origin));
    assert_eq!(frame2.payload.as_ref(), msg.payload.as_ref());

    // Third foreign device was capped and received nothing
    assert_no_bvll(&fd_sockets[2], "third foreign device").await;

    // Counter was incremented
    let counters = bbmd_state.lock().unwrap().fdt_counters();
    assert_eq!(counters.fanout_budget_reached, 1);
}

#[tokio::test]
async fn forwarded_npdu_bbmd_self_sender_is_ignored_before_delivery_and_fanout() {
    let socket = Arc::new(super::BipSocket::new(
        UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap(),
        None,
    ));
    let local_port = socket.local_addr().unwrap().port();
    let local_sink = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let foreign = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let peer_sender = (
        Ipv4Addr::LOCALHOST.octets(),
        peer.local_addr().unwrap().port(),
    );
    let self_sender = (Ipv4Addr::LOCALHOST.octets(), local_port);
    let mut state = BbmdState::new(self_sender.0, local_port);
    state
        .set_bdt(vec![BdtEntry {
            ip: peer_sender.0,
            port: peer_sender.1,
            broadcast_mask: [255; 4],
        }])
        .unwrap();
    assert!(
        state.is_bdt_peer(self_sender.0, self_sender.1),
        "retain the required self BDT entry"
    );
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    assert_eq!(
        state.register_foreign_device(
            Ipv4Addr::LOCALHOST.octets(),
            foreign.local_addr().unwrap().port(),
            60
        ),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    let (npdu_tx, mut npdu_rx) = mpsc::channel(1);
    let ctx = RecvContext {
        ..RecvContext::for_test(
            socket,
            npdu_tx,
            SocketAddrV4::new(Ipv4Addr::from(self_sender.0), local_port),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, local_sink.local_addr().unwrap().port()),
            Some(Arc::new(std::sync::Mutex::new(state))),
        )
    };
    let mut msg = forwarded_message(([192, 0, 2, 9], 47808), &[1, 0x80, 0x13, 0, 99, 1]);
    // Remote embedded origin does not make our looped-back UDP send a peer.
    handle_bvll_message(&msg, self_sender, Delivery::Unicast, &ctx).await;
    msg = forwarded_message(([192, 0, 2, 9], 47808), &[1, 0x80, 0x13, 0, 77, 1]);
    // Same IP, different port is an admitted peer: this fences every output
    // without a timeout-only silence assertion and rejects an IP-only filter.
    handle_bvll_message(&msg, peer_sender, Delivery::Unicast, &ctx).await;
    let received = timeout(Duration::from_secs(2), npdu_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.npdu, msg.npdu().unwrap());
    assert!(received.link_layer_group);
    assert_eq!(
        received.source_mac.as_slice(),
        encode_bip_mac([192, 0, 2, 9], 47808)
    );
    for sink in [&local_sink, &foreign] {
        let received = recv_bvll(sink).await;
        assert_eq!(received.function, BvlcFunction::FORWARDED_NPDU);
        assert_eq!(received.payload, msg.payload);
        assert_eq!(origin_of(&received), origin_of(&msg));
    }
}

#[test]
fn delivery_is_broadcast_only_for_the_configured_or_limited_broadcast_address() {
    use bacnet_bip_core::node::{Addressed, LocalAddresses};
    let configured = Ipv4Addr::new(192, 0, 2, 255);
    let local = LocalAddresses::new(Ipv4Addr::new(192, 0, 2, 10), configured);
    for (destination, os, broadcast) in [
        (configured, None, true),
        (Ipv4Addr::BROADCAST, None, true),
        (configured, Some(true), true),
        // An OS that reports unicast delivery overrides the destination.
        (configured, Some(false), false),
        (Ipv4Addr::new(192, 0, 2, 10), None, false),
        (Ipv4Addr::new(198, 51, 100, 255), None, false),
    ] {
        assert_eq!(
            Addressed::classify(destination, os, &local).broadcast,
            broadcast,
            "{destination} {os:?}"
        );
    }
}

/// A Forwarded-NPDU from a one-hop (/32) BDT peer is rebroadcast locally only
/// when it came by unicast: one that came by broadcast already reached the
/// subnet, and rebroadcasting it could loop (#937).
#[tokio::test]
async fn forwarded_npdu_arriving_by_broadcast_is_not_rebroadcast_locally() {
    let socket = Arc::new(super::BipSocket::new(
        UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap(),
        None,
    ));
    let local_port = socket.local_addr().unwrap().port();
    let local_sink = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let foreign = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let peer = ([192, 0, 2, 10], 0xBAC0);
    let mut state = BbmdState::new(Ipv4Addr::LOCALHOST.octets(), local_port);
    state
        .set_bdt(vec![BdtEntry {
            ip: peer.0,
            port: peer.1,
            broadcast_mask: [255; 4],
        }])
        .unwrap();
    assert!(state.forwarded_npdu_needs_local_broadcast(peer.0, peer.1));
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    assert_eq!(
        state.register_foreign_device(
            Ipv4Addr::LOCALHOST.octets(),
            foreign.local_addr().unwrap().port(),
            60
        ),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    let (npdu_tx, mut npdu_rx) = mpsc::channel(2);
    let ctx = RecvContext {
        ..RecvContext::for_test(
            socket,
            npdu_tx,
            SocketAddrV4::new(Ipv4Addr::from(Ipv4Addr::LOCALHOST.octets()), local_port),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, local_sink.local_addr().unwrap().port()),
            Some(Arc::new(std::sync::Mutex::new(state))),
        )
    };
    let msg = forwarded_message(([192, 0, 2, 20], 0xBAC1), &[0x01, 0x00, 0xAA, 0xBB]);
    let assert_forwarded = |frame: &BvllMessage, label: &str| {
        assert_eq!(frame.function, BvlcFunction::FORWARDED_NPDU, "{label}");
        assert_eq!(origin_of(frame), origin_of(&msg), "{label}");
        assert_eq!(frame.payload, msg.payload, "{label}");
    };

    handle_bvll_message(&msg, peer, Delivery::Broadcast, &ctx).await;
    // Still delivered to this device and sent on to the foreign device, which
    // is sent after any local rebroadcast.
    let received = timeout(Duration::from_secs(2), npdu_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.npdu, msg.npdu().unwrap());
    assert_forwarded(&recv_bvll(&foreign).await, "foreign device");
    assert_no_bvll(&local_sink, "local broadcast after a broadcast arrival").await;

    handle_bvll_message(&msg, peer, Delivery::Unicast, &ctx).await;
    assert_forwarded(&recv_bvll(&local_sink).await, "local broadcast");
    assert_forwarded(&recv_bvll(&foreign).await, "foreign device");
}
