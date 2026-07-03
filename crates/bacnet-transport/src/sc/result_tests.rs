use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

use super::*;

async fn hub_accept(ws_hub: &LoopbackWebSocket, hub_vmac: Vmac) {
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);

    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&hub_vmac);
    accept_payload.extend_from_slice(&[0u8; 16]);
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
    send_message(ws_hub, &accept).await;
}

fn bvlc_result_nak(message_id: u16) -> ScMessage {
    ScMessage {
        function: ScFunction::Result,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x06, 0x01, 0x00, 0x00, 0x07, 0x01, 0x17]),
    }
}

fn connect_result_nak(message_id: u16, error_code: u16) -> ScMessage {
    let mut payload = Vec::from([
        ScFunction::ConnectRequest.to_raw(),
        0x01,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
    ]);
    payload[3..5].copy_from_slice(&ErrorClass::COMMUNICATION.to_raw().to_be_bytes());
    payload[5..7].copy_from_slice(&error_code.to_be_bytes());
    ScMessage {
        function: ScFunction::Result,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    }
}

async fn send_message(ws: &LoopbackWebSocket, msg: &ScMessage) {
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, msg);
    ws.send(&buf).await.unwrap();
}

async fn wait_until_disconnected(conn: &Arc<Mutex<ScConnection>>) {
    for _ in 0..20 {
        if conn.lock().await.state == ScConnectionState::Disconnected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("connection did not transition to Disconnected");
}

#[test]
fn bvlc_result_nak_disconnects() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;
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
        payload: Bytes::from_static(&[0x0C, 0x00]),
    };
    let result = conn.handle_received(&msg);
    assert!(result.is_none());
    assert_eq!(conn.state, ScConnectionState::Connected);
}

#[test]
fn bvlc_result_ack_with_payload_no_disconnect() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;
    let msg = ScMessage {
        function: ScFunction::Result,
        message_id: 1,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x0C, 0x00]),
    };
    let result = conn.handle_received(&msg);
    assert!(result.is_none());
    assert_eq!(conn.state, ScConnectionState::Connected);
}

#[test]
fn malformed_bvlc_result_disconnects() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    conn.state = ScConnectionState::Connected;
    let msg = ScMessage {
        function: ScFunction::Result,
        message_id: 1,
        originating_vmac: Some([0x10; 6]),
        destination_vmac: Some([0x01; 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let result = conn.handle_received(&msg);
    assert!(result.is_none());
    assert_eq!(conn.state, ScConnectionState::Disconnected);
}

#[test]
fn random48_vmac_sets_annex_h_marker_nibble() {
    let vmac = generate_random48_vmac().unwrap();
    assert_eq!(vmac[0] & 0x0F, 0x02);
}

#[test]
fn connect_result_duplicate_vmac_reseeds_local_vmac() {
    let mut conn = ScConnection::new([0x01; 6], [0u8; 16]);
    let req = conn.build_connect_request();
    let result = decode_sc_bvlc_result(&connect_result_nak(
        req.message_id,
        ErrorCode::NODE_DUPLICATE_VMAC.to_raw(),
    ))
    .unwrap();

    assert!(conn.handle_connect_result(req.message_id, &result).unwrap());
    assert_eq!(conn.state, ScConnectionState::Disconnected);
    assert_ne!(conn.local_vmac, [0x01; 6]);
    assert_eq!(conn.local_vmac[0] & 0x0F, 0x02);

    let retry = conn.build_connect_request();
    assert_eq!(&retry.payload[0..6], conn.local_vmac.as_slice());
}

#[test]
fn connect_result_duplicate_vmac_wrong_message_id_does_not_reseed() {
    let original_vmac = [0x22, 0x01, 0x02, 0x03, 0x04, 0x05];
    let mut conn = ScConnection::new(original_vmac, [0u8; 16]);
    let req = conn.build_connect_request();
    let wrong_message_id = req.message_id.wrapping_add(1);
    let result = decode_sc_bvlc_result(&connect_result_nak(
        wrong_message_id,
        ErrorCode::NODE_DUPLICATE_VMAC.to_raw(),
    ))
    .unwrap();

    assert!(!conn
        .handle_connect_result(wrong_message_id, &result)
        .unwrap());
    assert_eq!(conn.state, ScConnectionState::Disconnected);
    assert_eq!(conn.local_vmac, original_vmac);

    let retry = conn.build_connect_request();
    assert_eq!(&retry.payload[0..6], original_vmac.as_slice());
}

#[test]
fn connect_result_generic_nak_does_not_reseed_local_vmac() {
    let original_vmac = [0x22, 0x01, 0x02, 0x03, 0x04, 0x05];
    let mut conn = ScConnection::new(original_vmac, [0u8; 16]);
    let req = conn.build_connect_request();
    let result = decode_sc_bvlc_result(&connect_result_nak(
        req.message_id,
        ErrorCode::UNEXPECTED_DATA.to_raw(),
    ))
    .unwrap();

    assert!(!conn.handle_connect_result(req.message_id, &result).unwrap());
    assert_eq!(conn.state, ScConnectionState::Disconnected);
    assert_eq!(conn.local_vmac, original_vmac);
}

fn fail_random48_vmac() -> Result<Vmac, Error> {
    Err(Error::Encoding(
        "test Random-48 VMAC generation failure".into(),
    ))
}

#[tokio::test]
async fn sc_connect_result_nak_fails_without_timeout() {
    let (ws_client, ws_server) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_connect_timeout_ms(5000);

    let hub_task = tokio::spawn(async move {
        let data = ws_server.recv().await.unwrap();
        let req = decode_sc_message(&data).unwrap();
        assert_eq!(req.function, ScFunction::ConnectRequest);
        send_message(&ws_server, &bvlc_result_nak(req.message_id)).await;
    });

    let started = Instant::now();
    let result = transport.start().await;
    assert!(result.is_err());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "BVLC-Result NAK should fail connect before timeout"
    );
    let err = result.unwrap_err();
    let sc_error = ScConnectError::from_error(&err).expect("NAK should be a typed SC error");
    match sc_error {
        ScConnectError::HandshakeNak {
            result_for,
            error_class,
            error_code,
            duplicate_vmac_reseeded,
            ..
        } => {
            assert_eq!(*result_for, ScFunction::ConnectRequest);
            assert_eq!(*error_class, ErrorClass::COMMUNICATION.to_raw());
            assert_eq!(*error_code, 0x0117);
            assert!(!duplicate_vmac_reseeded);
        }
        other => panic!("expected HandshakeNak, got {other:?}"),
    }

    let conn = transport.connection().unwrap();
    assert_eq!(conn.lock().await.state, ScConnectionState::Disconnected);
    hub_task.await.unwrap();
}

#[tokio::test]
async fn sc_connect_result_nak_preserves_error_details() {
    let (ws_client, ws_server) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_connect_timeout_ms(5000);

    let hub_task = tokio::spawn(async move {
        let data = ws_server.recv().await.unwrap();
        let req = decode_sc_message(&data).unwrap();
        assert_eq!(req.function, ScFunction::ConnectRequest);
        let mut nak = connect_result_nak(req.message_id, ErrorCode::NODE_DUPLICATE_VMAC.to_raw());
        let mut payload = nak.payload.to_vec();
        payload[2] = 0xBF;
        payload.extend_from_slice(b"duplicate VMAC");
        nak.payload = Bytes::from(payload);
        send_message(&ws_server, &nak).await;
    });

    let err = transport.start().await.unwrap_err();
    let sc_error = ScConnectError::from_error(&err).expect("NAK should be a typed SC error");
    match sc_error {
        ScConnectError::HandshakeNak {
            result_for,
            error_header_marker,
            error_class,
            error_code,
            error_details,
            duplicate_vmac_reseeded,
        } => {
            assert_eq!(*result_for, ScFunction::ConnectRequest);
            assert_eq!(*error_header_marker, 0xBF);
            assert_eq!(*error_class, ErrorClass::COMMUNICATION.to_raw());
            assert_eq!(*error_code, ErrorCode::NODE_DUPLICATE_VMAC.to_raw());
            assert_eq!(error_details, "duplicate VMAC");
            assert!(*duplicate_vmac_reseeded);
        }
        other => panic!("expected HandshakeNak, got {other:?}"),
    }
    hub_task.await.unwrap();
}

#[tokio::test]
async fn sc_connect_not_hub_nak_preserves_error_code_for_matching() {
    let (ws_client, ws_server) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_connect_timeout_ms(5000);

    let hub_task = tokio::spawn(async move {
        let data = ws_server.recv().await.unwrap();
        let req = decode_sc_message(&data).unwrap();
        assert_eq!(req.function, ScFunction::ConnectRequest);
        send_message(
            &ws_server,
            &connect_result_nak(req.message_id, ErrorCode::NOT_A_BACNET_SC_HUB.to_raw()),
        )
        .await;
    });

    let err = transport.start().await.unwrap_err();
    let sc_error = ScConnectError::from_error(&err).expect("NAK should be a typed SC error");
    match sc_error {
        ScConnectError::HandshakeNak {
            error_class,
            error_code,
            duplicate_vmac_reseeded,
            ..
        } => {
            assert_eq!(*error_class, ErrorClass::COMMUNICATION.to_raw());
            assert_eq!(*error_code, ErrorCode::NOT_A_BACNET_SC_HUB.to_raw());
            assert!(!duplicate_vmac_reseeded);
        }
        other => panic!("expected HandshakeNak, got {other:?}"),
    }
    hub_task.await.unwrap();
}

#[tokio::test]
async fn sc_connect_malformed_result_returns_typed_sc_error() {
    let (ws_client, ws_server) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_connect_timeout_ms(5000);

    let hub_task = tokio::spawn(async move {
        let data = ws_server.recv().await.unwrap();
        let req = decode_sc_message(&data).unwrap();
        assert_eq!(req.function, ScFunction::ConnectRequest);
        let malformed = ScMessage {
            function: ScFunction::Result,
            message_id: req.message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(vec![ScFunction::ConnectRequest.to_raw(), 0x01, 0x00]),
        };
        send_message(&ws_server, &malformed).await;
    });

    let err = transport.start().await.unwrap_err();
    let sc_error =
        ScConnectError::from_error(&err).expect("malformed Result should be a typed SC error");
    match sc_error {
        ScConnectError::MalformedBvlcResult { offset, message } => {
            assert_eq!(*offset, 2);
            assert!(message.contains("payload too short"), "{message}");
        }
        other => panic!("expected MalformedBvlcResult, got {other:?}"),
    }
    hub_task.await.unwrap();
}

#[tokio::test]
async fn sc_connect_timeout_returns_timeout_error() {
    let (ws_client, _ws_server) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_connect_timeout_ms(20);

    let err = transport.start().await.unwrap_err();
    assert!(
        ScConnectError::from_error(&err).is_none(),
        "handshake timeout should use top-level Error::Timeout, got {err:?}"
    );
    match err {
        Error::Timeout(duration) => assert_eq!(duration, Duration::from_millis(20)),
        other => panic!("expected timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn sc_connect_duplicate_vmac_nak_retries_failover_with_new_vmac() {
    let (primary_client, primary_hub) = LoopbackWebSocket::pair();
    let (failover_client, failover_hub) = LoopbackWebSocket::pair();
    let original_vmac = [0x01; 6];
    let mut transport = ScTransport::new(primary_client, original_vmac)
        .with_connect_timeout_ms(5000)
        .with_failover(failover_client);

    let primary_task = tokio::spawn(async move {
        let data = primary_hub.recv().await.unwrap();
        let req = decode_sc_message(&data).unwrap();
        assert_eq!(req.function, ScFunction::ConnectRequest);
        assert_eq!(&req.payload[0..6], &original_vmac);
        send_message(
            &primary_hub,
            &connect_result_nak(req.message_id, ErrorCode::NODE_DUPLICATE_VMAC.to_raw()),
        )
        .await;
    });

    let failover_task = tokio::spawn(async move {
        let data = failover_hub.recv().await.unwrap();
        let req = decode_sc_message(&data).unwrap();
        assert_eq!(req.function, ScFunction::ConnectRequest);
        let retry_vmac: Vmac = req.payload[0..6].try_into().unwrap();
        assert_ne!(retry_vmac, original_vmac);
        assert_eq!(retry_vmac[0] & 0x0F, 0x02);

        let mut accept_payload = Vec::with_capacity(26);
        accept_payload.extend_from_slice(&[0x20; 6]);
        accept_payload.extend_from_slice(&[0u8; 16]);
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
        send_message(&failover_hub, &accept).await;
    });

    let _rx = transport.start().await.unwrap();
    primary_task.await.unwrap();
    failover_task.await.unwrap();

    let conn = transport.connection().unwrap();
    let c = conn.lock().await;
    assert_eq!(c.state, ScConnectionState::Connected);
    assert_ne!(c.local_vmac, original_vmac);
    assert_eq!(c.local_vmac[0] & 0x0F, 0x02);
    drop(c);

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn sc_connect_duplicate_vmac_reseed_failure_does_not_try_failover() {
    let _random48_guard = set_test_random48_vmac_generator(fail_random48_vmac);
    let (primary_client, primary_hub) = LoopbackWebSocket::pair();
    let (failover_client, failover_hub) = LoopbackWebSocket::pair();
    let original_vmac = [0x01; 6];
    let mut transport = ScTransport::new(primary_client, original_vmac)
        .with_connect_timeout_ms(5000)
        .with_failover(failover_client);

    let primary_task = tokio::spawn(async move {
        let data = primary_hub.recv().await.unwrap();
        let req = decode_sc_message(&data).unwrap();
        assert_eq!(req.function, ScFunction::ConnectRequest);
        assert_eq!(&req.payload[0..6], &original_vmac);
        send_message(
            &primary_hub,
            &connect_result_nak(req.message_id, ErrorCode::NODE_DUPLICATE_VMAC.to_raw()),
        )
        .await;
    });

    let result = transport.start().await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("test Random-48 VMAC generation failure"),
        "expected reseed failure, got: {err_msg}"
    );
    primary_task.await.unwrap();

    match tokio::time::timeout(Duration::from_millis(100), failover_hub.recv()).await {
        Ok(Ok(data)) => panic!(
            "failover received stale-VMAC Connect-Request: {:02x?}",
            data
        ),
        Ok(Err(_)) | Err(_) => {}
    }

    let conn = transport.connection().unwrap();
    let c = conn.lock().await;
    assert_eq!(c.state, ScConnectionState::Disconnected);
    assert_eq!(c.pending_connect_message_id, None);
    assert_eq!(c.local_vmac, original_vmac);
}

#[tokio::test]
async fn sc_result_nak_closes_receive_loop_before_heartbeat() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];

    let mut transport =
        ScTransport::new(ws_client, client_vmac).with_test_heartbeat_timing_ms(500, 5000);

    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();
    let conn = transport.connection().unwrap().clone();

    send_message(&ws_hub, &bvlc_result_nak(1)).await;
    wait_until_disconnected(&conn).await;

    assert!(
        tokio::time::timeout(Duration::from_millis(200), ws_hub.recv())
            .await
            .is_err(),
        "receive loop should close before sending another heartbeat"
    );

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn malformed_wire_bvlc_result_closes_receive_loop() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];

    let mut transport =
        ScTransport::new(ws_client, client_vmac).with_test_heartbeat_timing_ms(500, 5000);

    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();
    let conn = transport.connection().unwrap().clone();

    ws_hub.send(&[ScFunction::Result.to_raw()]).await.unwrap();
    wait_until_disconnected(&conn).await;

    assert!(
        tokio::time::timeout(Duration::from_millis(200), ws_hub.recv())
            .await
            .is_err(),
        "malformed Result should close the receive loop without a Result response"
    );

    transport.stop().await.unwrap();
}
