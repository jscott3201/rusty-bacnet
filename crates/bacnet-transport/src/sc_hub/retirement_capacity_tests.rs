//! Exercise the real accepting supervisor, not a reduced connection cap.

use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::heartbeat_test_support::{frame, ClockIo};
use super::retirement_tests::{encode, heartbeat_wire, unicast_wire};
use super::*;

async fn connect(tls: &TestTls, hub: &CountedHub, id: u8) -> ClientWs {
    let mut ws = tls.websocket(hub.address).await;
    ws.send(request([id; 6], [id; 16])).await.unwrap();
    assert!(matches!(poll_io(ws.next()).await,
        Some(Ok(Message::Binary(data))) if data[0] == 7));
    ws
}

async fn settled(hub: &CountedHub, expected: usize) {
    until(|| hub.active.load(Ordering::Acquire) == expected).await;
    until(|| hub.hub.tasks.len() == expected + 1).await; // live clients + heartbeat
    assert_eq!(hub.clients.lock().await.len(), expected);
}

async fn heartbeat(ws: &mut ClientWs) {
    ws.send(heartbeat_wire(25)).await.unwrap();
    assert!(matches!(poll_io(ws.next()).await,
        Some(Ok(Message::Binary(data))) if data[0..4] == [0x0B, 0, 0, 25]));
}

#[tokio::test]
async fn replacement_and_heartbeat_retirement_each_recover_beyond_512_blocked_relays() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut recipient = connect(&tls, &hub, 0x43).await;
    let sink = hub
        .clients
        .lock()
        .await
        .get(&[0x43; 6])
        .unwrap()
        .sink
        .clone();
    let held = sink.lock_owned().await;
    for replace in [true, false] {
        for cycle in 0..513 {
            let mut source = connect(&tls, &hub, 0x42).await;
            let (activity, attempt) = {
                let map = hub.clients.lock().await;
                let client = map.get(&[0x42; 6]).unwrap();
                (
                    client.last_activity.clone(),
                    heartbeat::Attempt {
                        vmac: [0x42; 6],
                        sink: client.sink.clone(),
                        generation: client.heartbeat.generation,
                    },
                )
            };
            activity.store(0, Ordering::Release);
            source.send(unicast_wire([0x43; 6])).await.unwrap();
            until(|| activity.load(Ordering::Acquire) > 0).await;
            // Admission/activity barrier: the real source dispatch reached its
            // held target sink. Its own Close sink remains available.
            if replace {
                let mut replacement = connect(&tls, &hub, 0x42).await;
                settled(&hub, 2).await;
                heartbeat(&mut replacement).await;
                replacement.close(None).await.unwrap();
            } else {
                // Repeat external heartbeat retirement signals. The separate
                // send-failure and timeout tests witness their detection paths.
                assert!(
                    heartbeat::retire(
                        &hub.clients,
                        &attempt,
                        heartbeat::Retirement::SendFailed,
                        &ClockIo(AtomicU64::new(now_secs()))
                    )
                    .await
                );
            }
            settled(&hub, 1).await;
            assert!(matches!(
                poll_io(source.next()).await,
                None | Some(Err(_)) | Some(Ok(Message::Close(_)))
            ));
            if cycle % 32 == 0 {
                // Keep the healthy recipient active even on slow hosts while
                // its sink is deliberately held; ACK input requires no reply.
                recipient
                    .send(encode(frame(ScFunction::HeartbeatAck, 26)))
                    .await
                    .unwrap();
            }
        }
    }
    drop(held);
    heartbeat(&mut recipient).await;
    recipient.close(None).await.unwrap();
    settled(&hub, 0).await;
    let mut recovery = connect(&tls, &hub, 0x44).await;
    heartbeat(&mut recovery).await;
    recovery.close(None).await.unwrap();
    settled(&hub, 0).await;
    hub.hub.stop().await;
}

#[tokio::test]
async fn terminal_relays_and_peer_close_each_recover_beyond_512_connections() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut source = connect(&tls, &hub, 0x44).await;
    // 513 actual admissions for EACH included path: NPDU unicast, broadcast,
    // forwarded Result, and normal observed peer Close (2,052 target sessions).
    for path in 0..4 {
        for _ in 0..513 {
            let mut target = connect(&tls, &hub, 0x42).await;
            if path == 3 {
                target.close(None).await.unwrap();
            } else {
                let sink = hub
                    .clients
                    .lock()
                    .await
                    .get(&[0x42; 6])
                    .unwrap()
                    .sink
                    .clone();
                sink.lock().await.send(Message::Close(None)).await.unwrap();
                let wire = match path {
                    0 => unicast_wire([0x42; 6]),
                    1 => unicast_wire([0xFF; 6]),
                    _ => {
                        let mut result = frame(ScFunction::Result, 24);
                        result.destination_vmac = Some([0x42; 6]);
                        result.payload = Bytes::from_static(&[1, 0]);
                        encode(result)
                    }
                };
                source.send(wire).await.unwrap();
                heartbeat(&mut source).await; // relay processing barrier
            }
            settled(&hub, 1).await;
        }
    }
    heartbeat(&mut source).await;
    source.close(None).await.unwrap();
    settled(&hub, 0).await;
    let mut recovery = connect(&tls, &hub, 0x45).await;
    heartbeat(&mut recovery).await;
    recovery.close(None).await.unwrap();
    settled(&hub, 0).await;
    hub.hub.stop().await;
}
