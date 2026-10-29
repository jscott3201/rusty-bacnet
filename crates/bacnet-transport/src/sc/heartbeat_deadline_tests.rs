//! Heartbeat cadence and timeout measured from the last inbound BVLC (#2124).
//!
//! All tests run on tokio's paused clock at the default 1:2 interval-to-timeout
//! ratio, scaled to the smallest valid interval (3 s / 6 s). The clock only
//! moves when the test advances it or every task is idle, so each assertion
//! lands at an exact offset from the last heartbeat.

use super::data_attribute_tests::hub_accept;
use super::*;
use tokio::sync::mpsc;
use tokio::time::Instant;

const INTERVAL: Duration = Duration::from_millis(3_000);
const TIMEOUT: Duration = Duration::from_millis(6_000);

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Starts a node at 3 s / 6 s against a hub that has accepted the connection.
async fn start_node() -> (ScTransport<LoopbackWebSocket>, LoopbackWebSocket) {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6])
        .with_device_uuid([1; 16])
        .with_heartbeat_interval_ms(INTERVAL.as_millis() as u64)
        .with_heartbeat_timeout_ms(TIMEOUT.as_millis() as u64);
    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_hub, [0x10; 6]).await;
        ws_hub
    });
    let _ = transport.start().await.unwrap();
    (transport, hub_task.await.unwrap())
}

/// A hub that reports each Heartbeat-Request's arrival time and, when
/// `ack_after` is set, answers it with a Heartbeat-ACK that much later.
fn spawn_hub(
    ws_hub: LoopbackWebSocket,
    ack_after: Option<Duration>,
) -> (
    tokio::task::JoinHandle<()>,
    mpsc::UnboundedReceiver<Instant>,
) {
    let (seen_tx, seen) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        while let Ok(data) = ws_hub.recv().await {
            let msg = decode_sc_message(&data).unwrap();
            if msg.function != ScFunction::HeartbeatRequest {
                continue;
            }
            let _ = seen_tx.send(Instant::now());
            let Some(delay) = ack_after else { continue };
            tokio::time::sleep(delay).await;
            let ack = ScMessage {
                function: ScFunction::HeartbeatAck,
                message_id: msg.message_id,
                originating_vmac: None,
                destination_vmac: None,
                dest_options: Vec::new(),
                data_options: Vec::new(),
                payload: Bytes::new(),
            };
            let mut buf = BytesMut::new();
            encode_sc_message(&mut buf, &ack);
            if ws_hub.send(&buf).await.is_err() {
                return;
            }
        }
    });
    (task, seen)
}

/// Moves the paused clock to `target`, then lets every woken task run.
async fn advance_to(target: Instant) {
    let now = Instant::now();
    if target > now {
        tokio::time::advance(target - now).await;
    }
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

/// Waits (auto-advancing the paused clock) for the next heartbeat.
async fn next_heartbeat(seen: &mut mpsc::UnboundedReceiver<Instant>, what: &str) -> Instant {
    tokio::time::timeout(INTERVAL * 3, seen.recv())
        .await
        .unwrap_or_else(|_| panic!("no heartbeat: {what}"))
        .unwrap()
}

async fn state(conn: &Arc<Mutex<ScConnection>>) -> ScConnectionState {
    conn.lock().await.state
}

/// An idle node whose hub answers every heartbeat must stay connected even
/// when the ACK takes 1 ms and the node's timer fires 2 ms late, as on any
/// real host. A fixed tick that both sent and checked the timeout skipped the
/// tick after each late ACK, then sent and gave up in the same tick one
/// interval later, so 2 ms of lateness against a 1 ms round trip dropped it.
#[tokio::test(start_paused = true)]
async fn sc_idle_node_with_answering_hub_survives_late_timer_ticks() {
    let (mut transport, ws_hub) = start_node().await;
    let conn = transport.connection().unwrap().clone();
    let (hub, mut seen) = spawn_hub(ws_hub, Some(ms(1)));
    for cycle in 0..5 {
        while seen.try_recv().is_ok() {}
        let h = next_heartbeat(&mut seen, &format!("cycle {cycle}")).await;
        // Its ACK lands 1 ms later. Then any timer due in the next interval
        // is seen up to 2 ms late, and the next ACK again lands 1 ms after
        // its request, ending 2 ms past two intervals from `h`.
        for target in [
            h + ms(1),
            h + INTERVAL,
            h + INTERVAL + ms(3),
            h + INTERVAL + ms(4),
            h + INTERVAL * 2 + ms(2),
        ] {
            advance_to(target).await;
            assert_eq!(
                state(&conn).await,
                ScConnectionState::Connected,
                "idle node dropped a hub that answered every heartbeat (cycle {cycle})"
            );
        }
    }
    transport.stop().await.unwrap();
    hub.abort();
}

/// Heartbeats go out one interval after the last inbound BVLC, here the
/// previous ACK, not every second interval.
#[tokio::test(start_paused = true)]
async fn sc_idle_node_sends_a_heartbeat_every_interval() {
    let (mut transport, ws_hub) = start_node().await;
    let (hub, mut seen) = spawn_hub(ws_hub, Some(ms(1)));
    let mut previous = next_heartbeat(&mut seen, "first").await;
    for n in 1..5 {
        let at = next_heartbeat(&mut seen, &format!("heartbeat {n}")).await;
        assert_eq!(
            at - previous,
            INTERVAL + ms(1),
            "heartbeat {n} must follow the previous ACK by one interval"
        );
        previous = at;
    }
    transport.stop().await.unwrap();
    hub.abort();
}

/// An unanswered heartbeat ends the link exactly at the timeout from the
/// last inbound BVLC, not at a later tick.
#[tokio::test(start_paused = true)]
async fn sc_unanswered_heartbeat_disconnects_at_the_timeout() {
    let connected_at = Instant::now();
    let (mut transport, ws_hub) = start_node().await;
    let conn = transport.connection().unwrap().clone();
    let (hub, mut seen) = spawn_hub(ws_hub, None);
    let h = next_heartbeat(&mut seen, "first").await;
    assert_eq!(
        h - connected_at,
        INTERVAL,
        "first heartbeat after one idle interval"
    );
    advance_to(connected_at + TIMEOUT - ms(1)).await;
    assert_eq!(state(&conn).await, ScConnectionState::Connected);
    advance_to(connected_at + TIMEOUT).await;
    assert_eq!(
        state(&conn).await,
        ScConnectionState::Disconnected,
        "an unanswered heartbeat must disconnect at the timeout"
    );
    assert!(
        seen.try_recv().is_err(),
        "no second heartbeat before the disconnect"
    );
    transport.stop().await.unwrap();
    hub.abort();
}
