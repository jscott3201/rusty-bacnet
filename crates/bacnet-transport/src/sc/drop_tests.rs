use super::*;
use tokio::time::timeout;

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

async fn assert_closed_without_post_drop_heartbeat(ws_hub: &LoopbackWebSocket, context: &str) {
    timeout(Duration::from_secs(1), async {
        loop {
            match ws_hub.recv().await {
                Ok(data) => {
                    let msg = decode_sc_message(&data).unwrap();
                    assert_ne!(
                        msg.function,
                        ScFunction::HeartbeatAck,
                        "{context} must not leave SC answering heartbeats"
                    );
                }
                Err(_) => break,
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{context} did not close the loopback WebSocket"));

    let heartbeat = ScMessage {
        function: ScFunction::HeartbeatRequest,
        message_id: 0x66,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &heartbeat);
    assert!(
        ws_hub.send(&buf).await.is_err(),
        "{context} must reject post-drop Heartbeat-Request on the closed socket"
    );
}

async fn start_loopback_sc(
    client_vmac: Vmac,
    hub_vmac: Vmac,
) -> (ScTransport<LoopbackWebSocket>, LoopbackWebSocket) {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, client_vmac);
    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    (transport, hub_task.await.unwrap())
}

#[tokio::test]
async fn sc_drop_aborts_recv_task_and_closes_loopback_socket() {
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];
    let (transport, ws_hub) = start_loopback_sc(client_vmac, hub_vmac).await;
    let conn = transport.connection().unwrap().clone();
    let abort_handle = transport
        .recv_task
        .as_ref()
        .expect("SC start spawns recv task")
        .abort_handle();

    assert!(!abort_handle.is_finished());
    drop(transport);

    timeout(Duration::from_secs(1), async {
        while !abort_handle.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping SC transport aborts recv task");

    assert_eq!(conn.lock().await.state, ScConnectionState::Disconnected);

    assert_closed_without_post_drop_heartbeat(&ws_hub, "dropped SC transport").await;

    let (mut replacement, _replacement_hub) = start_loopback_sc(client_vmac, hub_vmac).await;
    replacement.stop().await.unwrap();
}

#[tokio::test]
async fn sc_stop_sends_disconnect_and_drop_after_stop_is_idempotent() {
    let (mut transport, ws_hub) = start_loopback_sc([0x01; 6], [0x10; 6]).await;
    let abort_handle = transport
        .recv_task
        .as_ref()
        .expect("SC start spawns recv task")
        .abort_handle();

    transport.stop().await.unwrap();

    let data = timeout(Duration::from_secs(1), ws_hub.recv())
        .await
        .expect("SC stop sends Disconnect-Request before closing socket")
        .expect("hub socket receives Disconnect-Request");
    let msg = decode_sc_message(&data).unwrap();
    assert_eq!(msg.function, ScFunction::DisconnectRequest);
    assert!(abort_handle.is_finished());

    drop(transport);
}
