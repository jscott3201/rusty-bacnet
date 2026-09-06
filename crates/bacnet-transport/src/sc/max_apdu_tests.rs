use bytes::{Bytes, BytesMut};
use tokio::time::{Duration, Instant};

use crate::port::TransportPort;
use crate::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage, Vmac};

use super::{LoopbackWebSocket, ScReconnectConfig, ScTransport, WebSocketPort};

async fn accept_with_limits(
    ws_hub: &LoopbackWebSocket,
    hub_vmac: Vmac,
    hub_max_bvlc_length: u16,
    hub_max_npdu_length: u16,
) {
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);

    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&hub_vmac);
    payload.extend_from_slice(&[0u8; 16]);
    payload.extend_from_slice(&hub_max_bvlc_length.to_be_bytes());
    payload.extend_from_slice(&hub_max_npdu_length.to_be_bytes());

    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    ws_hub.send(&buf).await.unwrap();
}

async fn wait_for_transport_max_apdu_length(
    transport: &ScTransport<LoopbackWebSocket>,
    expected: u16,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        if transport.max_apdu_length() == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for max APDU {expected}, got {}",
            transport.max_apdu_length()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn sc_max_apdu_length_accounts_for_sc_and_npdu_headers() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let hub_vmac = [0x10; 6];
    let mut transport = ScTransport::new(ws_client, [0x01; 6]);

    let hub_task = tokio::spawn(async move {
        accept_with_limits(&ws_hub, hub_vmac, 1476, 1476).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();

    assert_eq!(transport.max_apdu_length(), 1464);

    transport.stop().await.unwrap();
    drop(ws_hub);
}

#[tokio::test]
async fn sc_max_apdu_length_reflects_negotiated_hub_npdu_limit() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let hub_vmac = [0x10; 6];
    let mut transport = ScTransport::new(ws_client, [0x01; 6]);
    assert_eq!(transport.max_apdu_length(), 1476);

    let hub_task = tokio::spawn(async move {
        accept_with_limits(&ws_hub, hub_vmac, 1200, 480).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();

    assert_eq!(transport.max_apdu_length(), 478);

    transport.stop().await.unwrap();
    drop(ws_hub);
}

#[tokio::test]
async fn sc_max_apdu_length_updates_after_failover_handshake() {
    let (primary_client, primary_hub) = LoopbackWebSocket::pair();
    let (failover_client, failover_hub) = LoopbackWebSocket::pair();
    let primary_hub_vmac = [0x10; 6];
    let failover_hub_vmac = [0x20; 6];
    let mut transport = ScTransport::new(primary_client, [0x01; 6])
        .with_connect_timeout_ms(100)
        .with_heartbeat_interval_ms(5_000)
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 25,
            max_delay_ms: 25,
            max_retries: 1,
        })
        .with_failover(failover_client);

    let primary_task = tokio::spawn(async move {
        accept_with_limits(&primary_hub, primary_hub_vmac, 1200, 480).await;
        primary_hub
    });

    let _rx = transport.start().await.unwrap();
    let primary_hub = primary_task.await.unwrap();
    assert_eq!(transport.max_apdu_length(), 478);

    let failover_task = tokio::spawn(async move {
        accept_with_limits(&failover_hub, failover_hub_vmac, 300, 1476).await;
        failover_hub
    });

    drop(primary_hub);
    let failover_hub = tokio::time::timeout(Duration::from_secs(2), failover_task)
        .await
        .expect("timed out waiting for failover handshake")
        .unwrap();

    wait_for_transport_max_apdu_length(&transport, 288, Duration::from_secs(1)).await;

    transport.stop().await.unwrap();
    drop(failover_hub);
}

#[tokio::test]
async fn sc_max_apdu_length_reflects_negotiated_hub_bvlc_limit() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let hub_vmac = [0x10; 6];
    let mut transport = ScTransport::new(ws_client, [0x01; 6]);

    let hub_task = tokio::spawn(async move {
        accept_with_limits(&ws_hub, hub_vmac, 300, 1476).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();

    assert_eq!(transport.max_apdu_length(), 288);

    transport.stop().await.unwrap();
    drop(ws_hub);
}
