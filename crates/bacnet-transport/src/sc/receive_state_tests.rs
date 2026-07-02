use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

async fn wait_for_connection_state(
    conn: &Arc<Mutex<ScConnection>>,
    expected: ScConnectionState,
) -> bool {
    for _ in 0..20 {
        if conn.lock().await.state == expected {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

struct HeartbeatSendFailureWebSocket {
    hub_vmac: Vmac,
    send_count: AtomicUsize,
    connect_request_id: Mutex<Option<u16>>,
    accept_sent: AtomicBool,
}

impl HeartbeatSendFailureWebSocket {
    fn new(hub_vmac: Vmac) -> Self {
        Self {
            hub_vmac,
            send_count: AtomicUsize::new(0),
            connect_request_id: Mutex::new(None),
            accept_sent: AtomicBool::new(false),
        }
    }
}

impl WebSocketPort for HeartbeatSendFailureWebSocket {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        let send_count = self.send_count.fetch_add(1, Ordering::SeqCst);
        if send_count == 0 {
            let request = decode_sc_message(data)?;
            assert_eq!(request.function, ScFunction::ConnectRequest);
            *self.connect_request_id.lock().await = Some(request.message_id);
            Ok(())
        } else {
            Err(Error::Encoding("scripted heartbeat send failure".into()))
        }
    }

    async fn recv(&self) -> Result<Vec<u8>, Error> {
        if !self.accept_sent.swap(true, Ordering::SeqCst) {
            let message_id = (*self.connect_request_id.lock().await)
                .expect("connect request must be sent before accept");
            let mut payload = Vec::with_capacity(26);
            payload.extend_from_slice(&self.hub_vmac);
            payload.extend_from_slice(&[0u8; 16]);
            payload.extend_from_slice(&1476u16.to_be_bytes());
            payload.extend_from_slice(&1476u16.to_be_bytes());
            let accept = ScMessage {
                function: ScFunction::ConnectAccept,
                message_id,
                originating_vmac: None,
                destination_vmac: None,
                dest_options: Vec::new(),
                data_options: Vec::new(),
                payload: Bytes::from(payload),
            };
            let mut buf = BytesMut::new();
            encode_sc_message(&mut buf, &accept);
            return Ok(buf.to_vec());
        }

        std::future::pending().await
    }
}

#[tokio::test]
async fn sc_recv_error_transitions_to_disconnected_without_reconnect() {
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

    drop(ws_hub);

    assert!(wait_for_connection_state(&conn, ScConnectionState::Disconnected).await);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn sc_heartbeat_send_error_transitions_to_disconnected() {
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];
    let ws = HeartbeatSendFailureWebSocket::new(hub_vmac);
    let mut transport = ScTransport::new(ws, client_vmac).with_test_heartbeat_timing_ms(50, 5000);

    let _rx = transport.start().await.unwrap();
    let conn = transport.connection().unwrap().clone();

    assert!(wait_for_connection_state(&conn, ScConnectionState::Disconnected).await);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn sc_receive_loop_discards_frame_above_local_max_bvlc() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];
    let mut transport =
        ScTransport::new(ws_client, client_vmac).with_test_heartbeat_timing_ms(500, 5000);

    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let mut rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();
    let oversized = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 7,
        originating_vmac: Some(hub_vmac),
        destination_vmac: Some(client_vmac),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(vec![0x42; 1461]),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &oversized);
    assert!(buf.len() > 1476);

    ws_hub.send(&buf).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(50), rx.recv())
        .await
        .is_err());
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn sc_reconnect_exhaustion_leaves_connection_disconnected() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];

    let mut transport = ScTransport::new(ws_client, client_vmac)
        .with_test_heartbeat_timing_ms(500, 5000)
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 10,
            max_delay_ms: 10,
            max_retries: 1,
        });

    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();
    let conn = transport.connection().unwrap().clone();

    drop(ws_hub);
    tokio::time::sleep(Duration::from_millis(150)).await;

    assert_eq!(conn.lock().await.state, ScConnectionState::Disconnected);
    transport.stop().await.unwrap();
}
