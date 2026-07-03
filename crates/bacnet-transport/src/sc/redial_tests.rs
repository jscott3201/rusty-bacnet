use super::*;
use bacnet_types::error::Error;
use std::future::{pending, Future};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

struct GateSendWebSocket {
    inner: LoopbackWebSocket,
    hang_send: Arc<AtomicBool>,
}

impl GateSendWebSocket {
    fn pair(client_hang_send: Arc<AtomicBool>) -> (Self, Self) {
        let (client, hub) = LoopbackWebSocket::pair();
        (
            Self {
                inner: client,
                hang_send: client_hang_send,
            },
            Self {
                inner: hub,
                hang_send: Arc::new(AtomicBool::new(false)),
            },
        )
    }
}

impl WebSocketPort for GateSendWebSocket {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        if self.hang_send.load(Ordering::SeqCst) {
            pending::<Result<(), Error>>().await
        } else {
            self.inner.send(data).await
        }
    }

    async fn recv(&self) -> Result<Vec<u8>, Error> {
        self.inner.recv().await
    }
}

fn loopback_redial_connector(
    dial_count: Arc<AtomicUsize>,
    hub_tx: tokio::sync::mpsc::UnboundedSender<LoopbackWebSocket>,
) -> impl Fn() -> Pin<Box<dyn Future<Output = Result<LoopbackWebSocket, Error>> + Send>>
       + Send
       + Sync
       + 'static {
    move || {
        let dial_count = dial_count.clone();
        let hub_tx = hub_tx.clone();
        Box::pin(async move {
            dial_count.fetch_add(1, Ordering::SeqCst);
            let (client, hub) = LoopbackWebSocket::pair();
            hub_tx
                .send(hub)
                .map_err(|_| Error::Encoding("test connector hub receiver dropped".into()))?;
            Ok(client)
        })
    }
}

fn hanging_redial_connector(
    dial_count: Arc<AtomicUsize>,
) -> impl Fn() -> Pin<Box<dyn Future<Output = Result<LoopbackWebSocket, Error>> + Send>>
       + Send
       + Sync
       + 'static {
    move || {
        let dial_count = dial_count.clone();
        Box::pin(async move {
            dial_count.fetch_add(1, Ordering::SeqCst);
            pending::<Result<LoopbackWebSocket, Error>>().await
        })
    }
}

fn loopback_first_success_then_error_connector(
    dial_count: Arc<AtomicUsize>,
    hub_tx: tokio::sync::mpsc::UnboundedSender<LoopbackWebSocket>,
) -> impl Fn() -> Pin<Box<dyn Future<Output = Result<LoopbackWebSocket, Error>> + Send>>
       + Send
       + Sync
       + 'static {
    move || {
        let dial_count = dial_count.clone();
        let hub_tx = hub_tx.clone();
        Box::pin(async move {
            let attempt = dial_count.fetch_add(1, Ordering::SeqCst) + 1;
            if attempt != 1 {
                return Err(Error::Encoding(format!(
                    "test connector rejects dial attempt {attempt}"
                )));
            }
            let (client, hub) = LoopbackWebSocket::pair();
            hub_tx
                .send(hub)
                .map_err(|_| Error::Encoding("test connector hub receiver dropped".into()))?;
            Ok(client)
        })
    }
}

fn gate_send_redial_connector(
    dial_count: Arc<AtomicUsize>,
    hub_tx: tokio::sync::mpsc::UnboundedSender<GateSendWebSocket>,
) -> impl Fn() -> Pin<Box<dyn Future<Output = Result<GateSendWebSocket, Error>> + Send>>
       + Send
       + Sync
       + 'static {
    move || {
        let dial_count = dial_count.clone();
        let hub_tx = hub_tx.clone();
        Box::pin(async move {
            dial_count.fetch_add(1, Ordering::SeqCst);
            let (client, hub) = GateSendWebSocket::pair(Arc::new(AtomicBool::new(false)));
            hub_tx
                .send(hub)
                .map_err(|_| Error::Encoding("test connector hub receiver dropped".into()))?;
            Ok(client)
        })
    }
}

fn failing_loopback_connector(
) -> impl Fn() -> Pin<Box<dyn Future<Output = Result<LoopbackWebSocket, Error>> + Send>>
       + Send
       + Sync
       + 'static {
    move || {
        Box::pin(async {
            Err(Error::Encoding(
                "test primary connector intentionally fails".into(),
            ))
        })
    }
}

async fn hub_accept<W: WebSocketPort>(ws_hub: &W, hub_vmac: Vmac) {
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

async fn wait_for_hub_vmac(conn: &Arc<Mutex<ScConnection>>, expected: Vmac, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if conn.lock().await.hub_vmac == Some(expected) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for hub VMAC {expected:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_dial_count(count: &AtomicUsize, expected: usize, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if count.load(Ordering::SeqCst) >= expected {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for dial count {expected}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_state(
    conn: &Arc<Mutex<ScConnection>>,
    expected: ScConnectionState,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if conn.lock().await.state == expected {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for SC state {expected:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn sc_reconnect_redials_fresh_websocket_after_socket_teardown() {
    let (primary_client, primary_hub) = LoopbackWebSocket::pair();
    let (redial_hub_tx, mut redial_hub_rx) =
        tokio::sync::mpsc::unbounded_channel::<LoopbackWebSocket>();
    let redial_count = Arc::new(AtomicUsize::new(0));

    let client_vmac = [0x01; 6];
    let primary_hub_vmac = [0x10; 6];
    let redial_hub_vmac = [0x11; 6];

    let mut transport = ScTransport::new(primary_client, client_vmac)
        .with_connect_timeout_ms(100)
        .with_test_heartbeat_timing_ms(5_000, 10_000)
        .with_connector(loopback_redial_connector(
            redial_count.clone(),
            redial_hub_tx,
        ))
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 10,
            max_delay_ms: 10,
            max_retries: 3,
        });

    let primary_task = tokio::spawn(async move {
        hub_accept(&primary_hub, primary_hub_vmac).await;
        primary_hub
    });

    let _rx = transport.start().await.unwrap();
    let primary_hub = primary_task.await.unwrap();
    let conn = transport.connection().unwrap().clone();
    assert_eq!(conn.lock().await.hub_vmac, Some(primary_hub_vmac));

    let redial_task = tokio::spawn(async move {
        let redial_hub = tokio::time::timeout(Duration::from_secs(1), redial_hub_rx.recv())
            .await
            .expect("timed out waiting for redial connector")
            .expect("redial connector channel closed");
        hub_accept(&redial_hub, redial_hub_vmac).await;
        redial_hub
    });

    drop(primary_hub);
    let redial_hub = tokio::time::timeout(Duration::from_secs(2), redial_task)
        .await
        .expect("timed out waiting for redial handshake")
        .unwrap();

    wait_for_hub_vmac(&conn, redial_hub_vmac, Duration::from_secs(1)).await;
    assert_eq!(redial_count.load(Ordering::SeqCst), 1);

    let payload = [0x09, 0x08, 0x07];
    let dest_vmac = [0x55; 6];
    transport.send_unicast(&payload, &dest_vmac).await.unwrap();

    let data = tokio::time::timeout(Duration::from_secs(1), redial_hub.recv())
        .await
        .expect("timed out waiting for unicast on redialed socket")
        .unwrap();
    let msg = decode_sc_message(&data).unwrap();
    assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(msg.destination_vmac, Some(dest_vmac));
    assert_eq!(msg.payload.as_ref(), payload);

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn sc_failover_connector_dials_when_failover_is_needed() {
    let (primary_client, _primary_hub) = LoopbackWebSocket::pair();
    let (failover_hub_tx, mut failover_hub_rx) =
        tokio::sync::mpsc::unbounded_channel::<LoopbackWebSocket>();
    let failover_dial_count = Arc::new(AtomicUsize::new(0));

    let client_vmac = [0x01; 6];
    let failover_hub_vmac = [0x20; 6];

    let mut transport = ScTransport::new(primary_client, client_vmac)
        .with_connect_timeout_ms(100)
        .with_failover_connector(loopback_redial_connector(
            failover_dial_count.clone(),
            failover_hub_tx,
        ));

    let failover_task = tokio::spawn(async move {
        let failover_hub = tokio::time::timeout(Duration::from_secs(1), failover_hub_rx.recv())
            .await
            .expect("timed out waiting for failover connector")
            .expect("failover connector channel closed");
        hub_accept(&failover_hub, failover_hub_vmac).await;
        failover_hub
    });

    let _rx = transport.start().await.unwrap();
    let failover_hub = failover_task.await.unwrap();
    let conn = transport.connection().unwrap().clone();
    assert_eq!(conn.lock().await.hub_vmac, Some(failover_hub_vmac));
    assert_eq!(failover_dial_count.load(Ordering::SeqCst), 1);

    let payload = [0x02, 0x04, 0x06];
    let dest_vmac = [0x66; 6];
    transport.send_unicast(&payload, &dest_vmac).await.unwrap();
    let data = tokio::time::timeout(Duration::from_secs(1), failover_hub.recv())
        .await
        .expect("timed out waiting for failover unicast")
        .unwrap();
    let msg = decode_sc_message(&data).unwrap();
    assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(msg.destination_vmac, Some(dest_vmac));
    assert_eq!(msg.payload.as_ref(), payload);

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn sc_primary_restore_connector_redials_primary_socket() {
    let (primary_client, _stale_primary_hub) = LoopbackWebSocket::pair();
    let (failover_client, failover_hub) = LoopbackWebSocket::pair();
    let (primary_hub_tx, mut primary_hub_rx) =
        tokio::sync::mpsc::unbounded_channel::<LoopbackWebSocket>();
    let primary_dial_count = Arc::new(AtomicUsize::new(0));

    let client_vmac = [0x01; 6];
    let primary_hub_vmac = [0x10; 6];
    let failover_hub_vmac = [0x20; 6];

    let mut transport = ScTransport::new(primary_client, client_vmac)
        .with_connect_timeout_ms(100)
        .with_heartbeat_interval_ms(5_000)
        .with_connector(loopback_redial_connector(
            primary_dial_count.clone(),
            primary_hub_tx,
        ))
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
    let failover_hub = failover_task.await.unwrap();
    let conn = transport.connection().unwrap().clone();
    assert_eq!(conn.lock().await.hub_vmac, Some(failover_hub_vmac));

    let primary_restore_task = tokio::spawn(async move {
        let primary_hub = tokio::time::timeout(Duration::from_secs(1), primary_hub_rx.recv())
            .await
            .expect("timed out waiting for primary restore connector")
            .expect("primary restore connector channel closed");
        hub_accept(&primary_hub, primary_hub_vmac).await;
        primary_hub
    });

    let primary_hub = tokio::time::timeout(Duration::from_secs(2), primary_restore_task)
        .await
        .expect("timed out waiting for primary restore handshake")
        .unwrap();

    wait_for_hub_vmac(&conn, primary_hub_vmac, Duration::from_secs(1)).await;
    assert_eq!(primary_dial_count.load(Ordering::SeqCst), 1);

    let failover_data = tokio::time::timeout(Duration::from_secs(1), failover_hub.recv())
        .await
        .expect("timed out waiting for failover disconnect")
        .unwrap();
    let failover_msg = decode_sc_message(&failover_data).unwrap();
    assert_eq!(failover_msg.function, ScFunction::DisconnectRequest);

    let payload = [0x03, 0x05, 0x07];
    let dest_vmac = [0x77; 6];
    transport.send_unicast(&payload, &dest_vmac).await.unwrap();
    let data = tokio::time::timeout(Duration::from_secs(1), primary_hub.recv())
        .await
        .expect("timed out waiting for primary restore unicast")
        .unwrap();
    let msg = decode_sc_message(&data).unwrap();
    assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(msg.destination_vmac, Some(dest_vmac));
    assert_eq!(msg.payload.as_ref(), payload);

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn sc_failover_connector_timeout_does_not_hang_start() {
    let (primary_client, _primary_hub) = LoopbackWebSocket::pair();
    let failover_dial_count = Arc::new(AtomicUsize::new(0));

    let mut transport = ScTransport::new(primary_client, [0x01; 6])
        .with_connect_timeout_ms(20)
        .with_failover_connector(hanging_redial_connector(failover_dial_count.clone()));

    let result = tokio::time::timeout(Duration::from_secs(1), transport.start())
        .await
        .expect("start hung behind failover connector timeout");

    assert!(result.is_err());
    assert_eq!(failover_dial_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn sc_reconnect_connector_timeout_counts_as_failed_attempt() {
    let (primary_client, primary_hub) = LoopbackWebSocket::pair();
    let redial_count = Arc::new(AtomicUsize::new(0));

    let client_vmac = [0x01; 6];
    let primary_hub_vmac = [0x10; 6];

    let mut transport = ScTransport::new(primary_client, client_vmac)
        .with_connect_timeout_ms(20)
        .with_test_heartbeat_timing_ms(5_000, 10_000)
        .with_connector(hanging_redial_connector(redial_count.clone()))
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 10,
            max_delay_ms: 10,
            max_retries: 1,
        });

    let primary_task = tokio::spawn(async move {
        hub_accept(&primary_hub, primary_hub_vmac).await;
        primary_hub
    });

    let _rx = transport.start().await.unwrap();
    let primary_hub = primary_task.await.unwrap();
    let conn = transport.connection().unwrap().clone();
    assert_eq!(conn.lock().await.hub_vmac, Some(primary_hub_vmac));

    drop(primary_hub);
    wait_for_dial_count(&redial_count, 1, Duration::from_secs(1)).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if conn.lock().await.state == ScConnectionState::Disconnected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("reconnect did not finish after connector timeout");

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn sc_primary_restore_connector_timeout_leaves_failover_send_path_active() {
    let (primary_client, _stale_primary_hub) = LoopbackWebSocket::pair();
    let (failover_client, failover_hub) = LoopbackWebSocket::pair();
    let primary_dial_count = Arc::new(AtomicUsize::new(0));

    let client_vmac = [0x01; 6];
    let failover_hub_vmac = [0x20; 6];

    let mut transport = ScTransport::new(primary_client, client_vmac)
        .with_connect_timeout_ms(50)
        .with_heartbeat_interval_ms(5_000)
        .with_connector(hanging_redial_connector(primary_dial_count.clone()))
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 10,
            max_delay_ms: 10,
            max_retries: 1,
        })
        .with_failover(failover_client);

    let failover_task = tokio::spawn(async move {
        hub_accept(&failover_hub, failover_hub_vmac).await;
        failover_hub
    });

    let _rx = transport.start().await.unwrap();
    let failover_hub = failover_task.await.unwrap();
    let conn = transport.connection().unwrap().clone();
    assert_eq!(conn.lock().await.hub_vmac, Some(failover_hub_vmac));

    wait_for_dial_count(&primary_dial_count, 1, Duration::from_secs(1)).await;

    let payload = [0x0a, 0x0b, 0x0c];
    let dest_vmac = [0x88; 6];
    transport.send_unicast(&payload, &dest_vmac).await.unwrap();
    let data = tokio::time::timeout(Duration::from_secs(1), failover_hub.recv())
        .await
        .expect("timed out waiting for failover unicast while primary restore dial hung")
        .unwrap();
    let msg = decode_sc_message(&data).unwrap();
    assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(msg.destination_vmac, Some(dest_vmac));
    assert_eq!(msg.payload.as_ref(), payload);

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn sc_failover_reconnect_exhaustion_does_not_redial_failover_again() {
    let (primary_client, primary_hub) = LoopbackWebSocket::pair();
    drop(primary_hub);

    let (failover_hub_tx, mut failover_hub_rx) =
        tokio::sync::mpsc::unbounded_channel::<LoopbackWebSocket>();
    let failover_dial_count = Arc::new(AtomicUsize::new(0));

    let client_vmac = [0x01; 6];
    let failover_hub_vmac = [0x20; 6];

    let mut transport = ScTransport::new(primary_client, client_vmac)
        .with_connect_timeout_ms(50)
        .with_test_heartbeat_timing_ms(5_000, 10_000)
        .with_connector(failing_loopback_connector())
        .with_failover_connector(loopback_first_success_then_error_connector(
            failover_dial_count.clone(),
            failover_hub_tx,
        ))
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 10,
            max_delay_ms: 10,
            max_retries: 1,
        });

    let failover_task = tokio::spawn(async move {
        let failover_hub = tokio::time::timeout(Duration::from_secs(1), failover_hub_rx.recv())
            .await
            .expect("timed out waiting for initial failover connector")
            .expect("failover connector channel closed");
        hub_accept(&failover_hub, failover_hub_vmac).await;
        failover_hub
    });

    let _rx = transport.start().await.unwrap();
    let failover_hub = failover_task.await.unwrap();
    let conn = transport.connection().unwrap().clone();
    assert_eq!(conn.lock().await.hub_vmac, Some(failover_hub_vmac));
    assert_eq!(failover_dial_count.load(Ordering::SeqCst), 1);

    drop(failover_hub);
    wait_for_dial_count(&failover_dial_count, 2, Duration::from_secs(1)).await;
    wait_for_state(
        &conn,
        ScConnectionState::Disconnected,
        Duration::from_secs(1),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(
        failover_dial_count.load(Ordering::SeqCst),
        2,
        "failover connector should not be redialed again after failover reconnect budget is exhausted"
    );

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn sc_primary_restore_publishes_before_hung_failover_disconnect_send() {
    let (primary_client, stale_primary_hub) =
        GateSendWebSocket::pair(Arc::new(AtomicBool::new(false)));
    drop(stale_primary_hub);
    let failover_hang_send = Arc::new(AtomicBool::new(false));
    let (failover_client, failover_hub) = GateSendWebSocket::pair(failover_hang_send.clone());
    let (primary_hub_tx, mut primary_hub_rx) =
        tokio::sync::mpsc::unbounded_channel::<GateSendWebSocket>();
    let primary_dial_count = Arc::new(AtomicUsize::new(0));

    let client_vmac = [0x01; 6];
    let primary_hub_vmac = [0x10; 6];
    let failover_hub_vmac = [0x20; 6];

    let mut transport = ScTransport::new(primary_client, client_vmac)
        .with_connect_timeout_ms(750)
        .with_heartbeat_interval_ms(5_000)
        .with_connector(gate_send_redial_connector(
            primary_dial_count.clone(),
            primary_hub_tx,
        ))
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 10,
            max_delay_ms: 10,
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

    failover_hang_send.store(true, Ordering::SeqCst);
    let primary_restore_task = tokio::spawn(async move {
        let primary_hub = tokio::time::timeout(Duration::from_secs(1), primary_hub_rx.recv())
            .await
            .expect("timed out waiting for primary restore connector")
            .expect("primary restore connector channel closed");
        hub_accept(&primary_hub, primary_hub_vmac).await;
        primary_hub
    });

    let primary_hub = tokio::time::timeout(Duration::from_secs(2), primary_restore_task)
        .await
        .expect("timed out waiting for primary restore handshake")
        .unwrap();

    wait_for_hub_vmac(&conn, primary_hub_vmac, Duration::from_millis(250)).await;
    assert_eq!(primary_dial_count.load(Ordering::SeqCst), 1);

    let payload = [0x0d, 0x0e, 0x0f];
    let dest_vmac = [0x99; 6];
    transport.send_unicast(&payload, &dest_vmac).await.unwrap();
    let data = tokio::time::timeout(Duration::from_secs(1), primary_hub.recv())
        .await
        .expect("timed out waiting for primary unicast while failover disconnect send hung")
        .unwrap();
    let msg = decode_sc_message(&data).unwrap();
    assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(msg.destination_vmac, Some(dest_vmac));
    assert_eq!(msg.payload.as_ref(), payload);

    transport.stop().await.unwrap();
}
