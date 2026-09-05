use super::*;
use bacnet_transport::sc::{LoopbackWebSocket, ScReconnectConfig, WebSocketPort};
use bacnet_transport::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage};
use bytes::{Bytes, BytesMut};
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn sc_client_builder_sends_configured_vmac_and_device_uuid() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x22, 0x01, 0x02, 0x03, 0x04, 0x05];
    let device_uuid = [0xAB; 16];
    let hub_vmac = [0x10; 6];

    let hub_task = tokio::spawn(async move {
        let data = ws_hub.recv().await.unwrap();
        let mut expected = vec![ScFunction::ConnectRequest.to_raw(), 0x00, 0x00, 0x01];
        expected.extend_from_slice(&client_vmac);
        expected.extend_from_slice(&device_uuid);
        expected.extend_from_slice(&1476u16.to_be_bytes());
        expected.extend_from_slice(&1476u16.to_be_bytes());
        assert_eq!(data, expected);

        let req = decode_sc_message(&data).unwrap();
        assert_eq!(req.function, ScFunction::ConnectRequest);
        assert_eq!(req.message_id, 1);
        assert_eq!(req.originating_vmac, None);
        assert_eq!(req.destination_vmac, None);
        assert!(req.dest_options.is_empty());
        assert!(req.data_options.is_empty());
        assert_eq!(req.payload.len(), 26);
        assert_eq!(&req.payload[0..6], &client_vmac);
        assert_eq!(&req.payload[6..22], &device_uuid);

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
        ws_hub
    });

    let mut client = BACnetClient::sc_builder()
        .vmac(client_vmac)
        .device_uuid(device_uuid)
        .build_with_websocket_for_test(ws_client)
        .await
        .unwrap();
    let ws_hub = hub_task.await.unwrap();

    client.stop().await.unwrap();
    drop(ws_hub);
}

#[tokio::test]
async fn sc_client_builder_rejects_reserved_vmac_before_connect() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let result = BACnetClient::sc_builder()
        .vmac(bacnet_transport::sc_frame::UNKNOWN_VMAC)
        .device_uuid([0xAB; 16])
        .build_with_websocket_for_test(ws_client)
        .await;

    assert!(matches!(
        result,
        Err(Error::Encoding(message)) if message.contains("unknown VMAC")
    ));

    if let Ok(Ok(data)) = timeout(Duration::from_millis(50), ws_hub.recv()).await {
        panic!("reserved VMAC reached wire: {data:?}");
    }
}

#[tokio::test]
async fn sc_client_builder_rejects_invalid_max_segments_before_connect() {
    let result = BACnetClient::sc_builder()
        .vmac([0x22, 0x01, 0x02, 0x03, 0x04, 0x05])
        .device_uuid([0xAB; 16])
        .max_segments(Some(1))
        .build()
        .await;

    assert!(matches!(
        result,
        Err(Error::Encoding(message)) if message.contains("max-segments-accepted")
    ));
}

#[test]
fn sc_client_builder_rejects_broadcast_vmac_and_zero_device_uuid() {
    let broadcast = BACnetClient::sc_builder()
        .vmac(bacnet_transport::sc_frame::BROADCAST_VMAC)
        .device_uuid([0xAB; 16])
        .validate_identity();
    assert!(matches!(
        broadcast,
        Err(Error::Encoding(message)) if message.contains("broadcast VMAC")
    ));

    let zero_uuid = BACnetClient::sc_builder()
        .vmac([0x22, 0x01, 0x02, 0x03, 0x04, 0x05])
        .validate_identity();
    assert!(matches!(
        zero_uuid,
        Err(Error::Encoding(message)) if message.contains("device_uuid")
    ));
}

#[tokio::test]
async fn sc_client_builder_rejects_invalid_reconnect_before_tls_lookup() {
    for max_retries in [0, 10] {
        for (initial_delay_ms, max_delay_ms) in [(0, 1), (1, 0), (0, 0), (2, 1)] {
            let error = BACnetClient::sc_builder()
                .vmac([0x22; 6])
                .device_uuid([0xAB; 16])
                .reconnect(ScReconnectConfig {
                    initial_delay_ms,
                    max_delay_ms,
                    max_retries,
                })
                .build()
                .await
                .err()
                .expect("invalid reconnect must fail");
            assert!(
                matches!(&error, Error::OutOfRange(message) if message.contains("reconnect")),
                "expected reconnect error before TLS lookup, got {error:?}"
            );
        }
    }
}

#[tokio::test]
async fn sc_client_builder_valid_reconnect_preserves_missing_tls_error() {
    for reconnect in [
        None,
        Some(ScReconnectConfig::default()),
        Some(ScReconnectConfig {
            initial_delay_ms: 1,
            max_delay_ms: 1,
            max_retries: 0,
        }),
        Some(ScReconnectConfig {
            initial_delay_ms: 1,
            max_delay_ms: 2,
            max_retries: u32::MAX,
        }),
    ] {
        let mut builder = BACnetClient::sc_builder()
            .vmac([0x22; 6])
            .device_uuid([0xAB; 16]);
        if let Some(config) = reconnect {
            builder = builder.reconnect(config);
        }
        assert!(matches!(
            builder.build().await,
            Err(Error::Encoding(message)) if message == "SC client builder: tls_config is required"
        ));
    }
}

struct RejectIoWebSocket {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    drops: Arc<std::sync::atomic::AtomicUsize>,
}

impl WebSocketPort for RejectIoWebSocket {
    async fn send(&self, _: &[u8]) -> Result<(), Error> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(Error::Encoding("unexpected WebSocket send".into()))
    }

    async fn recv(&self) -> Result<Vec<u8>, Error> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(Error::Encoding("unexpected WebSocket receive".into()))
    }
}

impl Drop for RejectIoWebSocket {
    fn drop(&mut self) {
        self.drops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[tokio::test]
async fn sc_client_test_builder_reconnect_preflight_drops_input_without_io() {
    for (initial_delay_ms, max_delay_ms) in [(0, 1), (1, 0), (0, 0), (2, 1)] {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let error = BACnetClient::sc_builder()
            .vmac([0x22; 6])
            .device_uuid([0xAB; 16])
            .reconnect(ScReconnectConfig {
                initial_delay_ms,
                max_delay_ms,
                max_retries: 0,
            })
            .build_with_websocket_for_test(RejectIoWebSocket {
                calls: calls.clone(),
                drops: drops.clone(),
            })
            .await
            .err()
            .expect("invalid reconnect must fail");
        assert!(
            matches!(&error, Error::OutOfRange(message) if message.contains("reconnect")),
            "expected reconnect preflight error, got {error:?}"
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
