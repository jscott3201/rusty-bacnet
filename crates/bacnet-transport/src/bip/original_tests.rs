use super::*;
use bytes::Bytes;
use tokio::time::{timeout, Duration};

#[test]
fn original_function_must_match_actual_ipv4_destination() {
    let local = Ipv4Addr::new(192, 0, 2, 10);
    let broadcast = Ipv4Addr::new(192, 0, 2, 255);

    assert!(admitted_delivery(
        BvlcFunction::ORIGINAL_UNICAST_NPDU,
        local.into(),
        local,
        broadcast,
        &[local],
        false,
        None,
    )
    .is_some());
    assert!(admitted_delivery(
        BvlcFunction::ORIGINAL_UNICAST_NPDU,
        broadcast.into(),
        local,
        broadcast,
        &[local],
        false,
        None,
    )
    .is_none());
    assert!(admitted_delivery(
        BvlcFunction::ORIGINAL_BROADCAST_NPDU,
        local.into(),
        local,
        broadcast,
        &[local],
        false,
        None,
    )
    .is_none());
    assert!(admitted_delivery(
        BvlcFunction::ORIGINAL_BROADCAST_NPDU,
        broadcast.into(),
        local,
        broadcast,
        &[local],
        false,
        None,
    )
    .is_some());

    assert!(admitted_delivery(
        BvlcFunction::ORIGINAL_UNICAST_NPDU,
        Ipv4Addr::new(192, 0, 2, 11).into(),
        local,
        broadcast,
        &[local, Ipv4Addr::new(192, 0, 2, 11)],
        true,
        None,
    )
    .is_some());
    assert!(admitted_delivery(
        BvlcFunction::ORIGINAL_UNICAST_NPDU,
        broadcast.into(),
        local,
        broadcast,
        &[local],
        true,
        Some(true),
    )
    .is_none());
    // A wildcard bind takes unicast only to a listed address, on every OS
    // (#952). Windows flagging the datagram as unicast delivery is not
    // enough on its own any more.
    for (destination, accepted) in [(local, true), (Ipv4Addr::new(192, 0, 2, 12), false)] {
        assert_eq!(
            admitted_delivery(
                BvlcFunction::ORIGINAL_UNICAST_NPDU,
                destination.into(),
                local,
                broadcast,
                &[local],
                true,
                Some(false),
            )
            .is_some(),
            accepted,
            "{destination}"
        );
    }

    for function in [
        BvlcFunction::BVLC_RESULT,
        BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
        BvlcFunction::REGISTER_FOREIGN_DEVICE,
        BvlcFunction::READ_FOREIGN_DEVICE_TABLE,
        BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
        BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
        BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
    ] {
        assert!(admitted_delivery(
            function,
            local.into(),
            local,
            broadcast,
            &[local],
            false,
            None,
        )
        .is_some());
        assert!(admitted_delivery(
            function,
            broadcast.into(),
            local,
            broadcast,
            &[local],
            false,
            Some(true),
        )
        .is_none());
    }

    assert!(admitted_delivery(
        BvlcFunction::FORWARDED_NPDU,
        local.into(),
        local,
        broadcast,
        &[local],
        false,
        None,
    )
    .is_some());
    assert!(admitted_delivery(
        BvlcFunction::FORWARDED_NPDU,
        broadcast.into(),
        local,
        broadcast,
        &[local],
        false,
        Some(true),
    )
    .is_some());
}

async fn recv_bvll(socket: &UdpSocket) -> BvllMessage {
    let mut recv_buf = [0u8; 2048];
    let (len, _addr) = timeout(Duration::from_secs(2), socket.recv_from(&mut recv_buf))
        .await
        .expect("timed out waiting for BVLL frame")
        .unwrap();
    decode_bvll(&recv_buf[..len]).unwrap()
}

#[tokio::test]
async fn original_unicast_npdu_uses_udp_sender_source_mac_and_ignores_self() {
    let socket = Arc::new(super::BipSocket::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
        None,
    ));
    let local_port = socket.local_addr().unwrap().port();
    let (npdu_tx, mut npdu_rx) = mpsc::channel(1);
    let ctx = RecvContext {
        local_mac: encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), local_port),
        socket,
        npdu_tx,
        bbmd: None,
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_port,
        client_management: Arc::default(),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        fanout: None,
        force_dbtn_forward_failure: false,
        group_sources: super::groups::GroupSources::detached(),
    };
    let sender = ([192, 0, 2, 30], 0xBAC0);
    let msg = BvllMessage {
        function: BvlcFunction::ORIGINAL_UNICAST_NPDU,
        payload: Bytes::from_static(&[0x01, 0x04, 0xAA, 0xBB]),
        originating_ip: None,
        originating_port: None,
    };

    handle_bvll_message(&msg, sender, Delivery::Unicast, &ctx).await;

    let received = timeout(Duration::from_secs(2), npdu_rx.recv())
        .await
        .expect("timed out waiting for received NPDU")
        .expect("NPDU channel closed");
    assert_eq!(received.npdu.as_ref(), msg.payload.as_ref());
    assert_eq!(
        received.source_mac.as_slice(),
        &encode_bip_mac(sender.0, sender.1)
    );
    assert!(!received.link_layer_group);

    handle_bvll_message(
        &msg,
        (Ipv4Addr::LOCALHOST.octets(), local_port),
        Delivery::Unicast,
        &ctx,
    )
    .await;
    assert!(
        timeout(Duration::from_millis(100), npdu_rx.recv())
            .await
            .is_err(),
        "Original-Unicast-NPDU from this transport's B/IP MAC must be ignored"
    );
}

#[tokio::test]
async fn original_broadcast_npdu_bbmd_forwards_to_bdt_and_fdt_without_local_echo() {
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
    let sender = ([192, 0, 2, 40], 0xBAC1);
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
        function: BvlcFunction::ORIGINAL_BROADCAST_NPDU,
        payload: Bytes::from_static(&[0x01, 0x20, 0xDE, 0xAD, 0xBE, 0xEF]),
        originating_ip: None,
        originating_port: None,
    };

    handle_bvll_message(&msg, sender, Delivery::Unicast, &ctx).await;

    let received = timeout(Duration::from_secs(2), npdu_rx.recv())
        .await
        .expect("timed out waiting for received NPDU")
        .expect("NPDU channel closed");
    assert_eq!(received.npdu.as_ref(), msg.payload.as_ref());
    assert_eq!(
        received.source_mac.as_slice(),
        &encode_bip_mac(sender.0, sender.1)
    );
    assert!(received.link_layer_group);

    for (label, socket) in [
        ("BDT peer", &bdt_peer_sink),
        ("foreign device", &fdt_socket),
    ] {
        let frame = recv_bvll(socket).await;
        assert_eq!(frame.function, BvlcFunction::FORWARDED_NPDU, "{label}");
        assert_eq!(frame.originating_ip, Some(sender.0), "{label}");
        assert_eq!(frame.originating_port, Some(sender.1), "{label}");
        assert_eq!(frame.payload.as_ref(), msg.payload.as_ref(), "{label}");
    }

    let mut local_broadcast_buf = [0u8; 2048];
    assert!(
        timeout(
            Duration::from_millis(100),
            local_broadcast_sink.recv_from(&mut local_broadcast_buf)
        )
        .await
        .is_err(),
        "Original-Broadcast-NPDU was already visible on the local subnet and must not be echoed locally as Forwarded-NPDU"
    );
}

/// The fanout arm for a received Original-Broadcast-NPDU, at handler level so
/// it runs on every OS: a loopback datagram can't arrive as a broadcast on
/// Windows (#950).
#[tokio::test]
async fn original_broadcast_npdu_bbmd_fanout_deduplicates_and_throttles() {
    async fn sink() -> (UdpSocket, u16) {
        let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = socket.local_addr().unwrap().port();
        (socket, port)
    }
    async fn assert_silent(socket: &UdpSocket, label: &str) {
        let mut buf = [0u8; 2048];
        assert!(
            timeout(Duration::from_millis(100), socket.recv_from(&mut buf))
                .await
                .is_err(),
            "{label} received an unexpected frame"
        );
    }

    let bbmd_socket = Arc::new(super::BipSocket::new(sink().await.0, None));
    let local_port = bbmd_socket.local_addr().unwrap().port();
    let (local_broadcast_sink, local_broadcast_port) = sink().await;
    // BDT peers A and B; foreign devices A (again) and C.
    let (peer_a, port_a) = sink().await;
    let (peer_b, port_b) = sink().await;
    let (device_c, port_c) = sink().await;
    let (npdu_tx, mut npdu_rx) = mpsc::channel(1);
    let sender = ([192, 0, 2, 40], 0xBAC1);
    let mut state = BbmdState::new(Ipv4Addr::LOCALHOST.octets(), local_port);
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    let peer = |port| BdtEntry {
        ip: Ipv4Addr::LOCALHOST.octets(),
        port,
        broadcast_mask: [255, 255, 255, 255],
    };
    state.set_bdt(vec![peer(port_a), peer(port_b)]).unwrap();
    for port in [port_a, port_c] {
        assert_eq!(
            state.register_foreign_device(Ipv4Addr::LOCALHOST.octets(), port, 60),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }

    // Three distinct targets (A, B, C) after deduplication, two admitted.
    let policy = fanout::FanoutPolicy {
        max_fanout_per_input: 2,
        ..fanout::FanoutPolicy::default()
    };
    let (fanout_tx, fanout_rx) = mpsc::channel(policy.queue_capacity);
    let counters = Arc::new(fanout::AtomicFanoutCounters::default());
    let limiter = Arc::new(std::sync::Mutex::new(fanout::FanoutRateLimiter::new(
        policy,
    )));
    let _worker = tokio::spawn(fanout::run_fanout_worker(
        Arc::clone(&bbmd_socket),
        fanout_rx,
        Arc::clone(&counters),
    ));
    let ctx = RecvContext {
        local_mac: encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), local_port),
        socket: bbmd_socket,
        npdu_tx,
        bbmd: Some(Arc::new(std::sync::Mutex::new(state))),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_broadcast_port,
        client_management: Arc::default(),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        fanout: Some(fanout::FanoutDispatcher::new(
            fanout_tx,
            limiter,
            Arc::clone(&counters),
        )),
        force_dbtn_forward_failure: false,
        group_sources: super::groups::GroupSources::detached(),
    };
    let msg = BvllMessage {
        function: BvlcFunction::ORIGINAL_BROADCAST_NPDU,
        payload: Bytes::from_static(&[0x01, 0x20, 0xFE, 0xED]),
        originating_ip: None,
        originating_port: None,
    };

    handle_bvll_message(&msg, sender, Delivery::Broadcast, &ctx).await;

    let received = timeout(Duration::from_secs(2), npdu_rx.recv())
        .await
        .expect("timed out waiting for received NPDU")
        .expect("NPDU channel closed");
    assert_eq!(received.npdu.as_ref(), msg.payload.as_ref());
    assert!(received.link_layer_group);
    // BDT peers come first, so A and B are admitted, each exactly once.
    for (label, socket) in [("BDT peer A", &peer_a), ("BDT peer B", &peer_b)] {
        let frame = recv_bvll(socket).await;
        assert_eq!(frame.function, BvlcFunction::FORWARDED_NPDU, "{label}");
        assert_eq!(frame.originating_ip, Some(sender.0), "{label}");
        assert_eq!(frame.originating_port, Some(sender.1), "{label}");
        assert_eq!(frame.payload.as_ref(), msg.payload.as_ref(), "{label}");
        assert_silent(socket, label).await;
    }
    assert_silent(&device_c, "throttled foreign device C").await;
    assert_silent(&local_broadcast_sink, "local subnet").await;

    let counters = counters.snapshot();
    assert_eq!(counters.destinations_deduplicated, 1);
    assert_eq!(counters.packets_forwarded, 2);
    assert_eq!(counters.packets_throttled, 1);
    assert_eq!(counters.queue_overflow_drops, 0);
    assert_eq!(counters.send_errors, 0);
}
