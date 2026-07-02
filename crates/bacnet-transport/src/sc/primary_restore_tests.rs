use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

use super::*;

async fn hub_accept(ws_hub: &LoopbackWebSocket, hub_vmac: Vmac) {
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);

    send_connect_accept(ws_hub, req.message_id, hub_vmac).await;
}

async fn send_connect_accept(ws_hub: &LoopbackWebSocket, message_id: u16, hub_vmac: Vmac) {
    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&hub_vmac);
    accept_payload.extend_from_slice(&[0u8; 16]);
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());

    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(accept_payload),
    };
    send_message(ws_hub, &accept).await;
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

fn fail_random48_vmac() -> Result<Vmac, Error> {
    Err(Error::Encoding(
        "test Random-48 VMAC generation failure".into(),
    ))
}

async fn wait_for_hub_vmac(conn: &Arc<Mutex<ScConnection>>, expected: Vmac, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if conn.lock().await.hub_vmac == Some(expected) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "timed out waiting for hub VMAC {:02x?}, last state: {:?}",
        expected,
        conn.lock().await.hub_vmac
    );
}

#[tokio::test]
async fn primary_restore_duplicate_vmac_reseed_is_reused_by_next_probe() {
    let (primary_client, primary_hub) = LoopbackWebSocket::pair();
    let (failover_client, failover_hub) = LoopbackWebSocket::pair();

    let client_vmac = [0x01; 6];
    let primary_hub_vmac = [0x10; 6];
    let failover_hub_vmac = [0x20; 6];

    let mut transport = ScTransport::new(primary_client, client_vmac)
        .with_connect_timeout_ms(100)
        .with_heartbeat_interval_ms(5_000)
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 25,
            max_delay_ms: 25,
            max_retries: 1,
        })
        .with_failover(failover_client);

    let failover_task = tokio::spawn(async move {
        hub_accept(&failover_hub, failover_hub_vmac).await;
        failover_hub
    });

    let _rx = transport.start().await.unwrap();
    let _failover_hub = failover_task.await.unwrap();
    let conn = transport.connection().unwrap().clone();
    assert_eq!(conn.lock().await.hub_vmac, Some(failover_hub_vmac));

    let primary_restore_task = tokio::spawn(async move {
        let stale = primary_hub.recv().await.unwrap();
        let stale_req = decode_sc_message(&stale).unwrap();
        assert_eq!(stale_req.function, ScFunction::ConnectRequest);
        assert_eq!(&stale_req.payload[0..6], &client_vmac);

        let first = primary_hub.recv().await.unwrap();
        let first_req = decode_sc_message(&first).unwrap();
        assert_eq!(first_req.function, ScFunction::ConnectRequest);
        assert_eq!(&first_req.payload[0..6], &client_vmac);
        send_message(
            &primary_hub,
            &connect_result_nak(
                first_req.message_id,
                ErrorCode::NODE_DUPLICATE_VMAC.to_raw(),
            ),
        )
        .await;

        let second = primary_hub.recv().await.unwrap();
        let second_req = decode_sc_message(&second).unwrap();
        assert_eq!(second_req.function, ScFunction::ConnectRequest);
        let retry_vmac: Vmac = second_req.payload[0..6].try_into().unwrap();
        assert_ne!(retry_vmac, client_vmac);
        assert_eq!(retry_vmac[0] & 0x0F, 0x02);
        send_connect_accept(&primary_hub, second_req.message_id, primary_hub_vmac).await;
        retry_vmac
    });

    let retry_vmac = tokio::time::timeout(Duration::from_secs(2), primary_restore_task)
        .await
        .expect("timed out waiting for primary restore retry")
        .unwrap();

    wait_for_hub_vmac(&conn, primary_hub_vmac, Duration::from_secs(1)).await;
    assert_eq!(conn.lock().await.local_vmac, retry_vmac);

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn primary_restore_reseed_failure_blocks_stale_restore_retry() {
    let _random48_guard = set_test_random48_vmac_generator(fail_random48_vmac);
    let (primary_client, primary_hub) = LoopbackWebSocket::pair();
    let (failover_client, failover_hub) = LoopbackWebSocket::pair();

    let client_vmac = [0x01; 6];
    let failover_hub_vmac = [0x20; 6];

    let mut transport = ScTransport::new(primary_client, client_vmac)
        .with_connect_timeout_ms(100)
        .with_heartbeat_interval_ms(5_000)
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 25,
            max_delay_ms: 25,
            max_retries: 1,
        })
        .with_failover(failover_client);

    let failover_task = tokio::spawn(async move {
        hub_accept(&failover_hub, failover_hub_vmac).await;
        failover_hub
    });

    let _rx = transport.start().await.unwrap();
    let _failover_hub = failover_task.await.unwrap();
    let conn = transport.connection().unwrap().clone();
    assert_eq!(conn.lock().await.hub_vmac, Some(failover_hub_vmac));

    let primary_restore_task = tokio::spawn(async move {
        let stale = primary_hub.recv().await.unwrap();
        let stale_req = decode_sc_message(&stale).unwrap();
        assert_eq!(stale_req.function, ScFunction::ConnectRequest);
        assert_eq!(&stale_req.payload[0..6], &client_vmac);

        let first = primary_hub.recv().await.unwrap();
        let first_req = decode_sc_message(&first).unwrap();
        assert_eq!(first_req.function, ScFunction::ConnectRequest);
        assert_eq!(&first_req.payload[0..6], &client_vmac);
        send_message(
            &primary_hub,
            &connect_result_nak(
                first_req.message_id,
                ErrorCode::NODE_DUPLICATE_VMAC.to_raw(),
            ),
        )
        .await;

        match tokio::time::timeout(Duration::from_millis(200), primary_hub.recv()).await {
            Ok(Ok(data)) => panic!(
                "primary restore retried with stale VMAC after reseed failure: {:02x?}",
                data
            ),
            Ok(Err(_)) | Err(_) => {}
        }
    });

    tokio::time::timeout(Duration::from_secs(2), primary_restore_task)
        .await
        .expect("timed out waiting for primary restore reseed-failure check")
        .unwrap();

    let c = conn.lock().await;
    assert_eq!(c.hub_vmac, Some(failover_hub_vmac));
    assert_eq!(c.local_vmac, client_vmac);
    assert!(!c.connect_retry_allowed);
    drop(c);

    transport.stop().await.unwrap();
}
