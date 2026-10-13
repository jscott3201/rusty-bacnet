use bytes::{Bytes, BytesMut};
use tokio::time::{Duration, Instant};

use crate::port::TransportPort;
use crate::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage, Vmac};

use super::{LoopbackWebSocket, ScReconnectConfig, ScTransport, WebSocketPort};

#[tokio::test]
async fn sc_local_receive_accepts_plain_npdu_1478_before_marker() {
    let (node, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(node, [1; 6]).with_device_uuid([1; 16]);
    let handshake = tokio::spawn(async move {
        accept_with_limits(&hub, [0x10; 6], 5705, 1497).await;
        hub
    });
    let mut incoming = transport.start().await.unwrap();
    let hub = handshake.await.unwrap();
    for payload in [vec![0x55; 1478], vec![0x77; 1479], vec![0x66; 3]] {
        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 9,
            originating_vmac: Some([2; 6]),
            destination_vmac: None,
            dest_options: vec![],
            data_options: vec![],
            payload: Bytes::from(payload),
        };
        let mut wire = BytesMut::new();
        encode_sc_message(&mut wire, &msg);
        hub.send(&wire).await.unwrap();
    }
    let received = tokio::time::timeout(Duration::from_secs(2), incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        received.npdu.len(),
        1478,
        "APDU1476 plus plain NPDU2 must fit local receive"
    );
    let marker = tokio::time::timeout(Duration::from_secs(2), incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        marker.npdu.as_ref(),
        &[0x66; 3],
        "1479 must be rejected before same-path marker"
    );
    assert_eq!(transport.local_receive_apdu_capacity(), 1476);
    transport.stop().await.unwrap();
}

async fn accept_with_limits(
    ws_hub: &LoopbackWebSocket,
    hub_vmac: Vmac,
    hub_max_bvlc_length: u16,
    hub_max_npdu_length: u16,
) {
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);
    assert_eq!(req.payload.len(), 26);
    assert_eq!(u16::from_be_bytes([req.payload[22], req.payload[23]]), 5705);
    assert_eq!(u16::from_be_bytes([req.payload[24], req.payload[25]]), 1478);

    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&hub_vmac);
    payload.extend_from_slice(&[0x33; 16]);
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
    assert_eq!(transport.local_receive_apdu_capacity(), 1476);
    let deadline = Instant::now() + timeout;
    loop {
        if transport.egress_apdu_limit() == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for max APDU {expected}, got {}",
            transport.egress_apdu_limit()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn sc_max_apdu_length_accounts_for_sc_and_npdu_headers() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let hub_vmac = [0x10; 6];
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_device_uuid([1; 16]);

    let hub_task = tokio::spawn(async move {
        accept_with_limits(&ws_hub, hub_vmac, 1476, 1476).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();

    assert_eq!(transport.egress_apdu_limit(), 1464);

    transport.stop().await.unwrap();
    drop(ws_hub);
}

#[tokio::test]
async fn sc_max_apdu_length_reflects_negotiated_hub_npdu_limit() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let hub_vmac = [0x10; 6];
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_device_uuid([1; 16]);
    assert_eq!(transport.egress_apdu_limit(), 1476);

    let hub_task = tokio::spawn(async move {
        accept_with_limits(&ws_hub, hub_vmac, 1200, 480).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();

    assert_eq!(transport.egress_apdu_limit(), 478);

    transport.stop().await.unwrap();
    drop(ws_hub);
}

/// What a router reads from the port once it has started (#1665): the
/// hub connection's Max-NPDU-Length, within its Max-BVLC-Length less the
/// encapsulation, both ways; past 1497 octets when the hub carries more.
/// Before start, the default.
#[tokio::test]
async fn sc_reports_its_hub_connections_npdu_limit() {
    use crate::port::NpduLimits;
    for (bvlc, npdu, limit) in [(1200, 480, 480), (1476, 1476, 1466), (65535, 61327, 61327)] {
        let (ws_client, ws_hub) = LoopbackWebSocket::pair();
        let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_device_uuid([1; 16]);
        assert_eq!(transport.max_npdu(), NpduLimits::STANDARD);
        let hub_task = tokio::spawn(async move {
            accept_with_limits(&ws_hub, [0x10; 6], bvlc, npdu).await;
            ws_hub
        });
        let _rx = transport.start().await.unwrap();
        let ws_hub = hub_task.await.unwrap();
        assert_eq!(
            transport.max_npdu(),
            NpduLimits::both(limit),
            "{bvlc}/{npdu}"
        );
        transport.stop().await.unwrap();
        drop(ws_hub);
    }
}

#[tokio::test]
async fn sc_max_apdu_length_updates_after_failover_handshake() {
    let (primary_client, primary_hub) = LoopbackWebSocket::pair();
    let (failover_client, failover_hub) = LoopbackWebSocket::pair();
    let primary_hub_vmac = [0x10; 6];
    let failover_hub_vmac = [0x20; 6];
    let mut transport = ScTransport::new(primary_client, [0x01; 6])
        .with_device_uuid([1; 16])
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
    assert_eq!(transport.egress_apdu_limit(), 478);

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
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_device_uuid([1; 16]);

    let hub_task = tokio::spawn(async move {
        accept_with_limits(&ws_hub, hub_vmac, 300, 1476).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();

    assert_eq!(transport.egress_apdu_limit(), 288);

    transport.stop().await.unwrap();
    drop(ws_hub);
}
