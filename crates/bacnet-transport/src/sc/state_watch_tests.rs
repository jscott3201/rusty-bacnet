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
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    ws_hub.send(&buf).await.unwrap();
}

async fn wait_for_state_notification(
    states: &mut tokio::sync::watch::Receiver<ScConnectionState>,
    expected: ScConnectionState,
    timeout: Duration,
) {
    tokio::time::timeout(timeout, async {
        loop {
            if *states.borrow_and_update() == expected {
                return;
            }
            states.changed().await.expect("state watch closed");
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {expected:?} notification"));
}

async fn send_message(ws: &LoopbackWebSocket, msg: &ScMessage) {
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, msg);
    ws.send(&buf).await.unwrap();
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

#[tokio::test]
async fn sc_connection_state_changes_reports_connected_then_disconnected() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];

    let mut transport =
        ScTransport::new(ws_client, client_vmac).with_test_heartbeat_timing_ms(100, 300);
    let mut states = transport.connection_state_changes();

    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let _ws_hub = hub_task.await.unwrap();

    wait_for_state_notification(
        &mut states,
        ScConnectionState::Connected,
        Duration::from_secs(1),
    )
    .await;
    wait_for_state_notification(
        &mut states,
        ScConnectionState::Disconnected,
        Duration::from_secs(1),
    )
    .await;

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn sc_connection_state_changes_reports_bvlc_result_disconnect_without_stale_stop() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];

    let mut transport = ScTransport::new(ws_client, client_vmac);
    let mut states = transport.connection_state_changes();

    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();

    wait_for_state_notification(
        &mut states,
        ScConnectionState::Connected,
        Duration::from_secs(1),
    )
    .await;
    send_message(&ws_hub, &bvlc_result_nak(0x66)).await;
    wait_for_state_notification(
        &mut states,
        ScConnectionState::Disconnected,
        Duration::from_secs(1),
    )
    .await;

    transport.stop().await.unwrap();

    match tokio::time::timeout(Duration::from_millis(100), ws_hub.recv()).await {
        Ok(Ok(data)) => {
            let msg = decode_sc_message(&data).unwrap();
            assert_ne!(
                msg.function,
                ScFunction::DisconnectRequest,
                "stop sent a stale Disconnect-Request after fatal disconnect"
            );
        }
        Ok(Err(_)) | Err(_) => {}
    }
}
