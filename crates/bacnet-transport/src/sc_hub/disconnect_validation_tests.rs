use super::heartbeat::sweep;
use super::heartbeat_test_support::*;
use super::*;
use crate::sc_frame::heartbeat_test_support::invalid_disconnects;
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
async fn disconnect_request_payload_gets_nak_before_state_change() {
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    send_raw(&mut live, &[0x08, 0, 0x22, 0x33, 0x42]).await;
    assert_eq!(
        recv_raw(&mut live).await,
        [0, 0, 0x22, 0x33, 8, 1, 0, 0, 7, 0, 7]
    );
    assert!(clients.lock().await.contains_key(&live.vmac));
}

async fn rejection_barrier(live: &mut LiveClient) {
    send_raw(live, &[8, 0, 0x33, 0x44, 0x42]).await;
    assert_eq!(
        recv_raw(live).await,
        [0, 0, 0x33, 0x44, 8, 1, 0, 0, 7, 0, 7]
    );
}

#[tokio::test]
async fn disconnect_envelopes_preserve_hub_registration_activity_and_probe() {
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
    for case in invalid_disconnects([0x10; 6]) {
        assert!(decode_sc_message(&case.wire).is_ok(), "{}", case.name);
        send_raw(&mut live, &case.wire).await;
        if let Some(nak) = case.nak {
            assert_eq!(recv_raw(&mut live).await, nak, "{}", case.name);
        }
        let mut ack = case.wire;
        ack[0] = 9;
        send_raw(&mut live, &ack).await;
        rejection_barrier(&mut live).await;
        let map = clients.lock().await;
        let client = map
            .get(&live.vmac)
            .expect("malformed Disconnect retired the registration");
        assert_eq!(client.heartbeat, before, "{}", case.name);
        assert_eq!(
            client.last_activity.load(Ordering::Acquire),
            0,
            "{}",
            case.name
        );
        assert!(!client.closed.load(Ordering::Acquire));
        drop(map);
        assert!(
            timeout(Duration::from_millis(10), live.ws.next())
                .await
                .is_err(),
            "{} caused a response to invalid ACK",
            case.name
        );
    }
    live.ack(0x2233).await;
    assert!(clients
        .lock()
        .await
        .get(&live.vmac)
        .unwrap()
        .heartbeat
        .pending
        .is_none());
    send_raw(&mut live, &[0x0A, 0, 0x44, 0x55]).await;
    assert_eq!(recv_raw(&mut live).await, [0x0B, 0, 0x44, 0x55]);
    send_raw(&mut live, &[8, 2, 0x55, 0x66, 0x9E, 0x1E]).await;
    assert_eq!(recv_raw(&mut live).await, [9, 0, 0x55, 0x66]);
    live.expect_closed().await;
    assert!(!clients.lock().await.contains_key(&live.vmac));
}
