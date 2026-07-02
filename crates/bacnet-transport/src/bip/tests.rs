use super::*;
use bytes::{Bytes, BytesMut};
use tokio::time::{timeout, Duration};

fn test_bvll_message(function: BvlcFunction, payload: &[u8]) -> BvllMessage {
    BvllMessage {
        function,
        payload: Bytes::copy_from_slice(payload),
        originating_ip: None,
        originating_port: None,
    }
}

async fn raw_bvlc_request(target: &[u8], function: BvlcFunction, payload: &[u8]) -> BvllMessage {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let (ip, port) = decode_bip_mac(target).unwrap();
    let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);

    let mut buf = BytesMut::new();
    encode_bvll(&mut buf, function, payload).unwrap();
    socket.send_to(&buf, dest).await.unwrap();

    let mut recv_buf = [0u8; 2048];
    let (len, _addr) = timeout(Duration::from_secs(2), socket.recv_from(&mut recv_buf))
        .await
        .expect("timed out waiting for BVLC response")
        .unwrap();
    decode_bvll(&recv_buf[..len]).unwrap()
}

#[test]
fn bip_max_apdu_length() {
    let transport = BipTransport::new(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        std::net::Ipv4Addr::LOCALHOST,
    );
    assert_eq!(transport.max_apdu_length(), 1476);
}

#[test]
fn decode_bvlc_result_code_accepts_named_and_unknown_codes() {
    let named = test_bvll_message(BvlcFunction::BVLC_RESULT, &[0x00, 0x30]);
    assert_eq!(
        decode_bvlc_result_code(&named).unwrap(),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );

    let unknown = test_bvll_message(BvlcFunction::BVLC_RESULT, &[0x12, 0x34]);
    assert_eq!(decode_bvlc_result_code(&unknown).unwrap().to_raw(), 0x1234);
}

#[test]
fn decode_bvlc_result_code_rejects_wrong_function() {
    let msg = test_bvll_message(BvlcFunction::ORIGINAL_UNICAST_NPDU, &[0x00, 0x00]);
    let err = decode_bvlc_result_code(&msg).unwrap_err();

    assert!(
        format!("{err}").contains("expected BVLC response BvlcFunction::BVLC_RESULT"),
        "unexpected error: {err}"
    );
}

#[test]
fn decode_bvlc_result_code_rejects_malformed_payload_lengths() {
    for payload in [&[][..], &[0x00][..], &[0x00, 0x00, 0x00][..]] {
        let msg = test_bvll_message(BvlcFunction::BVLC_RESULT, payload);
        let err = decode_bvlc_result_code(&msg).unwrap_err();

        assert!(
            format!("{err}").contains("BVLC-Result payload must be 2 bytes"),
            "unexpected error: {err}"
        );
    }
}

#[test]
fn expect_bvlc_function_rejects_unexpected_response_function() {
    let msg = test_bvll_message(BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK, &[]);
    let err = expect_bvlc_function(&msg, BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK)
        .unwrap_err();

    assert!(
        format!("{err}")
            .contains("expected BVLC response BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn pending_bvlc_response_requires_sender_and_expected_function() {
    let socket = Arc::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
    );
    let (npdu_tx, _npdu_rx) = mpsc::channel(1);
    let pending_bvlc_response = Arc::new(Mutex::new(None));
    let (tx, mut rx) = oneshot::channel();

    {
        let mut slot = pending_bvlc_response.lock().await;
        *slot = Some(PendingBvlcResponse {
            target: ([127, 0, 0, 1], 47808),
            expected: BvlcResponseKind::ReadBroadcastDistributionTableAck,
            tx,
        });
    }

    let ctx = RecvContext {
        local_mac: [0; 6],
        socket,
        npdu_tx,
        bbmd: None,
        broadcast_addr: Ipv4Addr::BROADCAST,
        broadcast_port: 47808,
        pending_bvlc_response: pending_bvlc_response.clone(),
        bdt_persist_path: None,
        force_dbtn_forward_failure: false,
    };

    let result = test_bvll_message(BvlcFunction::BVLC_RESULT, &[0x00, 0x00]);
    handle_bvll_message(&result, ([127, 0, 0, 2], 47808), &ctx).await;
    assert!(pending_bvlc_response.lock().await.is_some());
    assert!(rx.try_recv().is_err());

    let wrong_ack = test_bvll_message(BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK, &[]);
    handle_bvll_message(&wrong_ack, ([127, 0, 0, 1], 47808), &ctx).await;
    assert!(pending_bvlc_response.lock().await.is_some());
    assert!(rx.try_recv().is_err());

    let expected_ack = test_bvll_message(BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK, &[]);
    handle_bvll_message(&expected_ack, ([127, 0, 0, 1], 47808), &ctx).await;
    assert!(pending_bvlc_response.lock().await.is_none());
    assert_eq!(
        rx.await.unwrap().function,
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK
    );
}

#[tokio::test]
async fn start_stop() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    assert!(transport.socket.is_some());
    assert!(!transport.local_mac().iter().all(|b| *b == 0));
    transport.stop().await.unwrap();
    assert!(transport.socket.is_none());
}

#[tokio::test]
async fn unicast_loopback() {
    // Two transports on localhost with ephemeral ports
    let mut transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

    let _rx_a = transport_a.start().await.unwrap();
    let mut rx_b = transport_b.start().await.unwrap();

    let test_npdu = vec![0x01, 0x00, 0xDE, 0xAD, 0xBE, 0xEF];

    // A sends unicast to B
    transport_a
        .send_unicast(&test_npdu, transport_b.local_mac())
        .await
        .unwrap();

    // B should receive it
    let received = timeout(Duration::from_secs(2), rx_b.recv())
        .await
        .expect("Timed out waiting for packet")
        .expect("Channel closed");

    assert_eq!(received.npdu, test_npdu);
    assert_eq!(received.source_mac.as_slice(), transport_a.local_mac());

    transport_a.stop().await.unwrap();
    transport_b.stop().await.unwrap();
}

#[tokio::test]
async fn bbmd_register_foreign_device() {
    // Start a BBMD
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

    // Start a foreign device that registers with the BBMD
    let mut fd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    fd_transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::from(bbmd_ip),
        bbmd_port,
        ttl: 60,
    });
    let _fd_rx = fd_transport.start().await.unwrap();

    // Give a moment for the registration to be processed
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify the BBMD has the foreign device in its FDT
    {
        let bbmd_state = bbmd_transport.bbmd_state().unwrap();
        let mut state = bbmd_state.lock().await;
        let fdt = state.fdt();
        assert_eq!(fdt.len(), 1);
        assert_eq!(fdt[0].ttl, 60);
    }

    fd_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn read_bdt_from_bbmd() {
    // Start a BBMD with a known BDT
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let initial_bdt = vec![BdtEntry {
        ip: [10, 0, 0, 1],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    }];
    bbmd_transport.enable_bbmd(initial_bdt.clone());
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    // Start a second transport (client) to query the BBMD
    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    // Read the BDT — includes the configured entry plus the auto-inserted self entry
    let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert_eq!(bdt.len(), 2);
    assert!(bdt
        .iter()
        .any(|e| e.ip == [10, 0, 0, 1] && e.port == 0xBAC0));
    // Self entry is also present (auto-inserted by set_bdt)
    assert!(bdt.len() >= 2);

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn read_fdt_from_bbmd() {
    // Start a BBMD
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

    // Register a foreign device
    let mut fd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    fd_transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::from(bbmd_ip),
        bbmd_port,
        ttl: 120,
    });
    let _fd_rx = fd_transport.start().await.unwrap();
    let fd_mac = fd_transport.local_mac().to_vec();
    let (fd_ip, fd_port) = decode_bip_mac(&fd_mac).unwrap();

    // Wait for registration to be processed
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Start a third transport to query the FDT
    let mut query_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _query_rx = query_transport.start().await.unwrap();

    let fdt = query_transport.read_fdt(&bbmd_mac).await.unwrap();
    assert_eq!(fdt.len(), 1);
    assert_eq!(fdt[0].ip, fd_ip);
    assert_eq!(fdt[0].port, fd_port);
    assert_eq!(fdt[0].ttl, 120);
    assert!(fdt[0].seconds_remaining <= 150);

    query_transport.stop().await.unwrap();
    fd_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn read_bdt_from_non_bbmd_surfaces_typed_nak() {
    let mut server_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _server_rx = server_transport.start().await.unwrap();
    let server_mac = server_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let err = client_transport.read_bdt(&server_mac).await.unwrap_err();
    assert!(
        format!("{err}").contains("READ_BROADCAST_DISTRIBUTION_TABLE_NAK"),
        "unexpected error: {err}"
    );

    client_transport.stop().await.unwrap();
    server_transport.stop().await.unwrap();
}

#[tokio::test]
async fn read_fdt_from_non_bbmd_surfaces_typed_nak() {
    let mut server_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _server_rx = server_transport.start().await.unwrap();
    let server_mac = server_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let err = client_transport.read_fdt(&server_mac).await.unwrap_err();
    assert!(
        format!("{err}").contains("READ_FOREIGN_DEVICE_TABLE_NAK"),
        "unexpected error: {err}"
    );

    client_transport.stop().await.unwrap();
    server_transport.stop().await.unwrap();
}

#[tokio::test]
async fn write_bdt_to_bbmd() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let old_bdt_entry = BdtEntry {
        ip: [10, 0, 0, 1],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };
    bbmd_transport.enable_bbmd(vec![old_bdt_entry.clone()]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let new_bdt = vec![BdtEntry {
        ip: [192, 168, 1, 1],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 255],
    }];
    let result = client_transport
        .write_bdt(&bbmd_mac, &new_bdt)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);

    // Verify by reading back — includes written entry plus auto-inserted self
    let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert!(bdt
        .iter()
        .any(|e| e.ip == [192, 168, 1, 1] && e.port == 0xBAC0));
    assert!(
        !bdt.iter().any(|e| e == &old_bdt_entry),
        "Write-BDT must replace the prior configured BDT entries"
    );

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn write_bdt_rejects_malformed_payload_and_preserves_table() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let initial_entry = BdtEntry {
        ip: [10, 0, 0, 1],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };
    bbmd_transport.enable_bbmd(vec![initial_entry.clone()]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let response = raw_bvlc_request(
        &bbmd_mac,
        BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
        &[0; bbmd::BDT_ENTRY_SIZE - 1],
    )
    .await;
    assert_eq!(
        decode_bvlc_result_code(&response).unwrap(),
        BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK
    );

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();
    let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert!(
        bdt.iter().any(|e| e == &initial_entry),
        "malformed Write-BDT must leave the existing BDT intact"
    );

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn write_bdt_to_non_bbmd_surfaces_typed_nak() {
    let mut server_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _server_rx = server_transport.start().await.unwrap();
    let server_mac = server_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let result = client_transport.write_bdt(&server_mac, &[]).await.unwrap();
    assert_eq!(
        result,
        BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK
    );

    client_transport.stop().await.unwrap();
    server_transport.stop().await.unwrap();
}

#[tokio::test]
async fn register_foreign_device_via_bvlc() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let result = client_transport
        .register_foreign_device_bvlc(&bbmd_mac, 60)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn register_foreign_device_rejects_zero_ttl() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let result = client_transport
        .register_foreign_device_bvlc(&bbmd_mac, 0)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK);

    {
        let state = bbmd_transport.bbmd_state().unwrap();
        let mut state = state.lock().await;
        assert!(state.fdt().is_empty());
    }

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn register_foreign_device_rejects_malformed_ttl_payload_lengths() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    for payload in [&[0x00][..], &[0x00, 0x3c, 0x00][..]] {
        let response =
            raw_bvlc_request(&bbmd_mac, BvlcFunction::REGISTER_FOREIGN_DEVICE, payload).await;
        assert_eq!(
            decode_bvlc_result_code(&response).unwrap(),
            BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
        );
    }

    {
        let state = bbmd_transport.bbmd_state().unwrap();
        let mut state = state.lock().await;
        assert!(state.fdt().is_empty());
    }

    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn register_foreign_device_accepts_max_ttl_and_caps_remaining() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let result = client_transport
        .register_foreign_device_bvlc(&bbmd_mac, u16::MAX)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);

    let fdt = client_transport.read_fdt(&bbmd_mac).await.unwrap();
    assert_eq!(fdt.len(), 1);
    assert_eq!(fdt[0].ttl, u16::MAX);
    assert_eq!(fdt[0].seconds_remaining, u16::MAX);

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn register_foreign_device_to_non_bbmd_surfaces_typed_nak() {
    let mut server_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _server_rx = server_transport.start().await.unwrap();
    let server_mac = server_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let result = client_transport
        .register_foreign_device_bvlc(&server_mac, 60)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK);

    client_transport.stop().await.unwrap();
    server_transport.stop().await.unwrap();
}

#[tokio::test]
async fn delete_fdt_entry_to_non_bbmd_surfaces_typed_nak() {
    let mut server_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _server_rx = server_transport.start().await.unwrap();
    let server_mac = server_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let result = client_transport
        .delete_fdt_entry(&server_mac, [127, 0, 0, 1], 47808)
        .await
        .unwrap();
    assert_eq!(
        result,
        BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
    );

    client_transport.stop().await.unwrap();
    server_transport.stop().await.unwrap();
}

#[tokio::test]
async fn delete_fdt_entry_removes_registered_foreign_device() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let result = client_transport
        .register_foreign_device_bvlc(&bbmd_mac, 60)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);

    let (fd_ip, fd_port) = decode_bip_mac(client_transport.local_mac()).unwrap();
    let result = client_transport
        .delete_fdt_entry(&bbmd_mac, fd_ip, fd_port)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);

    let fdt = client_transport.read_fdt(&bbmd_mac).await.unwrap();
    assert!(fdt.is_empty());

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn delete_fdt_entry_rejects_malformed_payload_and_preserves_entry() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let result = client_transport
        .register_foreign_device_bvlc(&bbmd_mac, 60)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);

    let (fd_ip, fd_port) = decode_bip_mac(client_transport.local_mac()).unwrap();
    let mut payload_with_trailing_byte = Vec::from(fd_ip);
    payload_with_trailing_byte.extend_from_slice(&fd_port.to_be_bytes());
    payload_with_trailing_byte.push(0);

    for payload in [&[0; 5][..], payload_with_trailing_byte.as_slice()] {
        let response = raw_bvlc_request(
            &bbmd_mac,
            BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
            payload,
        )
        .await;
        assert_eq!(
            decode_bvlc_result_code(&response).unwrap(),
            BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
        );
    }

    let fdt = client_transport.read_fdt(&bbmd_mac).await.unwrap();
    assert_eq!(fdt.len(), 1);
    assert_eq!(fdt[0].ip, fd_ip);
    assert_eq!(fdt[0].port, fd_port);

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn foreign_device_broadcast_via_bbmd() {
    // BBMD
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let mut bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

    // Foreign device
    let mut fd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    fd_transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::from(bbmd_ip),
        bbmd_port,
        ttl: 60,
    });
    let _fd_rx = fd_transport.start().await.unwrap();

    // Give time for registration
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Foreign device sends a broadcast (should use Distribute-Broadcast-To-Network)
    let test_npdu = vec![0x01, 0x00, 0xAA, 0xBB];
    fd_transport.send_broadcast(&test_npdu).await.unwrap();

    // BBMD should receive it (as NPDU via Distribute-Broadcast-To-Network)
    let received = timeout(Duration::from_secs(2), bbmd_rx.recv())
        .await
        .expect("BBMD timed out")
        .expect("BBMD channel closed");

    assert_eq!(received.npdu, test_npdu);

    fd_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn distribute_broadcast_from_unregistered_foreign_device_naks_without_delivery() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let mut bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let response = raw_bvlc_request(
        &bbmd_mac,
        BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
        &[0x01, 0x00, 0xAA, 0xBB],
    )
    .await;
    assert_eq!(
        decode_bvlc_result_code(&response).unwrap(),
        BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK
    );

    assert!(
        timeout(Duration::from_millis(100), bbmd_rx.recv())
            .await
            .is_err(),
        "unregistered DBTN must not deliver an NPDU to the BBMD"
    );

    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn bbmd_management_acl_preserved_after_start() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.enable_bbmd(vec![]);
    transport.set_bbmd_management_acl(vec![[10, 0, 0, 1]]);
    let _rx = transport.start().await.unwrap();

    {
        let state = transport.bbmd_state().unwrap();
        let s = state.lock().await;
        assert!(s.is_management_allowed(&[10, 0, 0, 1]));
        assert!(!s.is_management_allowed(&[10, 0, 0, 2]));
    }

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bvlc_request_rejects_concurrent_calls() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();

    // Manually install a pending sender to simulate an in-flight request
    {
        let (tx, _rx) = oneshot::channel();
        let (ip, port) = decode_bip_mac(transport.local_mac()).unwrap();
        let mut slot = transport.pending_bvlc_response.lock().await;
        *slot = Some(PendingBvlcResponse {
            target: (ip, port),
            expected: BvlcResponseKind::ReadBroadcastDistributionTableAck,
            tx,
        });
    }

    // A second request should fail immediately
    let fake_target = transport.local_mac().to_vec();
    let result = transport.read_bdt(&fake_target).await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("already in flight"),
        "expected 'already in flight' error, got: {err}"
    );

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn socket_is_broadcast_capable_and_binds_inaddr_any() {
    // Regression for the "user-supplied interface IP" silently rejecting
    // broadcast traffic.  Even when the caller passes a specific interface,
    // the underlying socket must bind 0.0.0.0 so the kernel delivers
    // subnet- and limited-broadcast packets to it.  The interface IP is
    // still used for the announced local MAC.
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();

    let local = transport
        .socket
        .as_ref()
        .expect("socket exists after start")
        .local_addr()
        .expect("local_addr is queryable");

    assert!(
        local.ip().is_unspecified(),
        "BIP socket must bind to 0.0.0.0 for broadcast reception; got {local}"
    );
    assert!(
        socket2::SockRef::from(
            transport
                .socket
                .as_ref()
                .expect("socket exists after start")
                .as_ref()
        )
        .broadcast()
        .expect("SO_BROADCAST is queryable"),
        "BIP socket must enable SO_BROADCAST for Original-Broadcast-NPDU sends"
    );

    // The announced local MAC must still reflect the user-supplied interface,
    // not the bind address.
    let mac = transport.local_mac();
    assert_eq!(
        &mac[..4],
        &Ipv4Addr::LOCALHOST.octets(),
        "announced IP must match interface"
    );

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn start_fails_on_nonlocal_interface() {
    // Now that we bind the real socket to 0.0.0.0, a typo'd interface IP
    // would otherwise succeed at bind and only fail silently later when
    // peers reply to an address we don't own.  start() must instead probe
    // the configured interface and fail fast.  192.0.2.0/24 (RFC 5737
    // TEST-NET-1) is reserved and never assignable to a real NIC, so the
    // probe must reject it.
    let mut transport = BipTransport::new(Ipv4Addr::new(192, 0, 2, 1), 0, Ipv4Addr::BROADCAST);
    let err = transport
        .start()
        .await
        .expect_err("start() must reject a non-local interface IP");
    assert!(
        matches!(err, Error::Transport(_)),
        "expected Error::Transport, got: {err:?}"
    );
}
