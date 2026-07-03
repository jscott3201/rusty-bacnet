use super::*;
use crate::sc_frame::ScOption;

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
fn encapsulated_npdu_unicast_from_hub() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 1,
        originating_vmac: Some([0x02; 6]),
        destination_vmac: None,
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
fn encapsulated_npdu_rejects_non_broadcast_destination_from_hub() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 1,
        originating_vmac: Some([0x02; 6]),
        destination_vmac: Some([0x01; 6]),
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
fn build_disconnect_after_disconnect_returns_error() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.hub_vmac = Some([0x10; 6]);

    let result = conn.build_disconnect_request();
    assert!(result.is_err());
    assert_eq!(conn.state, ScConnectionState::Disconnected);
}

#[test]
fn connect_request_has_payload() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.max_bvlc_length = 1200;
    conn.max_apdu_length = 900;
    let req = conn.build_connect_request();

    assert_eq!(req.payload.len(), 26);
    assert!(req.originating_vmac.is_none());
    assert!(req.destination_vmac.is_none());

    assert_eq!(&req.payload[0..6], &[0x01; 6]); // VMAC
    assert_eq!(&req.payload[6..22], &[0u8; 16]); // Device UUID

    let max_bvlc = u16::from_be_bytes([req.payload[22], req.payload[23]]);
    assert_eq!(max_bvlc, 1200);

    let max_npdu = u16::from_be_bytes([req.payload[24], req.payload[25]]);
    assert_eq!(max_npdu, 900);
}

#[test]
fn connect_accept_with_payload_sets_hub_max_bvlc_and_apdu() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    let _req = conn.build_connect_request();

    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&[0x10; 6]); // hub VMAC
    accept_payload.extend_from_slice(&[0u8; 16]); // hub Device UUID
    accept_payload.extend_from_slice(&1200u16.to_be_bytes()); // Max-BVLC-Length
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
    assert_eq!(conn.hub_max_bvlc_length, 1200);
    assert_eq!(conn.hub_max_apdu_length, 480);
}

#[test]
fn handle_received_rejects_npdu_above_local_max_npdu() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;
    conn.max_apdu_length = 2;

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 1,
        originating_vmac: Some([0x02; 6]),
        destination_vmac: Some([0x01; 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x02, 0x03]),
    };

    assert!(conn.handle_received(&msg).is_none());
}

#[test]
fn connect_accept_rejects_short_payload() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    let _req = conn.build_connect_request();

    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: 1,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x10; 6]),
    };
    assert!(!conn.handle_connect_accept(&accept));
    assert_eq!(conn.state, ScConnectionState::Connecting);
    assert_eq!(conn.hub_vmac, None);
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
async fn transport_receive_preserves_data_options_as_attributes() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];
    let mut transport = ScTransport::new(ws_client, client_vmac);

    let hub_accept_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let mut rx = transport.start().await.unwrap();
    let ws_hub = hub_accept_task.await.unwrap();

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x1234,
        originating_vmac: Some(hub_vmac),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: vec![
            ScOption {
                option_type: 1,
                must_understand: true,
                data: Vec::new(),
            },
            ScOption {
                option_type: 31,
                must_understand: false,
                data: vec![0x12, 0x34, 0x56],
            },
        ],
        payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &msg);
    ws_hub.send(&buf).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for SC NPDU")
        .expect("SC NPDU channel closed");
    assert_eq!(received.npdu, msg.payload);
    assert_eq!(received.source_mac.as_slice(), hub_vmac);
    assert_eq!(received.data_attributes.len(), 2);
    assert_eq!(received.data_attributes[0].option_type, 1);
    assert!(received.data_attributes[0].must_understand);
    assert!(received.data_attributes[0].data.is_empty());
    assert_eq!(received.data_attributes[1].option_type, 31);
    assert!(!received.data_attributes[1].must_understand);
    assert_eq!(received.data_attributes[1].data, vec![0x12, 0x34, 0x56]);

    transport.stop().await.unwrap();
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
    hub_accept_with_limits(ws_hub, hub_vmac, 1476, 1476).await;
}

async fn hub_accept_with_limits(
    ws_hub: &LoopbackWebSocket,
    hub_vmac: Vmac,
    max_bvlc: u16,
    max_npdu: u16,
) {
    // Receive Connect-Request from the transport
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);

    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&hub_vmac);
    accept_payload.extend_from_slice(&[0u8; 16]); // Device UUID
    accept_payload.extend_from_slice(&max_bvlc.to_be_bytes());
    accept_payload.extend_from_slice(&max_npdu.to_be_bytes());

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
    assert_eq!(msg.originating_vmac, None);
    assert_eq!(msg.destination_vmac, Some(dest_vmac));
    assert_eq!(msg.payload, npdu_payload);

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn transport_send_unicast_encodes_data_attributes_as_options() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];
    let dest_vmac: Vmac = [0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
    let npdu_payload = vec![0x01, 0x00, 0x30, 0x42];
    let data_attributes = vec![
        DataAttribute {
            option_type: 1,
            must_understand: true,
            data: Vec::new(),
        },
        DataAttribute {
            option_type: 31,
            must_understand: false,
            data: vec![0x12, 0x34, 0x56],
        },
    ];

    let mut transport = ScTransport::new(ws_client, client_vmac);
    let hub_accept_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_accept_task.await.unwrap();

    transport
        .send_unicast_with_data_attributes(&npdu_payload, &dest_vmac, &data_attributes)
        .await
        .unwrap();

    let data = ws_hub.recv().await.unwrap();
    let msg = decode_sc_message(&data).unwrap();
    assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(msg.destination_vmac, Some(dest_vmac));
    assert_eq!(msg.data_options.len(), 2);
    assert_eq!(msg.data_options[0].option_type, 1);
    assert!(msg.data_options[0].must_understand);
    assert!(msg.data_options[0].data.is_empty());
    assert_eq!(msg.data_options[1].option_type, 31);
    assert!(!msg.data_options[1].must_understand);
    assert_eq!(msg.data_options[1].data, vec![0x12, 0x34, 0x56]);
    assert_eq!(msg.payload, npdu_payload);

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn transport_send_unicast_rejects_invalid_data_attribute_type() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]);
    let invalid_attribute = DataAttribute {
        option_type: 0,
        must_understand: false,
        data: Vec::new(),
    };

    let hub_accept_task = tokio::spawn(async move {
        hub_accept(&ws_hub, [0x10; 6]).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_accept_task.await.unwrap();

    let err = transport
        .send_unicast_with_data_attributes(&[0x01, 0x02], &[0x02; 6], &[invalid_attribute])
        .await
        .unwrap_err();

    assert!(format!("{err}").contains("1..31"));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), ws_hub.recv())
            .await
            .is_err()
    );

    transport.stop().await.unwrap();
}

#[test]
fn connection_rejects_too_many_data_attributes_on_encode() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    let attributes = vec![
        DataAttribute {
            option_type: 1,
            must_understand: false,
            data: Vec::new(),
        };
        65
    ];

    let err = conn
        .build_encapsulated_npdu_with_data_attributes([0x02; 6], &[0x01, 0x02], &attributes)
        .unwrap_err();

    assert!(format!("{err}").contains("exceed 64"));
}

#[test]
fn connection_rejects_oversize_data_attribute_payload_on_encode() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    let attribute = DataAttribute {
        option_type: 1,
        must_understand: false,
        data: vec![0; u16::MAX as usize + 1],
    };

    let err = conn
        .build_encapsulated_npdu_with_data_attributes([0x02; 6], &[0x01, 0x02], &[attribute])
        .unwrap_err();

    assert!(format!("{err}").contains("exceeds 65535"));
}

#[tokio::test]
async fn transport_send_unicast_rejects_peer_max_npdu() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]);

    let hub_accept_task = tokio::spawn(async move {
        hub_accept_with_limits(&ws_hub, [0x10; 6], 1476, 2).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_accept_task.await.unwrap();
    let err = transport
        .send_unicast(&[0x01, 0x02, 0x03], &[0x02; 6])
        .await
        .unwrap_err();

    assert!(format!("{err}").contains("Max-NPDU-Length"));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), ws_hub.recv())
            .await
            .is_err()
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn transport_send_unicast_rejects_peer_max_bvlc() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]);

    let hub_accept_task = tokio::spawn(async move {
        hub_accept_with_limits(&ws_hub, [0x10; 6], 13, 1476).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_accept_task.await.unwrap();
    let err = transport
        .send_unicast(&[0x01, 0x02, 0x03, 0x04], &[0x02; 6])
        .await
        .unwrap_err();

    assert!(format!("{err}").contains("Max-BVLC-Length"));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), ws_hub.recv())
            .await
            .is_err()
    );
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
    assert_eq!(msg.originating_vmac, None);
    assert_eq!(msg.destination_vmac, Some(BROADCAST_VMAC));
    assert_eq!(msg.payload, npdu_payload);

    transport.stop().await.unwrap();
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
