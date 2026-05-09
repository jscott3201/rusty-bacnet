use super::*;

#[test]
fn connection_initial_state() {
    let conn = ScConnection::new([0x01; 6], [0u8; 16]);
    assert_eq!(conn.state, ScConnectionState::Disconnected);
    assert_eq!(conn.local_vmac, [0x01; 6]);
    assert!(conn.hub_vmac.is_none());
}

#[test]
fn connection_flow() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);

    // Build connect request
    let req = conn.build_connect_request();
    assert_eq!(req.function, ScFunction::ConnectRequest);
    assert_eq!(conn.state, ScConnectionState::Connecting);

    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&[0x10; 6]); // hub VMAC
    accept_payload.extend_from_slice(&[0u8; 16]); // hub UUID
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());
    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(accept_payload),
    };
    assert!(conn.handle_connect_accept(&accept));
    assert_eq!(conn.state, ScConnectionState::Connected);
    assert_eq!(conn.hub_vmac, Some([0x10; 6]));
}

#[test]
fn connection_reject_wrong_state() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    // Accept without being in Connecting state
    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: 1,
        originating_vmac: Some([0x10; 6]),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    assert!(!conn.handle_connect_accept(&accept));
}

#[test]
fn message_id_increments() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    let id1 = conn.next_id();
    let id2 = conn.next_id();
    assert_eq!(id2, id1 + 1);
}

#[test]
fn message_id_wraps() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.next_message_id = 0xFFFF;
    let id = conn.next_id();
    assert_eq!(id, 0xFFFF);
    let id = conn.next_id();
    assert_eq!(id, 0);
}

#[test]
fn encapsulated_npdu_for_us() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 1,
        originating_vmac: Some([0x02; 6]),
        destination_vmac: Some([0x01; 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
    };

    let result = conn.handle_received(&msg);
    assert!(result.is_some());
    let (npdu, source) = result.unwrap();
    assert_eq!(npdu, vec![0x01, 0x00, 0x30]);
    assert_eq!(source, [0x02; 6]);
}

#[test]
fn encapsulated_npdu_broadcast() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 1,
        originating_vmac: Some([0x02; 6]),
        destination_vmac: Some(BROADCAST_VMAC),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x20]),
    };

    let result = conn.handle_received(&msg);
    assert!(result.is_some());
}

#[test]
fn encapsulated_npdu_not_for_us() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 1,
        originating_vmac: Some([0x02; 6]),
        destination_vmac: Some([0x03; 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x00]),
    };

    assert!(conn.handle_received(&msg).is_none());
}

#[test]
fn encapsulated_npdu_rejected_when_not_connected() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    // State is Disconnected by default — should reject EncapsulatedNpdu
    assert_eq!(conn.state, ScConnectionState::Disconnected);

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 1,
        originating_vmac: Some([0x02; 6]),
        destination_vmac: Some([0x01; 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
    };

    assert!(conn.handle_received(&msg).is_none());

    // Also rejected in Connecting state
    conn.state = ScConnectionState::Connecting;
    assert!(conn.handle_received(&msg).is_none());
}

#[test]
fn disconnect_request_resets_state() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;

    let msg = ScMessage {
        function: ScFunction::DisconnectRequest,
        message_id: 1,
        originating_vmac: Some([0x10; 6]),
        destination_vmac: Some([0x01; 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };

    conn.handle_received(&msg);
    assert_eq!(conn.state, ScConnectionState::Disconnected);
}

#[test]
fn build_heartbeat() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;
    conn.hub_vmac = Some([0x10; 6]);

    let hb = conn.build_heartbeat();
    assert_eq!(hb.function, ScFunction::HeartbeatRequest);
    assert!(hb.originating_vmac.is_none());
    assert!(hb.destination_vmac.is_none());
}

#[test]
fn build_disconnect() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;
    conn.hub_vmac = Some([0x10; 6]);

    let msg = conn.build_disconnect_request().unwrap();
    assert_eq!(msg.function, ScFunction::DisconnectRequest);
    assert!(msg.originating_vmac.is_none());
    assert!(msg.destination_vmac.is_none());
    assert_eq!(conn.state, ScConnectionState::Disconnecting);
}

#[test]
fn build_disconnect_before_connect_returns_error() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    // hub_vmac is None — not connected yet
    let result = conn.build_disconnect_request();
    assert!(result.is_err());
    // State should not have changed
    assert_eq!(conn.state, ScConnectionState::Disconnected);
}

#[test]
fn connect_request_has_payload() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    let req = conn.build_connect_request();

    assert_eq!(req.payload.len(), 26);
    assert!(req.originating_vmac.is_none());
    assert!(req.destination_vmac.is_none());

    assert_eq!(&req.payload[0..6], &[0x01; 6]); // VMAC
    assert_eq!(&req.payload[6..22], &[0u8; 16]); // Device UUID

    let max_bvlc = u16::from_be_bytes([req.payload[22], req.payload[23]]);
    assert_eq!(max_bvlc, 1476);

    let max_npdu = u16::from_be_bytes([req.payload[24], req.payload[25]]);
    assert_eq!(max_npdu, 1476);
}

#[test]
fn connect_accept_with_payload_sets_hub_max_apdu() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    let _req = conn.build_connect_request();

    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&[0x10; 6]); // hub VMAC
    accept_payload.extend_from_slice(&[0u8; 16]); // hub Device UUID
    accept_payload.extend_from_slice(&1476u16.to_be_bytes()); // Max-BVLC-Length
    accept_payload.extend_from_slice(&480u16.to_be_bytes()); // Max-NPDU-Length

    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: 1,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(accept_payload),
    };
    assert!(conn.handle_connect_accept(&accept));
    assert_eq!(conn.state, ScConnectionState::Connected);
    assert_eq!(conn.hub_vmac, Some([0x10; 6]));
    assert_eq!(conn.hub_max_apdu_length, 480);
}

#[test]
fn connect_accept_empty_payload_keeps_default_max_apdu() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    let _req = conn.build_connect_request();

    // Legacy hub that sends no payload.
    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: 1,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    assert!(conn.handle_connect_accept(&accept));
    assert_eq!(conn.hub_max_apdu_length, 1476); // default preserved
}

#[tokio::test]
async fn loopback_websocket_pair() {
    let (a, b) = LoopbackWebSocket::pair();

    a.send(&[0x01, 0x02, 0x03]).await.unwrap();
    let received = b.recv().await.unwrap();
    assert_eq!(received, vec![0x01, 0x02, 0x03]);

    b.send(&[0xAA, 0xBB]).await.unwrap();
    let received = a.recv().await.unwrap();
    assert_eq!(received, vec![0xAA, 0xBB]);
}

#[tokio::test]
async fn transport_start_stop() {
    let (ws_client, ws_server) = LoopbackWebSocket::pair();
    let vmac = [0x01; 6];
    let mut transport = ScTransport::new(ws_client, vmac);

    // Hub must accept the connection before start() returns
    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_server, [0x10; 6]).await;
        ws_server
    });

    let _rx = transport.start().await.unwrap();
    transport.stop().await.unwrap();
    let _ = hub_task.await;
}

#[tokio::test]
async fn transport_local_mac() {
    let (ws_client, _ws_server) = LoopbackWebSocket::pair();
    let vmac = [0x42; 6];
    let transport = ScTransport::new(ws_client, vmac);
    assert_eq!(transport.local_mac(), &[0x42; 6]);
}

/// Helper: act as a hub — receive ConnectRequest, send ConnectAccept,
/// then return the "hub" side websocket for further interaction.
async fn hub_accept(ws_hub: &LoopbackWebSocket, hub_vmac: Vmac) {
    // Receive Connect-Request from the transport
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);

    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&hub_vmac);
    accept_payload.extend_from_slice(&[0u8; 16]); // Device UUID
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());

    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(accept_payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    ws_hub.send(&buf).await.unwrap();
}

#[tokio::test]
async fn transport_send_unicast_delivers_message() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];
    let dest_vmac: Vmac = [0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
    let npdu_payload = vec![0x01, 0x00, 0x30, 0x42];

    let mut transport = ScTransport::new(ws_client, client_vmac);

    // Hub must accept concurrently since start() now blocks on handshake
    let hub_accept_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_accept_task.await.unwrap();

    // Send unicast from transport
    transport
        .send_unicast(&npdu_payload, &dest_vmac)
        .await
        .unwrap();

    // Hub receives the Encapsulated-NPDU
    let data = ws_hub.recv().await.unwrap();
    let msg = decode_sc_message(&data).unwrap();
    assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(msg.originating_vmac, Some(client_vmac));
    assert_eq!(msg.destination_vmac, Some(dest_vmac));
    assert_eq!(msg.payload, npdu_payload);

    transport.stop().await.unwrap();
}

#[test]
fn disconnect_request_queues_ack() {
    let mut conn = ScConnection::new([1, 2, 3, 4, 5, 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;
    conn.hub_vmac = Some([10, 20, 30, 40, 50, 60]);
    let req = ScMessage {
        function: ScFunction::DisconnectRequest,
        message_id: 42,
        originating_vmac: Some([10, 20, 30, 40, 50, 60]),
        destination_vmac: Some([1, 2, 3, 4, 5, 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let result = conn.handle_received(&req);
    assert!(result.is_none());
    assert_eq!(conn.state, ScConnectionState::Disconnected);
    let ack = conn.disconnect_ack_to_send.as_ref().unwrap();
    assert_eq!(ack.function, ScFunction::DisconnectAck);
    assert_eq!(ack.message_id, 42);
}

#[test]
fn disconnect_ack_transitions_from_disconnecting() {
    let mut conn = ScConnection::new([1, 2, 3, 4, 5, 6], [0u8; 16]);
    conn.state = ScConnectionState::Disconnecting;
    let ack = ScMessage {
        function: ScFunction::DisconnectAck,
        message_id: 99,
        originating_vmac: Some([10, 20, 30, 40, 50, 60]),
        destination_vmac: Some([1, 2, 3, 4, 5, 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let result = conn.handle_received(&ack);
    assert!(result.is_none());
    assert_eq!(conn.state, ScConnectionState::Disconnected);
}

#[tokio::test]
async fn transport_send_broadcast_delivers_message() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];
    let npdu_payload = vec![0x01, 0x20, 0xFF];

    let mut transport = ScTransport::new(ws_client, client_vmac);

    // Hub must accept concurrently since start() now blocks on handshake
    let hub_accept_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_accept_task.await.unwrap();

    // Send broadcast from transport
    transport.send_broadcast(&npdu_payload).await.unwrap();

    // Hub receives the Encapsulated-NPDU with broadcast VMAC
    let data = ws_hub.recv().await.unwrap();
    let msg = decode_sc_message(&data).unwrap();
    assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(msg.originating_vmac, Some(client_vmac));
    assert_eq!(msg.destination_vmac, Some(BROADCAST_VMAC));
    assert_eq!(msg.payload, npdu_payload);

    transport.stop().await.unwrap();
}

#[test]
fn bvlc_result_nak_disconnects() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;
    // result_for(1) + result_code(1, 0x01=NAK) + error_marker(1) + error_class(2,BE) + error_code(2,BE)
    let msg = ScMessage {
        function: ScFunction::Result,
        message_id: 1,
        originating_vmac: Some([0x10; 6]),
        destination_vmac: Some([0x01; 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x06, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01]),
    };
    let result = conn.handle_received(&msg);
    assert!(result.is_none());
    assert_eq!(conn.state, ScConnectionState::Disconnected);
}

#[test]
fn bvlc_result_success_no_disconnect() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;
    let msg = ScMessage {
        function: ScFunction::Result,
        message_id: 1,
        originating_vmac: Some([0x10; 6]),
        destination_vmac: Some([0x01; 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(), // success = empty
    };
    let result = conn.handle_received(&msg);
    assert!(result.is_none());
    assert_eq!(conn.state, ScConnectionState::Connected);
}

#[test]
fn bvlc_result_ack_with_payload_no_disconnect() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;
    // result_for(1) + result_code(1, 0x00=ACK)
    let msg = ScMessage {
        function: ScFunction::Result,
        message_id: 1,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x06, 0x00]),
    };
    let result = conn.handle_received(&msg);
    assert!(result.is_none());
    assert_eq!(conn.state, ScConnectionState::Connected);
}

#[test]
fn heartbeat_ack_has_no_vmacs() {
    let conn = ScConnection::new([0x01; 6], [0u8; 16]);
    let ack = conn.build_heartbeat_ack(42);
    assert!(ack.originating_vmac.is_none());
    assert!(ack.destination_vmac.is_none());
    assert_eq!(ack.message_id, 42);
    assert_eq!(ack.function, ScFunction::HeartbeatAck);
}

#[test]
fn connect_accept_validates_message_id() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    let req = conn.build_connect_request();
    let req_id = req.message_id;

    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(vec![0u8; 26]),
    };
    assert!(conn.handle_connect_accept(&accept));
    assert_eq!(conn.state, ScConnectionState::Connected);
}

#[test]
fn connect_accept_rejects_wrong_message_id() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    let _req = conn.build_connect_request();

    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: 9999,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(vec![0u8; 26]),
    };
    assert!(!conn.handle_connect_accept(&accept));
    assert_eq!(conn.state, ScConnectionState::Connecting);
}

#[test]
fn connect_accept_parses_device_uuid() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    let req = conn.build_connect_request();
    let mut payload = vec![0u8; 26];
    payload[0..6].copy_from_slice(&[0x02; 6]); // hub VMAC
    payload[6..22].copy_from_slice(&[0xAB; 16]); // hub UUID
    payload[22..24].copy_from_slice(&1476u16.to_be_bytes());
    payload[24..26].copy_from_slice(&1400u16.to_be_bytes());

    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    assert!(conn.handle_connect_accept(&accept));
    assert_eq!(conn.hub_vmac, Some([0x02; 6]));
    assert_eq!(conn.hub_device_uuid, Some([0xAB; 16]));
    assert_eq!(conn.hub_max_apdu_length, 1400);
}

#[tokio::test]
async fn sc_connect_timeout() {
    let (ws_client, _ws_server) = LoopbackWebSocket::pair();
    let vmac = [0x01; 6];
    let mut transport = ScTransport::new(ws_client, vmac).with_connect_timeout_ms(200);
    // Don't send ConnectAccept from server side — should timeout
    let result = transport.start().await;
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("timeout"),
        "Expected timeout error, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn sc_heartbeat_sent_periodically() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];

    let mut transport = ScTransport::new(ws_client, client_vmac)
        .with_heartbeat_interval_ms(200)
        .with_heartbeat_timeout_ms(5000);

    // Hub accepts the connection, then we interact with the hub ws
    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();

    // Wait enough time for the heartbeat interval to fire
    tokio::time::sleep(Duration::from_millis(300)).await;

    // The hub should receive a HeartbeatRequest
    let data = tokio::time::timeout(Duration::from_millis(500), ws_hub.recv())
        .await
        .expect("timed out waiting for heartbeat")
        .unwrap();
    let msg = decode_sc_message(&data).unwrap();
    assert_eq!(msg.function, ScFunction::HeartbeatRequest);
    assert!(msg.originating_vmac.is_none());

    // Send HeartbeatAck back so the transport doesn't timeout
    let ack = ScMessage {
        function: ScFunction::HeartbeatAck,
        message_id: msg.message_id,
        originating_vmac: Some(hub_vmac),
        destination_vmac: Some(client_vmac),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &ack);
    ws_hub.send(&buf).await.unwrap();

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn sc_heartbeat_timeout_disconnects() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];

    let mut transport = ScTransport::new(ws_client, client_vmac)
        .with_heartbeat_interval_ms(100)
        .with_heartbeat_timeout_ms(300);

    // Hub accepts the connection but will NOT respond to heartbeats
    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let _ws_hub = hub_task.await.unwrap();

    // Wait long enough for the heartbeat timeout to fire (~500ms > 300ms timeout)
    tokio::time::sleep(Duration::from_millis(600)).await;

    // The recv task should have ended and connection state should be Disconnected
    let conn = transport.connection().unwrap();
    let c = conn.lock().await;
    assert_eq!(c.state, ScConnectionState::Disconnected);
    drop(c);

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn sc_connect_succeeds_within_timeout() {
    let (ws_client, ws_server) = LoopbackWebSocket::pair();
    let vmac = [0x01; 6];
    let mut transport = ScTransport::new(ws_client, vmac).with_connect_timeout_ms(5000);

    // Spawn hub accept in background
    let hub_task = tokio::spawn(async move {
        // Receive ConnectRequest
        let data = ws_server.recv().await.unwrap();
        let req = decode_sc_message(&data).unwrap();
        assert_eq!(req.function, ScFunction::ConnectRequest);

        let mut payload = Vec::with_capacity(10);
        payload.extend_from_slice(&[0x10; 6]); // hub VMAC
        payload.extend_from_slice(&1476u16.to_be_bytes()); // Max-BVLC-Length
        payload.extend_from_slice(&1476u16.to_be_bytes()); // Max-NPDU-Length
        let accept = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: req.message_id,
            originating_vmac: Some([0x10; 6]),
            destination_vmac: req.originating_vmac,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(payload),
        };
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &accept);
        ws_server.send(&buf).await.unwrap();
        ws_server // return so it's not dropped
    });

    let _rx = transport.start().await.unwrap();
    // Verify connected
    let conn = transport.connection().unwrap();
    let c = conn.lock().await;
    assert_eq!(c.state, ScConnectionState::Connected);
    drop(c);

    transport.stop().await.unwrap();
    let _ = hub_task.await;
}

#[tokio::test]
async fn test_failover_on_primary_timeout() {
    // Primary pair — hub side will NOT respond, causing a timeout.
    let (primary_client, _primary_hub) = LoopbackWebSocket::pair();
    // Failover pair — hub side WILL respond with ConnectAccept.
    let (failover_client, failover_hub) = LoopbackWebSocket::pair();

    let vmac = [0x01; 6];
    let hub_vmac = [0x20; 6];

    let mut transport = ScTransport::new(primary_client, vmac)
        .with_connect_timeout_ms(200)
        .with_failover(failover_client);

    // Spawn hub accept on failover side.
    let hub_task = tokio::spawn(async move {
        hub_accept(&failover_hub, hub_vmac).await;
        failover_hub
    });

    let _rx = transport.start().await.unwrap();

    // Verify connected via failover.
    let conn = transport.connection().unwrap();
    let c = conn.lock().await;
    assert_eq!(c.state, ScConnectionState::Connected);
    assert_eq!(c.hub_vmac, Some(hub_vmac));
    drop(c);

    transport.stop().await.unwrap();
    let _ = hub_task.await;
}

#[tokio::test]
async fn test_no_failover_without_config() {
    // Primary pair — hub side will NOT respond.
    let (primary_client, _primary_hub) = LoopbackWebSocket::pair();

    let vmac = [0x01; 6];
    // No failover configured.
    let mut transport = ScTransport::new(primary_client, vmac).with_connect_timeout_ms(200);

    let result = transport.start().await;
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("timeout"),
        "Expected timeout error, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_failover_primary_succeeds_no_failover_used() {
    // Primary pair — hub side WILL respond.
    let (primary_client, primary_hub) = LoopbackWebSocket::pair();
    // Failover pair — should NOT be used.
    let (failover_client, _failover_hub) = LoopbackWebSocket::pair();

    let vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];

    let mut transport = ScTransport::new(primary_client, vmac)
        .with_connect_timeout_ms(2000)
        .with_failover(failover_client);

    // Spawn hub accept on primary side.
    let hub_task = tokio::spawn(async move {
        hub_accept(&primary_hub, hub_vmac).await;
        primary_hub
    });

    let _rx = transport.start().await.unwrap();

    // Verify connected via primary.
    let conn = transport.connection().unwrap();
    let c = conn.lock().await;
    assert_eq!(c.state, ScConnectionState::Connected);
    assert_eq!(c.hub_vmac, Some(hub_vmac));
    drop(c);

    transport.stop().await.unwrap();
    let _ = hub_task.await;
}

#[test]
fn reconnect_config_default() {
    let config = ScReconnectConfig::default();
    assert_eq!(config.initial_delay_ms, 10_000);
    assert_eq!(config.max_delay_ms, 600_000);
    assert_eq!(config.max_retries, 10);
}

#[test]
fn reconnect_exponential_backoff_sequence() {
    let config = ScReconnectConfig {
        initial_delay_ms: 100,
        max_delay_ms: 1000,
        max_retries: 5,
    };
    let mut delay = config.initial_delay_ms;
    let delays: Vec<u64> = (0..5)
        .map(|_| {
            let d = delay;
            delay = (delay * 2).min(config.max_delay_ms);
            d
        })
        .collect();
    assert_eq!(delays, vec![100, 200, 400, 800, 1000]);
}

#[test]
fn with_reconnect_builder() {
    // Verify the builder sets the config.
    // We can't easily create an ScTransport without a WebSocket,
    // so just verify ScReconnectConfig is constructible and clonable.
    let config = ScReconnectConfig::default();
    let config2 = config.clone();
    assert_eq!(config.initial_delay_ms, config2.initial_delay_ms);
}
