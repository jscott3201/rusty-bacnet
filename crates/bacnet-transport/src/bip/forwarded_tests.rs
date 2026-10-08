use super::*;
use bytes::Bytes;
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
        local_mac: encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), local_port),
        socket,
        npdu_tx,
        bbmd: Some(Arc::new(std::sync::Mutex::new(state))),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_broadcast_port,
        client_management: Arc::default(),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        fanout: None,
        force_dbtn_forward_failure: false,
        group_sources: super::groups::GroupSources::detached(),
    };
    let msg = BvllMessage {
        function: BvlcFunction::FORWARDED_NPDU,
        payload: Bytes::from_static(&[0x01, 0x00, 0xAA, 0xBB]),
        originating_ip: Some(origin.0),
        originating_port: Some(origin.1),
    };

    handle_bvll_message(&msg, peer, Delivery::Unicast, &ctx).await;

    let received = timeout(Duration::from_secs(2), npdu_rx.recv())
        .await
        .expect("timed out waiting for received NPDU")
        .expect("NPDU channel closed");
    assert_eq!(received.npdu.as_ref(), msg.payload.as_ref());
    assert_eq!(
        received.source_mac.as_slice(),
        &encode_bip_mac(origin.0, origin.1)
    );
    assert!(received.link_layer_group);

    let local_frame = recv_bvll(&local_broadcast_sink).await;
    assert_eq!(local_frame.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(local_frame.originating_ip, Some(origin.0));
    assert_eq!(local_frame.originating_port, Some(origin.1));
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
        local_mac: encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), local_port),
        socket: bbmd_socket,
        npdu_tx,
        bbmd: Some(Arc::new(std::sync::Mutex::new(state))),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_broadcast_port,
        client_management: Arc::default(),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        fanout: None,
        force_dbtn_forward_failure: false,
        group_sources: super::groups::GroupSources::detached(),
    };
    let msg = BvllMessage {
        function: BvlcFunction::FORWARDED_NPDU,
        payload: Bytes::from_static(&[0x01, 0x00, 0xAA, 0xBB]),
        originating_ip: Some(origin.0),
        originating_port: Some(origin.1),
    };

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

#[tokio::test]
async fn forwarded_npdu_from_directed_broadcast_peer_skips_local_rebroadcast() {
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
        local_mac: encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), local_port),
        socket: bbmd_socket,
        npdu_tx,
        bbmd: Some(Arc::new(std::sync::Mutex::new(state))),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_broadcast_port,
        client_management: Arc::default(),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        fanout: None,
        force_dbtn_forward_failure: false,
        group_sources: super::groups::GroupSources::detached(),
    };
    let msg = BvllMessage {
        function: BvlcFunction::FORWARDED_NPDU,
        payload: Bytes::from_static(&[0x01, 0x00, 0xAA, 0xBB]),
        originating_ip: Some(origin.0),
        originating_port: Some(origin.1),
    };

    handle_bvll_message(&msg, peer, Delivery::Unicast, &ctx).await;

    let fdt_frame = recv_bvll(&fdt_socket).await;
    assert_eq!(fdt_frame.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(fdt_frame.originating_ip, Some(origin.0));
    assert_eq!(fdt_frame.originating_port, Some(origin.1));
    assert_eq!(fdt_frame.payload.as_ref(), msg.payload.as_ref());

    let mut local_broadcast_buf = [0u8; 2048];
    assert!(
        timeout(
            Duration::from_millis(100),
            local_broadcast_sink.recv_from(&mut local_broadcast_buf)
        )
        .await
        .is_err(),
        "directed-broadcast Forwarded-NPDU from a peer must not be rebroadcast locally"
    );

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
        local_mac: encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), local_port),
        socket: bbmd_socket,
        npdu_tx,
        bbmd: Some(bbmd_state.clone()),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_broadcast_port,
        client_management: Arc::default(),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        fanout: None,
        force_dbtn_forward_failure: false,
        group_sources: super::groups::GroupSources::detached(),
    };
    let msg = BvllMessage {
        function: BvlcFunction::FORWARDED_NPDU,
        payload: Bytes::from_static(&[0x01, 0x00, 0xAA, 0xBB]),
        originating_ip: Some(origin.0),
        originating_port: Some(origin.1),
    };

    handle_bvll_message(&msg, peer, Delivery::Unicast, &ctx).await;

    // First two foreign devices received the frame
    let frame1 = recv_bvll(&fd_sockets[0]).await;
    assert_eq!(frame1.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(frame1.originating_ip, Some(origin.0));
    assert_eq!(frame1.originating_port, Some(origin.1));
    assert_eq!(frame1.payload.as_ref(), msg.payload.as_ref());

    let frame2 = recv_bvll(&fd_sockets[1]).await;
    assert_eq!(frame2.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(frame2.originating_ip, Some(origin.0));
    assert_eq!(frame2.originating_port, Some(origin.1));
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
        local_mac: encode_bip_mac(self_sender.0, local_port),
        socket,
        npdu_tx,
        bbmd: Some(Arc::new(std::sync::Mutex::new(state))),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_sink.local_addr().unwrap().port(),
        client_management: Arc::default(),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        fanout: None,
        force_dbtn_forward_failure: false,
        group_sources: super::groups::GroupSources::detached(),
    };
    let mut msg = BvllMessage {
        function: BvlcFunction::FORWARDED_NPDU,
        payload: Bytes::from_static(&[1, 0x80, 0x13, 0, 99, 1]),
        originating_ip: Some([192, 0, 2, 9]),
        originating_port: Some(47808),
    };
    // Remote embedded origin does not make our looped-back UDP send a peer.
    handle_bvll_message(&msg, self_sender, Delivery::Unicast, &ctx).await;
    msg.payload = Bytes::from_static(&[1, 0x80, 0x13, 0, 77, 1]);
    // Same IP, different port is an admitted peer: this fences every output
    // without a timeout-only silence assertion and rejects an IP-only filter.
    handle_bvll_message(&msg, peer_sender, Delivery::Unicast, &ctx).await;
    let received = timeout(Duration::from_secs(2), npdu_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.npdu, msg.payload);
    assert!(received.link_layer_group);
    assert_eq!(
        received.source_mac.as_slice(),
        encode_bip_mac([192, 0, 2, 9], 47808)
    );
    for sink in [&local_sink, &foreign] {
        let received = recv_bvll(sink).await;
        assert_eq!(received.function, BvlcFunction::FORWARDED_NPDU);
        assert_eq!(received.payload, msg.payload);
        assert_eq!(received.originating_ip, msg.originating_ip);
        assert_eq!(received.originating_port, msg.originating_port);
    }
}

#[test]
fn delivery_is_broadcast_only_for_the_configured_or_limited_broadcast_address() {
    use std::net::{IpAddr, Ipv6Addr};
    let configured = Ipv4Addr::new(192, 0, 2, 255);
    for (destination, os, expected) in [
        (IpAddr::V4(configured), None, Delivery::Broadcast),
        (IpAddr::V4(Ipv4Addr::BROADCAST), None, Delivery::Broadcast),
        (IpAddr::V4(configured), Some(true), Delivery::Broadcast),
        // An OS that reports unicast delivery overrides the destination.
        (IpAddr::V4(configured), Some(false), Delivery::Unicast),
        (
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
            None,
            Delivery::Unicast,
        ),
        (
            IpAddr::V4(Ipv4Addr::new(198, 51, 100, 255)),
            None,
            Delivery::Unicast,
        ),
        (IpAddr::V6(Ipv6Addr::LOCALHOST), None, Delivery::Unicast),
    ] {
        assert_eq!(
            Delivery::of(destination, configured, os),
            expected,
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
        local_mac: encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), local_port),
        socket,
        npdu_tx,
        bbmd: Some(Arc::new(std::sync::Mutex::new(state))),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_sink.local_addr().unwrap().port(),
        client_management: Arc::default(),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        fanout: None,
        force_dbtn_forward_failure: false,
        group_sources: super::groups::GroupSources::detached(),
    };
    let msg = BvllMessage {
        function: BvlcFunction::FORWARDED_NPDU,
        payload: Bytes::from_static(&[0x01, 0x00, 0xAA, 0xBB]),
        originating_ip: Some([192, 0, 2, 20]),
        originating_port: Some(0xBAC1),
    };
    let assert_forwarded = |frame: &BvllMessage, label: &str| {
        assert_eq!(frame.function, BvlcFunction::FORWARDED_NPDU, "{label}");
        assert_eq!(frame.originating_ip, msg.originating_ip, "{label}");
        assert_eq!(frame.originating_port, msg.originating_port, "{label}");
        assert_eq!(frame.payload, msg.payload, "{label}");
    };

    handle_bvll_message(&msg, peer, Delivery::Broadcast, &ctx).await;
    // Still delivered to this device and sent on to the foreign device, which
    // is sent after any local rebroadcast.
    let received = timeout(Duration::from_secs(2), npdu_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.npdu, msg.payload);
    assert_forwarded(&recv_bvll(&foreign).await, "foreign device");
    assert_no_bvll(&local_sink, "local broadcast after a broadcast arrival").await;

    handle_bvll_message(&msg, peer, Delivery::Unicast, &ctx).await;
    assert_forwarded(&recv_bvll(&local_sink).await, "local broadcast");
    assert_forwarded(&recv_bvll(&foreign).await, "foreign device");
}
