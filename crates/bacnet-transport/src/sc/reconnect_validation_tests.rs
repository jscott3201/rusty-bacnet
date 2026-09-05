use super::*;
use std::sync::atomic::AtomicUsize;

#[derive(Default)]
struct SocketCounts {
    sends: AtomicUsize,
    receives: AtomicUsize,
    drops: AtomicUsize,
}

struct CountingWebSocket {
    inner: LoopbackWebSocket,
    counts: Arc<SocketCounts>,
}

impl WebSocketPort for CountingWebSocket {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        self.counts.sends.fetch_add(1, Ordering::SeqCst);
        self.inner.send(data).await
    }

    async fn recv(&self) -> Result<Vec<u8>, Error> {
        self.counts.receives.fetch_add(1, Ordering::SeqCst);
        self.inner.recv().await
    }
}

impl Drop for CountingWebSocket {
    fn drop(&mut self) {
        self.counts.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn invalid_reconnect_start_preserves_transport_and_socket_for_repair() {
    for heartbeat_mode in ["default", "invalid", "test bypass"] {
        check_invalid_start_and_repair(heartbeat_mode).await;
    }
}

async fn check_invalid_start_and_repair(heartbeat_mode: &str) {
    let (client, hub) = LoopbackWebSocket::pair();
    let (failover, _failover_hub) = LoopbackWebSocket::pair();
    let counts = Arc::new(SocketCounts::default());
    let dials = Arc::new(AtomicUsize::new(0));
    let ws = CountingWebSocket {
        inner: client,
        counts: counts.clone(),
    };
    let vmac = [0x22; 6];
    let uuid = [0xAB; 16];
    let mut transport = ScTransport::new(ws, vmac)
        .with_device_uuid(uuid)
        .with_connect_timeout_ms(10)
        .with_failover(CountingWebSocket {
            inner: failover,
            counts: counts.clone(),
        })
        .with_connector({
            let dials = dials.clone();
            move || {
                dials.fetch_add(1, Ordering::SeqCst);
                async { Err(Error::Encoding("unexpected primary dial".into())) }
            }
        })
        .with_failover_connector({
            let dials = dials.clone();
            move || {
                dials.fetch_add(1, Ordering::SeqCst);
                async { Err(Error::Encoding("unexpected failover dial".into())) }
            }
        });
    transport = match heartbeat_mode {
        "invalid" => transport.with_heartbeat_interval_ms(0),
        "test bypass" => transport.with_test_heartbeat_timing_ms(1, 2),
        _ => transport,
    };
    let state = transport.connection_state_changes();

    for max_retries in [0, 10, u32::MAX] {
        for (initial_delay_ms, max_delay_ms) in [(0, 1), (1, 0), (0, 0), (2, 1)] {
            transport = transport.with_reconnect(ScReconnectConfig {
                initial_delay_ms,
                max_delay_ms,
                max_retries,
            });
            for _ in 0..2 {
                let error = transport.start().await.unwrap_err();
                assert!(
                    matches!(&error, Error::OutOfRange(message) if message.contains("reconnect")),
                    "expected reconnect configuration error, got {error:?}"
                );
                tokio::task::yield_now().await;
                assert_eq!(counts.sends.load(Ordering::SeqCst), 0);
                assert_eq!(counts.receives.load(Ordering::SeqCst), 0);
                assert_eq!(counts.drops.load(Ordering::SeqCst), 0);
                assert_eq!(dials.load(Ordering::SeqCst), 0);
                assert!(!state.has_changed().unwrap());
                assert_eq!(*state.borrow(), ScConnectionState::Disconnected);
                assert!(transport.connection().is_none());
                assert!(transport.ws.is_some());
                assert!(transport.failover_ws.is_some());
                assert!(transport.ws_shared.is_none());
                assert!(transport.recv_task.is_none());
                assert!(transport.restore_disconnect_task.lock().unwrap().is_none());
                assert_eq!(transport.local_mac(), vmac);
                assert_eq!(transport.device_uuid, uuid);
                assert_eq!(transport.max_apdu_length(), DEFAULT_MAX_APDU_LENGTH);
            }
        }
    }

    transport = transport
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 1,
            max_delay_ms: 1,
            max_retries: 0,
        })
        .with_heartbeat_interval_ms(30_000)
        .with_heartbeat_timeout_ms(60_000);
    let (started, ()) = tokio::join!(transport.start(), hub_accept(&hub, [0x10; 6]));
    let _rx = started.unwrap();
    assert_eq!(*state.borrow(), ScConnectionState::Connected);
    assert_eq!(counts.sends.load(Ordering::SeqCst), 1);
    assert_eq!(dials.load(Ordering::SeqCst), 0);
    assert_eq!(transport.local_mac(), vmac);
    assert_eq!(
        transport.connection().unwrap().lock().await.device_uuid,
        uuid
    );
    transport.stop().await.unwrap();
    assert_eq!(counts.drops.load(Ordering::SeqCst), 2);
    assert!(transport.recv_task.is_none());
    assert!(transport.restore_disconnect_task.lock().unwrap().is_none());
}

async fn hub_accept(hub: &LoopbackWebSocket, vmac: Vmac) {
    let request = tokio::time::timeout(Duration::from_secs(1), hub.recv())
        .await
        .unwrap()
        .unwrap();
    let request = decode_sc_message(&request).unwrap();
    assert_eq!(request.function, ScFunction::ConnectRequest);
    let mut payload = Vec::from(vmac);
    payload.extend_from_slice(&[0; 16]);
    payload.extend_from_slice(&1476u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());
    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: request.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    hub.send(&buf).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn zero_retries_skips_active_hub_retry_but_allows_initial_failover_and_restoration() {
    let (primary, primary_hub) = LoopbackWebSocket::pair();
    let (failover, failover_hub) = LoopbackWebSocket::pair();
    let dials = Arc::new(AtomicUsize::new(0));
    let (hub_tx, mut hub_rx) = mpsc::unbounded_channel();
    let mut transport = ScTransport::new(primary, [0x22; 6])
        .with_failover(failover)
        .with_connect_timeout_ms(500)
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 1_000,
            max_delay_ms: 1_000,
            max_retries: 0,
        })
        .with_connector({
            let dials = dials.clone();
            move || {
                dials.fetch_add(1, Ordering::SeqCst);
                let (client, hub) = LoopbackWebSocket::pair();
                hub_tx.send(hub).unwrap();
                async { Ok(client) }
            }
        });

    let (started, ()) = tokio::join!(transport.start(), hub_accept(&primary_hub, [0x10; 6]));
    let _rx = started.unwrap();
    wait_for_hub(&transport, [0x10; 6]).await;
    assert_eq!(dials.load(Ordering::SeqCst), 0);
    tokio::task::yield_now().await;

    // Losing the active hub skips its retry loop, but still permits failover.
    drop(primary_hub);
    hub_accept(&failover_hub, [0x20; 6]).await;
    wait_for_hub(&transport, [0x20; 6]).await;
    assert_eq!(dials.load(Ordering::SeqCst), 0);
    assert!(hub_rx.try_recv().is_err());

    // Primary restoration uses the reconnect timeout even with zero retries.
    tokio::time::advance(Duration::from_millis(999)).await;
    tokio::task::yield_now().await;
    assert_eq!(dials.load(Ordering::SeqCst), 0);
    tokio::time::advance(Duration::from_millis(1)).await;
    let restored_hub = tokio::time::timeout(Duration::from_secs(1), hub_rx.recv())
        .await
        .unwrap()
        .unwrap();
    hub_accept(&restored_hub, [0x10; 6]).await;
    wait_for_hub(&transport, [0x10; 6]).await;
    assert_eq!(dials.load(Ordering::SeqCst), 1);

    let disconnect = tokio::time::timeout(Duration::from_secs(1), failover_hub.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        decode_sc_message(&disconnect).unwrap().function,
        ScFunction::DisconnectRequest
    );
    transport
        .send_unicast(&[1, 2, 3], &[0x33; 6])
        .await
        .unwrap();
    let data = tokio::time::timeout(Duration::from_secs(1), restored_hub.recv())
        .await
        .unwrap()
        .unwrap();
    let message = decode_sc_message(&data).unwrap();
    assert_eq!(message.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(message.destination_vmac, Some([0x33; 6]));
    assert_eq!(message.payload.as_ref(), [1, 2, 3]);
    transport.stop().await.unwrap();
    assert!(transport.recv_task.is_none());
    assert!(transport.restore_disconnect_task.lock().unwrap().is_none());
}

async fn wait_for_hub<W: WebSocketPort>(transport: &ScTransport<W>, vmac: Vmac) {
    let mut state = transport.connection_state_changes();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            state.borrow_and_update();
            let conn = transport.connection().unwrap().lock().await;
            if conn.state == ScConnectionState::Connected && conn.hub_vmac == Some(vmac) {
                return;
            }
            drop(conn);
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
