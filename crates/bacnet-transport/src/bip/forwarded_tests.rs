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
    let socket = Arc::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
    );
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
        bbmd: Some(Arc::new(Mutex::new(state))),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_broadcast_port,
        pending_bvlc_response: Arc::new(Mutex::new(None)),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        force_dbtn_forward_failure: false,
    };
    let msg = BvllMessage {
        function: BvlcFunction::FORWARDED_NPDU,
        payload: Bytes::from_static(&[0x01, 0x00, 0xAA, 0xBB]),
        originating_ip: Some(origin.0),
        originating_port: Some(origin.1),
    };

    handle_bvll_message(&msg, peer, &ctx).await;

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
    let bbmd_socket = Arc::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
    );
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
        bbmd: Some(Arc::new(Mutex::new(state))),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_broadcast_port,
        pending_bvlc_response: Arc::new(Mutex::new(None)),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        force_dbtn_forward_failure: false,
    };
    let msg = BvllMessage {
        function: BvlcFunction::FORWARDED_NPDU,
        payload: Bytes::from_static(&[0x01, 0x00, 0xAA, 0xBB]),
        originating_ip: Some(origin.0),
        originating_port: Some(origin.1),
    };

    handle_bvll_message(&msg, sender, &ctx).await;

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
    let bbmd_socket = Arc::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
    );
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
        bbmd: Some(Arc::new(Mutex::new(state))),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_broadcast_port,
        pending_bvlc_response: Arc::new(Mutex::new(None)),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        force_dbtn_forward_failure: false,
    };
    let msg = BvllMessage {
        function: BvlcFunction::FORWARDED_NPDU,
        payload: Bytes::from_static(&[0x01, 0x00, 0xAA, 0xBB]),
        originating_ip: Some(origin.0),
        originating_port: Some(origin.1),
    };

    handle_bvll_message(&msg, peer, &ctx).await;

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
    let bbmd_socket = Arc::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
    );
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

    let bbmd_state = Arc::new(Mutex::new(state));
    let ctx = RecvContext {
        local_mac: encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), local_port),
        socket: bbmd_socket,
        npdu_tx,
        bbmd: Some(bbmd_state.clone()),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_broadcast_port,
        pending_bvlc_response: Arc::new(Mutex::new(None)),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        force_dbtn_forward_failure: false,
    };
    let msg = BvllMessage {
        function: BvlcFunction::FORWARDED_NPDU,
        payload: Bytes::from_static(&[0x01, 0x00, 0xAA, 0xBB]),
        originating_ip: Some(origin.0),
        originating_port: Some(origin.1),
    };

    handle_bvll_message(&msg, peer, &ctx).await;

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
    let counters = bbmd_state.lock().await.fdt_counters();
    assert_eq!(counters.fanout_budget_reached, 1);
}
