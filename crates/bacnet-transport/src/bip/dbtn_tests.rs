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
        "{label} received an unexpected extra BVLL frame"
    );
}

#[tokio::test]
async fn dbtn_registered_foreign_device_fans_out_without_origin_echo() {
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
    let origin_fd_sink = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let origin_fd_port = origin_fd_sink.local_addr().unwrap().port();
    let peer_fd_sink = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let peer_fd_port = peer_fd_sink.local_addr().unwrap().port();
    let (npdu_tx, mut npdu_rx) = mpsc::channel(1);

    let mut state = BbmdState::new(Ipv4Addr::LOCALHOST.octets(), local_port);
    state
        .set_bdt(vec![BdtEntry {
            ip: Ipv4Addr::LOCALHOST.octets(),
            port: bdt_peer_port,
            broadcast_mask: [255, 255, 255, 255],
        }])
        .unwrap();
    assert_eq!(
        state.register_foreign_device(Ipv4Addr::LOCALHOST.octets(), origin_fd_port, 60),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    assert_eq!(
        state.register_foreign_device(Ipv4Addr::LOCALHOST.octets(), peer_fd_port, 60),
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
        bdt_persist_path: None,
        force_dbtn_forward_failure: false,
    };
    let sender = (Ipv4Addr::LOCALHOST.octets(), origin_fd_port);
    let msg = BvllMessage {
        function: BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
        payload: Bytes::from_static(&[0x01, 0x20, 0xBA, 0xC0, 0xDB, 0x10]),
        originating_ip: None,
        originating_port: None,
    };

    handle_bvll_message(&msg, sender, &ctx).await;

    let received = timeout(Duration::from_secs(2), npdu_rx.recv())
        .await
        .expect("timed out waiting for received DBTN NPDU")
        .expect("NPDU channel closed");
    assert_eq!(received.npdu.as_ref(), msg.payload.as_ref());
    assert_eq!(
        received.source_mac.as_slice(),
        &encode_bip_mac(sender.0, sender.1)
    );
    assert!(received.link_layer_group);

    for (label, socket) in [
        ("local broadcast", &local_broadcast_sink),
        ("BDT peer", &bdt_peer_sink),
        ("foreign device peer", &peer_fd_sink),
    ] {
        let frame = recv_bvll(socket).await;
        assert_eq!(frame.function, BvlcFunction::FORWARDED_NPDU, "{label}");
        assert_eq!(frame.originating_ip, Some(sender.0), "{label}");
        assert_eq!(frame.originating_port, Some(sender.1), "{label}");
        assert_eq!(frame.payload.as_ref(), msg.payload.as_ref(), "{label}");
    }

    for (label, socket) in [
        ("local broadcast", &local_broadcast_sink),
        ("BDT peer", &bdt_peer_sink),
        ("foreign device peer", &peer_fd_sink),
    ] {
        assert_no_bvll(socket, label).await;
    }
    assert_no_bvll(&origin_fd_sink, "originating foreign device").await;
}

#[tokio::test]
async fn dbtn_registered_foreign_device_naks_when_forwarding_fails() {
    let bbmd_socket = Arc::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
    );
    let local_port = bbmd_socket.local_addr().unwrap().port();
    let origin_fd_sink = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let origin_fd_port = origin_fd_sink.local_addr().unwrap().port();
    let (npdu_tx, _npdu_rx) = mpsc::channel(1);

    let mut state = BbmdState::new(Ipv4Addr::LOCALHOST.octets(), local_port);
    assert_eq!(
        state.register_foreign_device(Ipv4Addr::LOCALHOST.octets(), origin_fd_port, 60),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );

    let ctx = RecvContext {
        local_mac: encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), local_port),
        socket: bbmd_socket,
        npdu_tx,
        bbmd: Some(Arc::new(Mutex::new(state))),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_port,
        pending_bvlc_response: Arc::new(Mutex::new(None)),
        bdt_persist_path: None,
        force_dbtn_forward_failure: true,
    };
    let sender = (Ipv4Addr::LOCALHOST.octets(), origin_fd_port);
    let msg = BvllMessage {
        function: BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
        payload: Bytes::from_static(&[0x01, 0x20, 0xBA, 0xC0, 0x00, 0x60]),
        originating_ip: None,
        originating_port: None,
    };

    handle_bvll_message(&msg, sender, &ctx).await;

    let result = recv_bvll(&origin_fd_sink).await;
    assert_eq!(result.function, BvlcFunction::BVLC_RESULT);
    assert_eq!(
        decode_bvlc_result_code(&result).unwrap(),
        BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK
    );
}
