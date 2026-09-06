use super::*;
use bytes::Bytes;
use tokio::time::{timeout, Duration};

#[test]
fn original_function_must_match_actual_ipv4_destination() {
    let local = Ipv4Addr::new(192, 0, 2, 10);
    let broadcast = Ipv4Addr::new(192, 0, 2, 255);

    assert!(original_destination_matches(
        BvlcFunction::ORIGINAL_UNICAST_NPDU,
        local.into(),
        local,
        broadcast,
        &[local],
        false,
        None,
    ));
    assert!(!original_destination_matches(
        BvlcFunction::ORIGINAL_UNICAST_NPDU,
        broadcast.into(),
        local,
        broadcast,
        &[local],
        false,
        None,
    ));
    assert!(!original_destination_matches(
        BvlcFunction::ORIGINAL_BROADCAST_NPDU,
        local.into(),
        local,
        broadcast,
        &[local],
        false,
        None,
    ));
    assert!(original_destination_matches(
        BvlcFunction::ORIGINAL_BROADCAST_NPDU,
        broadcast.into(),
        local,
        broadcast,
        &[local],
        false,
        None,
    ));

    assert!(original_destination_matches(
        BvlcFunction::ORIGINAL_UNICAST_NPDU,
        Ipv4Addr::new(192, 0, 2, 11).into(),
        local,
        broadcast,
        &[local, Ipv4Addr::new(192, 0, 2, 11)],
        true,
        None,
    ));
    assert!(!original_destination_matches(
        BvlcFunction::ORIGINAL_UNICAST_NPDU,
        broadcast.into(),
        local,
        broadcast,
        &[local],
        true,
        Some(true),
    ));

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
        assert!(original_destination_matches(
            function,
            local.into(),
            local,
            broadcast,
            &[local],
            false,
            None,
        ));
        assert!(!original_destination_matches(
            function,
            broadcast.into(),
            local,
            broadcast,
            &[local],
            false,
            Some(true),
        ));
    }

    assert!(original_destination_matches(
        BvlcFunction::FORWARDED_NPDU,
        local.into(),
        local,
        broadcast,
        &[local],
        false,
        None,
    ));
    assert!(original_destination_matches(
        BvlcFunction::FORWARDED_NPDU,
        broadcast.into(),
        local,
        broadcast,
        &[local],
        false,
        Some(true),
    ));
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
    let socket = Arc::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
    );
    let local_port = socket.local_addr().unwrap().port();
    let (npdu_tx, mut npdu_rx) = mpsc::channel(1);
    let ctx = RecvContext {
        local_mac: encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), local_port),
        socket,
        npdu_tx,
        bbmd: None,
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_port,
        pending_bvlc_response: Arc::new(Mutex::new(None)),
        bdt_persist_path: None,
        force_dbtn_forward_failure: false,
    };
    let sender = ([192, 0, 2, 30], 0xBAC0);
    let msg = BvllMessage {
        function: BvlcFunction::ORIGINAL_UNICAST_NPDU,
        payload: Bytes::from_static(&[0x01, 0x04, 0xAA, 0xBB]),
        originating_ip: None,
        originating_port: None,
    };

    handle_bvll_message(&msg, sender, &ctx).await;

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

    handle_bvll_message(&msg, (Ipv4Addr::LOCALHOST.octets(), local_port), &ctx).await;
    assert!(
        timeout(Duration::from_millis(100), npdu_rx.recv())
            .await
            .is_err(),
        "Original-Unicast-NPDU from this transport's B/IP MAC must be ignored"
    );
}

#[tokio::test]
async fn original_broadcast_npdu_bbmd_forwards_to_bdt_and_fdt_without_local_echo() {
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
    let sender = ([192, 0, 2, 40], 0xBAC1);
    let mut state = BbmdState::new(Ipv4Addr::LOCALHOST.octets(), local_port);
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
        bdt_persist_path: None,
        force_dbtn_forward_failure: false,
    };
    let msg = BvllMessage {
        function: BvlcFunction::ORIGINAL_BROADCAST_NPDU,
        payload: Bytes::from_static(&[0x01, 0x20, 0xDE, 0xAD, 0xBE, 0xEF]),
        originating_ip: None,
        originating_port: None,
    };

    handle_bvll_message(&msg, sender, &ctx).await;

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
