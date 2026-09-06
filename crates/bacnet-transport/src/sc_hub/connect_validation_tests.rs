use super::heartbeat::sweep;
use super::heartbeat_test_support::*;
use super::*;
use crate::sc_frame::connect_test_support::{
    invalid_connects, valid_connect, valid_connect_with_options,
};
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
async fn connect_request_invalid_matrix_rejects_before_registration() {
    for case in invalid_connects(6, [0x10; 6]) {
        let clients = clients();
        let mut live = LiveClient::open(clients.clone(), [0x22; 6]).await;
        assert!(decode_sc_message(&case.wire).is_ok(), "{}", case.name);
        send_raw(&mut live, &case.wire).await;
        if let Some(nak) = case.nak {
            assert_eq!(recv_raw(&mut live).await, nak, "{}", case.name);
        }
        live.expect_closed().await;
        timeout(Duration::from_secs(2), &mut live.reader)
            .await
            .unwrap()
            .unwrap();
        assert!(
            clients.lock().await.is_empty(),
            "{} registered a client",
            case.name
        );
    }
}

#[tokio::test]
async fn connect_request_valid_options_preserve_peer_limits_and_repeat_close() {
    let clients = clients();
    let mut live = LiveClient::open(clients.clone(), [0x22; 6]).await;
    let request = valid_connect_with_options(6, live.vmac);
    send_raw(&mut live, &request).await;
    let mut expected = vec![7, 0, 0x22, 0x33];
    expected.extend_from_slice(&[0x10; 6]);
    expected.extend_from_slice(&[0x10; 16]);
    expected.extend_from_slice(&[0x16, 0x49, 0x05, 0xD9]);
    assert_eq!(recv_raw(&mut live).await, expected);
    let map = clients.lock().await;
    let client = map.get(&live.vmac).unwrap();
    assert_eq!(client.device_uuid, [0x33; 16]);
    assert_eq!((client.max_bvlc, client.max_npdu), (8192, 4096));
    drop(map);
    send_raw(&mut live, &request).await;
    live.expect_closed().await;
    timeout(Duration::from_secs(2), &mut live.reader)
        .await
        .unwrap()
        .unwrap();
    assert!(clients.lock().await.is_empty());
}

async fn rejection_barrier(live: &mut LiveClient) {
    send_raw(live, &[6, 0, 0x44, 0x55]).await;
    assert_eq!(
        recv_raw(live).await,
        [0, 0, 0x44, 0x55, 6, 1, 0, 0, 7, 0, 149]
    );
}

#[tokio::test]
async fn connect_request_invalid_repeat_preserves_registration_activity_and_probe() {
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
    let sink = live.sink().await;
    for case in invalid_connects(6, [0x10; 6]) {
        send_raw(&mut live, &case.wire).await;
        if let Some(nak) = case.nak {
            assert_eq!(recv_raw(&mut live).await, nak, "{}", case.name);
        } else {
            // An invalid, eligible Request is a non-activity processing barrier.
            rejection_barrier(&mut live).await;
        }
        let map = clients.lock().await;
        assert_eq!(map.len(), 1);
        let client = map
            .get(&live.vmac)
            .expect("malformed repeat retired the registration");
        assert!(Arc::ptr_eq(&client.sink, &sink));
        assert_eq!(client.device_uuid, [0x22; 16]);
        assert_eq!((client.max_bvlc, client.max_npdu), (1476, 1476));
        assert_eq!(client.heartbeat, before, "{}", case.name);
        assert_eq!(
            client.last_activity.load(Ordering::Acquire),
            0,
            "{}",
            case.name
        );
        assert!(!client.closed.load(Ordering::Acquire));
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
    send_raw(&mut live, &[0x0A, 0, 0x66, 0x77]).await;
    assert_eq!(recv_raw(&mut live).await, [0x0B, 0, 0x66, 0x77]);
}

#[tokio::test]
async fn connect_request_malformed_known_uuid_cannot_replace_live_owner() {
    let clients = clients();
    let mut old = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    let sink = old.sink().await;
    let mut newcomer = LiveClient::open(clients.clone(), [0x42; 6]).await;
    let mut request = valid_connect(6, newcomer.vmac);
    request[10..26].fill(0x22); // spoof the old connection's Device UUID
    request[1] = 1;
    request.insert(4, 0x1E); // forbidden Data Option
    send_raw(&mut newcomer, &request).await;
    assert_eq!(
        recv_raw(&mut newcomer).await,
        [0, 0, 0x22, 0x33, 6, 1, 0, 0, 7, 0, 80]
    );
    newcomer.expect_closed().await;
    timeout(Duration::from_secs(2), &mut newcomer.reader)
        .await
        .unwrap()
        .unwrap();
    let map = clients.lock().await;
    assert_eq!(map.len(), 1);
    let client = map
        .get(&old.vmac)
        .expect("malformed newcomer replaced old UUID owner");
    assert!(Arc::ptr_eq(&client.sink, &sink));
    assert!(!client.closed.load(Ordering::Acquire));
    assert_eq!(client.device_uuid, [0x22; 16]);
    assert_eq!((client.max_bvlc, client.max_npdu), (1476, 1476));
    drop(map);
    send_raw(&mut old, &[0x0A, 0, 0x66, 0x77]).await;
    assert_eq!(recv_raw(&mut old).await, [0x0B, 0, 0x66, 0x77]);
}
