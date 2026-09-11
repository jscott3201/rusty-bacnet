//! Registered-hub admission over real TLS, without an encoder oracle.
use super::heartbeat_test_support::*;
use super::*;
use crate::sc_frame::BROADCAST_VMAC;
use std::time::Duration;
use tokio::time::timeout;

async fn send(live: &mut LiveClient, wire: Vec<u8>) {
    live.ws.send(Message::Binary(wire.into())).await.unwrap();
}

async fn recv(live: &mut LiveClient) -> Vec<u8> {
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

async fn disconnect(live: &mut LiveClient) {
    send(live, vec![8, 0, 0x55, 0x66]).await;
    // Drain any rejection NAK before the Disconnect-ACK.
    loop {
        let response = recv(live).await;
        if response == [9, 0, 0x55, 0x66] {
            break;
        }
        assert_eq!(response[0], 0);
    }
    live.expect_closed().await;
    timeout(Duration::from_secs(2), &mut live.reader)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn empty_npdu_hub_does_not_relay_before_positive_sentinel() {
    let clients = clients();
    let mut source = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    let mut target = LiveClient::connect(clients.clone(), [0x43; 6]).await;
    let mut wire = vec![1, 4, 0x22, 0x33];
    wire.extend_from_slice(&target.vmac);
    send(&mut source, wire.clone()).await;
    wire.extend_from_slice(&[1, 0, 0x30]);
    send(&mut source, wire).await;
    let first = recv(&mut target).await;
    let mut expected = vec![1, 8, 0x22, 0x33];
    expected.extend_from_slice(&source.vmac);
    expected.extend_from_slice(&[1, 0, 0x30]);
    // Close/join before the RED assertion, including a wrongly relayed sentinel.
    if first != expected {
        let _ = recv(&mut target).await;
    }
    disconnect(&mut source).await;
    disconnect(&mut target).await;
    assert!(clients.lock().await.is_empty());
    assert_eq!(first, expected, "empty NPDU was relayed");
}

fn npdu(source: Option<Vmac>, destination: Option<Vmac>, options: u8, body: &[u8]) -> Vec<u8> {
    let mut wire = vec![
        1,
        u8::from(source.is_some()) * 8 + u8::from(destination.is_some()) * 4 + options,
        0x22,
        0x33,
    ];
    if let Some(source) = source {
        wire.extend_from_slice(&source);
    }
    if let Some(destination) = destination {
        wire.extend_from_slice(&destination);
    }
    wire.extend_from_slice(body);
    wire
}

async fn barrier(live: &mut LiveClient) {
    send(live, vec![8, 0, 0x44, 0x55, 0x42]).await;
    assert_eq!(recv(live).await, [0, 0, 0x44, 0x55, 8, 1, 0, 0, 7, 0, 7]);
}

#[tokio::test]
async fn empty_npdu_hub_routing_options_no_fanout_or_activity_then_healthy_relay() {
    use super::response_silence_tests::Snapshot;
    use std::sync::atomic::AtomicU16;
    let clients = clients();
    let mut source = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    let mut targets = Vec::new();
    for id in [0x43, 0x44] {
        targets.push(LiveClient::connect(clients.clone(), [id; 6]).await);
    }
    source.idle().await;
    heartbeat::sweep(
        &clients,
        &AtomicU16::new(0x2233),
        &ClockIo(AtomicU64::new(100)),
    )
    .await;
    assert_eq!(recv(&mut source).await, [0x0A, 0, 0x22, 0x33]);
    let before: Vec<_> = clients
        .lock()
        .await
        .iter()
        .map(|(vmac, client)| (*vmac, Snapshot::capture(client)))
        .collect();
    for origin in [None, Some([0x55; 6]), Some([0; 6]), Some(BROADCAST_VMAC)] {
        for destination in [
            None,
            Some([0x43; 6]),
            Some(BROADCAST_VMAC),
            Some([0; 6]),
            Some([0x66; 6]),
        ] {
            for (flags, options) in [
                (0, vec![]),
                (2, vec![0xA2, 0, 0, 0x1F]),
                (3, vec![0xE2, 0, 0, 0x1F, 0xFE, 0, 1, 0xBB, 0x1F]),
            ] {
                let raw = npdu(origin, destination, flags, &options);
                assert!(decode_sc_message(&raw).unwrap().payload.is_empty());
                send(&mut source, raw).await;
                if origin.is_none() && destination.is_some() && destination != Some(BROADCAST_VMAC)
                {
                    // MU stays opaque at the hub; absence is structural. Unknown
                    // and zero destinations do not require recipient lookup.
                    assert_eq!(
                        recv(&mut source).await,
                        [0, 0, 0x22, 0x33, 1, 1, 0, 0, 7, 0, 0x95]
                    );
                }
                barrier(&mut source).await;
                let map = clients.lock().await;
                assert_eq!(map.len(), 3);
                for (vmac, snapshot) in &before {
                    snapshot.unchanged(map.get(vmac).unwrap());
                }
            }
        }
    }
    // Source's barrier is processed only after any erroneous relay send. These
    // recipient barriers therefore detect *all* queued empty unicasts/broadcasts.
    for target in &mut targets {
        barrier(target).await;
    }
    source.ack(0x2233).await;
    assert!(clients
        .lock()
        .await
        .get(&source.vmac)
        .unwrap()
        .heartbeat
        .pending
        .is_none());
    // The hub still forwards positive payloads and raw option markers exactly,
    // including MU which the final destination, not this hub, interprets.
    for payload in [&[0x42][..], &[1, 0, 0x10, 8][..]] {
        for destination in [[0x43; 6], BROADCAST_VMAC] {
            let mut body = vec![0xE2, 0, 0, 0x1F, 0xFE, 0, 1, 0xBB, 0x1F];
            body.extend_from_slice(payload);
            send(&mut source, npdu(None, Some(destination), 3, &body)).await;
            let broadcast = destination == BROADCAST_VMAC;
            let expected = npdu(
                Some(source.vmac),
                broadcast.then_some(BROADCAST_VMAC),
                3,
                &body,
            );
            assert_eq!(recv(&mut targets[0]).await, expected);
            if broadcast {
                assert_eq!(recv(&mut targets[1]).await, expected);
            }
            barrier(&mut source).await;
            barrier(&mut targets[1]).await;
        }
    }
    // Result relay is not covered by the empty-NPDU guard.
    let mut result = vec![0, 4, 0x22, 0x33];
    result.extend_from_slice(&source.vmac);
    result.extend_from_slice(&[1, 1, 0, 0, 7, 0, 0x95]);
    send(&mut targets[0], result).await;
    let mut expected = vec![0, 8, 0x22, 0x33];
    expected.extend_from_slice(&targets[0].vmac);
    expected.extend_from_slice(&[1, 1, 0, 0, 7, 0, 0x95]);
    assert_eq!(recv(&mut source).await, expected);
    send(&mut source, vec![0x0A, 0, 0x66, 0x77]).await;
    assert_eq!(recv(&mut source).await, [0x0B, 0, 0x66, 0x77]);
    for target in &mut targets {
        disconnect(target).await;
    }
    disconnect(&mut source).await;
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn empty_npdu_hub_pre_registration_keeps_other_and_later_connect() {
    let clients = clients();
    let mut live = LiveClient::open(clients.clone(), [0x22; 6]).await;
    for destination in [None, Some([0x43; 6]), Some(BROADCAST_VMAC)] {
        send(&mut live, npdu(Some([0x55; 6]), destination, 2, &[0x5E])).await;
        // Characterization, NOT a new preregistration broadcast conformance claim.
        assert_eq!(
            recv(&mut live).await,
            [0, 0, 0x22, 0x33, 1, 1, 0, 0, 7, 0, 0]
        );
        assert!(clients.lock().await.is_empty());
    }
    live.ws
        .send(super::deadline_test_support::request(live.vmac, [0x22; 16]))
        .await
        .unwrap();
    assert_eq!(live.recv().await.function, ScFunction::ConnectAccept);
    disconnect(&mut live).await;
    assert!(clients.lock().await.is_empty());
}

#[tokio::test]
async fn empty_npdu_hub_does_not_defer_idle_or_original_heartbeat_timeout() {
    use std::sync::atomic::AtomicU16;
    let clients = clients();
    let mut live = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    live.idle().await;
    let ids = AtomicU16::new(0x2233);
    for now in [60, 100, 101, 102, 103, 104, 105, 106] {
        send(&mut live, npdu(None, Some(BROADCAST_VMAC), 0, &[])).await;
        barrier(&mut live).await;
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
        heartbeat::sweep(&clients, &ids, &ClockIo(AtomicU64::new(now))).await;
        if now == 100 {
            assert_eq!(recv(&mut live).await, [0x0A, 0, 0x22, 0x33]);
        }
        if (100..=105).contains(&now) {
            assert_eq!(
                clients
                    .lock()
                    .await
                    .get(&live.vmac)
                    .unwrap()
                    .heartbeat
                    .pending,
                Some(heartbeat::PendingHeartbeat {
                    message_id: 0x2233,
                    published_at: 100
                })
            );
        }
    }
    live.expect_closed().await;
    timeout(Duration::from_secs(2), &mut live.reader)
        .await
        .unwrap()
        .unwrap();
    assert!(clients.lock().await.is_empty());
    assert_eq!(ids.load(Ordering::Acquire), 0x2234);
}
