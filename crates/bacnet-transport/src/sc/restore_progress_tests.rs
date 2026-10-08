//! Primary restore must leave the established failover connection responsive.

use super::test_waits::until;
use super::*;

const CONNECT_MS: u64 = 500;
const FAILOVER: Vmac = [0x20; 6];

async fn send(ws: &LoopbackWebSocket, message: &ScMessage) {
    let mut bytes = BytesMut::new();
    encode_sc_message(&mut bytes, message);
    ws.send(&bytes).await.unwrap();
}

async fn accept(ws: &LoopbackWebSocket, hub: Vmac) {
    let request = decode_sc_message(&ws.recv().await.unwrap()).unwrap();
    assert_eq!(request.function, ScFunction::ConnectRequest);
    let mut payload = Vec::from(hub);
    payload.extend_from_slice(&[3; 16]);
    payload.extend_from_slice(&1476u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());
    send(
        ws,
        &ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: request.message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: vec![],
            data_options: vec![],
            payload: payload.into(),
        },
    )
    .await;
}

struct Fixture {
    transport: ScTransport<LoopbackWebSocket>,
    npdus: mpsc::Receiver<ReceivedNpdu>,
    failover: LoopbackWebSocket,
    candidates: mpsc::UnboundedReceiver<LoopbackWebSocket>,
    releases: mpsc::UnboundedReceiver<tokio::sync::oneshot::Sender<()>>,
}

impl Fixture {
    async fn start(hang_dial: bool) -> Self {
        let (primary, stale) = LoopbackWebSocket::pair();
        drop(stale);
        let (failover, hub) = LoopbackWebSocket::pair();
        let (candidates_tx, candidates) = mpsc::unbounded_channel();
        let (releases_tx, releases) = mpsc::unbounded_channel();
        let mut transport = ScTransport::new(primary, [1; 6])
            .with_device_uuid([1; 16])
            .with_connect_timeout_ms(CONNECT_MS)
            .with_test_heartbeat_timing_ms(20, 60)
            .with_reconnect(ScReconnectConfig {
                initial_delay_ms: 10,
                max_delay_ms: 10,
                max_retries: 1,
            })
            .with_connector(move || {
                let candidates = candidates_tx.clone();
                let releases = releases_tx.clone();
                async move {
                    let (client, hub) = LoopbackWebSocket::pair();
                    candidates.send(hub).unwrap();
                    if hang_dial {
                        let (release, wait) = tokio::sync::oneshot::channel();
                        releases.send(release).unwrap();
                        let _ = wait.await;
                    }
                    Ok(client)
                }
            })
            .with_failover(failover);
        let accepted = tokio::spawn(async move {
            accept(&hub, FAILOVER).await;
            hub
        });
        let npdus = transport.start().await.unwrap();
        Self {
            transport,
            npdus,
            failover: accepted.await.unwrap(),
            candidates,
            releases,
        }
    }
}

async fn assert_receive_progress(hang_dial: bool) {
    let mut f = Fixture::start(hang_dial).await;
    let _candidate = f.candidates.recv().await.unwrap();
    let started = tokio::time::Instant::now();
    // A permitted hub relay must reach the application before the candidate
    // times out. Its VMAC is the leaf origin, not the authenticated hub.
    let npdu = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 77,
        originating_vmac: Some([0x30; 6]),
        destination_vmac: None,
        dest_options: vec![],
        data_options: vec![],
        payload: Bytes::from_static(&[1, 0, 0x10]),
    };
    send(&f.failover, &npdu).await;
    let received = tokio::time::timeout(Duration::from_millis(5), f.npdus.recv())
        .await
        .expect("failover NPDU stalled behind primary restore")
        .unwrap();
    assert_eq!(received.npdu, npdu.payload);
    // The initiating node keeps issuing heartbeats and consuming matching
    // ACKs for longer than its timeout, all within the pending restore dial.
    for _ in 0..5 {
        let data = tokio::time::timeout(Duration::from_millis(40), f.failover.recv())
            .await
            .expect("failover heartbeat stalled behind primary restore")
            .unwrap();
        let request = decode_sc_message(&data).unwrap();
        assert_eq!(request.function, ScFunction::HeartbeatRequest);
        send(
            &f.failover,
            &ScMessage {
                function: ScFunction::HeartbeatAck,
                ..request
            },
        )
        .await;
    }
    assert!(started.elapsed() > Duration::from_millis(60));
    assert!(started.elapsed() < Duration::from_millis(CONNECT_MS));
    assert!(
        f.candidates.try_recv().is_err(),
        "overlapping restore attempts"
    );
    let c = f.transport.connection().unwrap().lock().await;
    assert_eq!(c.state, ScConnectionState::Connected);
    assert_eq!(c.hub_vmac, Some(FAILOVER));
    drop(c);
    f.transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn primary_restore_pending_dial_keeps_failover_receiving() {
    assert_receive_progress(true).await;
}

#[tokio::test(start_paused = true)]
async fn primary_restore_pending_handshake_keeps_failover_receiving() {
    assert_receive_progress(false).await;
}

async fn assert_shutdown_cancels_dial(drop_transport: bool) {
    let mut f = Fixture::start(true).await;
    let _candidate = f.candidates.recv().await.unwrap();
    let release = f.releases.recv().await.unwrap();
    let connection = f.transport.connection().unwrap().clone();
    let now = tokio::time::Instant::now();
    if drop_transport {
        drop(f.transport);
    } else {
        f.transport.stop().await.unwrap();
    }
    until("restore dial dropped on shutdown", || async {
        release.is_closed()
    })
    .await;
    assert_eq!(tokio::time::Instant::now(), now);
    assert!(
        release.send(()).is_err(),
        "canceled dial could complete late"
    );
    let c = connection.lock().await;
    assert_eq!(c.state, ScConnectionState::Disconnected);
    assert_eq!(c.hub_vmac, Some(FAILOVER));
}

#[tokio::test(start_paused = true)]
async fn primary_restore_stop_cancels_pending_dial() {
    assert_shutdown_cancels_dial(false).await;
}

#[tokio::test(start_paused = true)]
async fn primary_restore_drop_cancels_pending_dial() {
    assert_shutdown_cancels_dial(true).await;
}

#[tokio::test(start_paused = true)]
async fn primary_restore_failover_loss_cancels_pending_handshake() {
    let mut f = Fixture::start(false).await;
    let candidate = f.candidates.recv().await.unwrap();
    let request = decode_sc_message(&candidate.recv().await.unwrap()).unwrap();
    assert_eq!(request.function, ScFunction::ConnectRequest);
    let now = tokio::time::Instant::now();
    drop(f.failover);
    // The candidate's client socket is owned only by the pending handshake.
    // Losing failover must drop it before reconnect, not at its 500 ms timeout.
    let closed = tokio::time::timeout(Duration::from_millis(5), candidate.recv())
        .await
        .expect("stale primary handshake survived failover loss");
    assert!(closed.is_err());
    assert_eq!(tokio::time::Instant::now(), now);
    let c = f.transport.connection().unwrap().lock().await;
    assert_eq!(c.state, ScConnectionState::Disconnected);
    assert_eq!(c.hub_vmac, Some(FAILOVER));
    drop(c);
    f.transport.stop().await.unwrap();
}
