use super::heartbeat::sweep;
use super::heartbeat_test_support::*;
use super::*;
use crate::sc_frame::heartbeat_test_support::invalid_heartbeats;
use std::sync::atomic::AtomicU16;
use std::time::Duration;
use tokio::time::timeout;

async fn send_raw(live: &mut LiveClient, wire: &[u8]) {
    live.ws
        .send(Message::Binary(wire.to_vec().into()))
        .await
        .unwrap();
}

async fn recv_raw(live: &mut LiveClient) -> Vec<u8> {
    match timeout(Duration::from_secs(2), live.ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    {
        Message::Binary(data) => data.to_vec(),
        other => panic!("expected binary frame, got {other:?}"),
    }
}

#[tokio::test]
async fn heartbeat_request_payload_gets_nak_before_ack() {
    let mut live = LiveClient::connect(clients(), [0x22; 6]).await;
    send_raw(&mut live, &[0x0A, 0, 0x22, 0x33, 0x42]).await;
    assert_eq!(
        recv_raw(&mut live).await,
        [0, 0, 0x22, 0x33, 0x0A, 1, 0, 0, 7, 0, 7]
    );
}

async fn rejection_barrier(live: &mut LiveClient) {
    send_raw(live, &[0x0A, 0, 0x33, 0x44, 0x42]).await;
    assert_eq!(
        recv_raw(live).await,
        [0, 0, 0x33, 0x44, 0x0A, 1, 0, 0, 7, 0, 7]
    );
}

#[tokio::test]
async fn heartbeat_invalid_ack_preserves_probe_and_activity() {
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    live.idle().await;
    sweep(
        &clients,
        &AtomicU16::new(0x2233),
        &ClockIo(AtomicU64::new(100)),
    )
    .await;
    assert_eq!(recv_raw(&mut live).await, [0x0A, 0, 0x22, 0x33]);
    let before = clients.lock().await.get(&live.vmac).unwrap().heartbeat;
    send_raw(&mut live, &[0x0B, 2, 0x22, 0x33, 0x5E]).await;
    rejection_barrier(&mut live).await;
    {
        let map = clients.lock().await;
        let client = map.get(&live.vmac).unwrap();
        assert_eq!(
            client.heartbeat, before,
            "invalid ACK cleared the pending probe"
        );
        assert_eq!(
            client.last_activity.load(Ordering::Acquire),
            0,
            "invalid ACK/barrier refreshed activity"
        );
    }
    live.ack(0x2233).await;
    let map = clients.lock().await;
    let client = map.get(&live.vmac).unwrap();
    assert!(client.heartbeat.pending.is_none());
    assert_eq!(client.heartbeat.generation, before.generation);
    assert!(client.last_activity.load(Ordering::Acquire) > 0);
}

#[tokio::test]
async fn heartbeat_envelopes_obey_nak_precedence_addressing_and_silence() {
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    live.idle().await;
    sweep(
        &clients,
        &AtomicU16::new(0x2233),
        &ClockIo(AtomicU64::new(100)),
    )
    .await;
    assert_eq!(recv_raw(&mut live).await, [0x0A, 0, 0x22, 0x33]);
    let before = clients.lock().await.get(&live.vmac).unwrap().heartbeat;
    for case in invalid_heartbeats([0x10; 6]) {
        assert!(decode_sc_message(&case.wire).is_ok(), "{}", case.name);
        send_raw(&mut live, &case.wire).await;
        if let Some(nak) = case.nak {
            assert_eq!(recv_raw(&mut live).await, nak, "{}", case.name);
        }
        let mut ack = case.wire;
        ack[0] = 0x0B; // fixture ID matches the pending probe
        send_raw(&mut live, &ack).await;
        rejection_barrier(&mut live).await;
        let map = clients.lock().await;
        let client = map.get(&live.vmac).unwrap();
        assert_eq!(client.heartbeat, before, "{}", case.name);
        assert_eq!(
            client.last_activity.load(Ordering::Acquire),
            0,
            "{}",
            case.name
        );
        drop(map);
        assert!(
            timeout(Duration::from_millis(10), live.ws.next())
                .await
                .is_err(),
            "{} caused a response loop",
            case.name
        );
    }
    send_raw(&mut live, &[0x0B, 2, 0x22, 0x33, 0x9E, 0x1E]).await;
    timeout(Duration::from_secs(2), live.ack_observed.notified())
        .await
        .unwrap();
    assert!(clients
        .lock()
        .await
        .get(&live.vmac)
        .unwrap()
        .heartbeat
        .pending
        .is_none());
    send_raw(&mut live, &[0x0A, 2, 0x44, 0x55, 0x9E, 0x1E]).await;
    assert_eq!(recv_raw(&mut live).await, [0x0B, 0, 0x44, 0x55]);
}

#[tokio::test]
async fn heartbeat_websocket_control_and_bad_binary_are_not_activity() {
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    for message in [
        Message::Ping(vec![0x42].into()),
        Message::Pong(vec![0x42].into()),
        Message::Binary(vec![0x0A, 0, 0].into()),
        Message::Binary(vec![0x42; usize::from(HUB_MAX_BVLC_LENGTH) + 1].into()),
    ] {
        live.idle().await;
        let expects_pong = matches!(message, Message::Ping(_));
        live.ws.send(message).await.unwrap();
        send_raw(&mut live, &[0x0A, 0, 0x33, 0x44, 0x42]).await;
        if expects_pong {
            assert_eq!(
                timeout(Duration::from_secs(2), live.ws.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap(),
                Message::Pong(vec![0x42].into())
            );
        }
        assert_eq!(
            recv_raw(&mut live).await,
            [0, 0, 0x33, 0x44, 0x0A, 1, 0, 0, 7, 0, 7]
        );
        assert_eq!(
            clients
                .lock()
                .await
                .get(&live.vmac)
                .unwrap()
                .last_activity
                .load(Ordering::Acquire),
            0
        );
    }
    send_raw(&mut live, &[0x0A, 0, 0x44, 0x55]).await;
    assert_eq!(recv_raw(&mut live).await, [0x0B, 0, 0x44, 0x55]);
    assert!(
        clients
            .lock()
            .await
            .get(&live.vmac)
            .unwrap()
            .last_activity
            .load(Ordering::Acquire)
            > 0
    );
}
