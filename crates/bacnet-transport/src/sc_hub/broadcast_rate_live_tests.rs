//! Public-startup TLS loopback tests with frozen Tokio time and wire barriers.
use super::*;
use crate::sc_frame::BROADCAST_VMAC;
use crate::sc_hub::deadline_test_support::{poll_io, request, ClientWs, TestTls};
use crate::sc_hub::ScHubHandshakeTimeouts;
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

async fn send(peer: &mut ClientWs, wire: Vec<u8>) {
    poll_io(peer.send(Message::Binary(wire.into())))
        .await
        .unwrap();
}

async fn recv(peer: &mut ClientWs) -> Vec<u8> {
    match poll_io(peer.next()).await.unwrap().unwrap() {
        Message::Binary(data) => data.to_vec(),
        other => panic!("expected binary frame, got {other:?}"),
    }
}

async fn connect(tls: &TestTls, hub: &ScHub, id: u8) -> ClientWs {
    let mut peer = poll_io(tls.websocket(hub.local_addr().unwrap())).await;
    poll_io(peer.send(request([id; 6], [id; 16])))
        .await
        .unwrap();
    assert_eq!(recv(&mut peer).await[0], 7);
    peer
}

// Independent wire oracle: no product encoder/relay helper constructs expected bytes.
fn wire(function: u8, id: u16, origin: Option<Vmac>, dest: Option<Vmac>) -> Vec<u8> {
    let mut bytes = vec![
        function,
        u8::from(origin.is_some()) * 8 + u8::from(dest.is_some()) * 4,
    ];
    bytes.extend_from_slice(&id.to_be_bytes());
    if let Some(vmac) = origin {
        bytes.extend_from_slice(&vmac);
    }
    if let Some(vmac) = dest {
        bytes.extend_from_slice(&vmac);
    }
    bytes.extend_from_slice(match function {
        0 => &[1, 0], // successful Result for NPDU
        2 | 3 | 5 => &[],
        4 => &[1, 0, 0xFF, 0xFF, 0x05, 0xD9],
        12 => &[0, 43, 7, 99],
        _ => &[1, 0],
    });
    bytes
}

async fn broadcast(peer: &mut ClientWs, count: u16) {
    for id in 0..count {
        let function = [1, 12, 0x80][usize::from(id % 3)];
        send(peer, wire(function, id, None, Some(BROADCAST_VMAC))).await;
    }
}

async fn barrier(peer: &mut ClientWs) {
    send(peer, vec![10, 0, 0xCA, 0xFE]).await;
    assert_eq!(recv(peer).await, [11, 0, 0xCA, 0xFE]);
}

async fn drain_broadcasts_to_barrier(peer: &mut ClientWs) -> usize {
    send(peer, vec![10, 0, 0xCA, 0xFE]).await;
    let mut count = 0;
    loop {
        let received = recv(peer).await;
        if received == [11, 0, 0xCA, 0xFE] {
            return count;
        }
        // Reject any NAK/reflection or non-broadcast traffic in the drain.
        assert!(matches!(received[0], 1 | 12 | 0x80));
        assert_eq!(received[1], 12);
        assert_eq!(&received[10..16], &BROADCAST_VMAC);
        count += 1;
    }
}

#[tokio::test(start_paused = true)]
async fn single_sender_flood_is_bounded_others_flow_and_hub_stays_responsive() {
    let tls = TestTls::new();
    let policy = ScHubBroadcastRatePolicy {
        sender_burst: 8,
        sender_per_second: 2,
        global_burst: 64,
        global_per_second: 16,
    };
    let mut hub = ScHub::start(
        "127.0.0.1:0",
        tls.hub_config.clone().with_broadcast_rate_policy(policy),
        [16; 6],
        [16; 16],
    )
    .await
    .unwrap();
    let mut a = connect(&tls, &hub, 42).await;
    let mut b = connect(&tls, &hub, 43).await;
    let mut c = connect(&tls, &hub, 44).await;
    broadcast(&mut a, 128).await;
    barrier(&mut a).await; // all input processed, no rate-drop NAKs
    for id in 0..8 {
        let expected = wire(
            [1, 12, 0x80][usize::from(id % 3)],
            id,
            Some([42; 6]),
            Some(BROADCAST_VMAC),
        );
        assert_eq!(recv(&mut b).await, expected);
        assert_eq!(recv(&mut c).await, expected);
    }
    barrier(&mut b).await;
    barrier(&mut c).await;
    assert_eq!(
        hub.broadcast_drop_counts(),
        ScHubBroadcastDropCounts {
            sender_exhausted: 120,
            global_exhausted: 0,
        }
    );
    broadcast(&mut b, 3).await;
    barrier(&mut b).await;
    for id in 0..3 {
        let expected = wire(
            [1, 12, 0x80][usize::from(id % 3)],
            id,
            Some([43; 6]),
            Some(BROADCAST_VMAC),
        );
        assert_eq!(recv(&mut a).await, expected);
        assert_eq!(recv(&mut c).await, expected);
    }
    // A new peer can still register while the offending sender is exhausted.
    let mut d = connect(&tls, &hub, 45).await;
    barrier(&mut d).await;
    // Exact boundary: 2/s produces its first replacement token at 500ms.
    tokio::time::advance(Duration::from_millis(500) - Duration::from_nanos(1)).await;
    broadcast(&mut a, 1).await;
    barrier(&mut a).await;
    tokio::time::advance(Duration::from_nanos(1)).await;
    broadcast(&mut a, 2).await;
    barrier(&mut a).await;
    for peer in [&mut b, &mut c, &mut d] {
        assert_eq!(
            recv(peer).await,
            wire(1, 0, Some([42; 6]), Some(BROADCAST_VMAC))
        );
        barrier(peer).await;
    }
    assert_eq!(hub.broadcast_drop_counts().sender_exhausted, 122);
    poll_io(hub.stop()).await;
    assert_eq!(hub.broadcast_drop_counts().sender_exhausted, 122);
}

#[tokio::test(start_paused = true)]
async fn concurrent_multi_sender_flood_and_reconnect_obey_global_budget() {
    let tls = TestTls::new();
    let policy = ScHubBroadcastRatePolicy {
        sender_burst: 128,
        sender_per_second: 128,
        global_burst: 10,
        global_per_second: 2,
    };
    let mut hub = ScHub::start(
        "127.0.0.1:0",
        tls.hub_config.clone().with_broadcast_rate_policy(policy),
        [16; 6],
        [16; 16],
    )
    .await
    .unwrap();
    let mut a = connect(&tls, &hub, 42).await;
    let mut b = connect(&tls, &hub, 43).await;
    let mut observer = connect(&tls, &hub, 44).await;
    tokio::join!(broadcast(&mut a, 64), broadcast(&mut b, 64));
    // Stop source traffic with ordered heartbeats before draining the observer.
    let (a_count, b_count) = tokio::join!(
        drain_broadcasts_to_barrier(&mut a),
        drain_broadcasts_to_barrier(&mut b)
    );
    // One source's ACK can precede the other source's final broadcasts. Now
    // both sources have finished, a second barrier drains that possible tail.
    let a_count = a_count + drain_broadcasts_to_barrier(&mut a).await;
    let b_count = b_count + drain_broadcasts_to_barrier(&mut b).await;
    assert_eq!(drain_broadcasts_to_barrier(&mut observer).await, 10);
    assert_eq!(a_count + b_count, 10);
    assert_eq!(
        hub.broadcast_drop_counts(),
        ScHubBroadcastDropCounts {
            sender_exhausted: 0,
            global_exhausted: 118,
        }
    );
    // Replacing a registered lease replenishes only its connection-local bucket.
    let mut replacement = connect(&tls, &hub, 42).await;
    broadcast(&mut replacement, 4).await;
    barrier(&mut replacement).await;
    barrier(&mut b).await;
    barrier(&mut observer).await;
    assert_eq!(hub.broadcast_drop_counts().global_exhausted, 122);
    tokio::time::advance(Duration::from_secs(1)).await;
    broadcast(&mut replacement, 5).await;
    barrier(&mut replacement).await;
    assert_eq!(drain_broadcasts_to_barrier(&mut b).await, 2);
    assert_eq!(drain_broadcasts_to_barrier(&mut observer).await, 2);
    assert_eq!(hub.broadcast_drop_counts().global_exhausted, 125);
    poll_io(hub.stop()).await;
}

#[tokio::test(start_paused = true)]
async fn exhausted_budgets_do_not_change_high_rate_unicast_or_control_bytes() {
    let tls = TestTls::new();
    let policy = ScHubBroadcastRatePolicy {
        sender_burst: 1,
        sender_per_second: 1,
        global_burst: 1,
        global_per_second: 1,
    };
    let mut hub = ScHub::start(
        "127.0.0.1:0",
        tls.hub_config.clone().with_broadcast_rate_policy(policy),
        [16; 6],
        [16; 16],
    )
    .await
    .unwrap();
    let mut a = connect(&tls, &hub, 42).await;
    let mut b = connect(&tls, &hub, 43).await;
    broadcast(&mut a, 2).await;
    barrier(&mut a).await;
    assert_eq!(
        recv(&mut b).await,
        wire(1, 0, Some([42; 6]), Some(BROADCAST_VMAC))
    );
    for id in 0..128 {
        // Includes the out-of-scope opaque controls and routed Result family.
        for function in [0, 1, 2, 3, 4, 5, 12, 0x80] {
            send(&mut a, wire(function, id, None, Some([43; 6]))).await;
            assert_eq!(recv(&mut b).await, wire(function, id, Some([42; 6]), None));
        }
    }
    // Absent destination is local, never reclassified as broadcast. An Unknown
    // local request retains its existing NAK even with both budgets exhausted.
    send(&mut a, wire(0x80, 0x1234, None, None)).await;
    assert_eq!(
        recv(&mut a).await,
        [0, 0, 0x12, 0x34, 0x80, 1, 0, 0, 7, 0, 143]
    );
    barrier(&mut a).await;
    barrier(&mut b).await;
    assert_eq!(
        hub.broadcast_drop_counts(),
        ScHubBroadcastDropCounts {
            sender_exhausted: 1,
            global_exhausted: 0,
        }
    );
    send(&mut a, vec![8, 0, 0x12, 0x34]).await;
    assert_eq!(recv(&mut a).await, [9, 0, 0x12, 0x34]);
    poll_io(hub.stop()).await;
}

#[tokio::test]
async fn every_startup_rejects_invalid_policy_before_bind() {
    let tls = TestTls::new();
    for index in 0..4 {
        for invalid in [0, u64::MAX / TOKEN + 1] {
            let mut p = ScHubBroadcastRatePolicy::default();
            let fields = [
                &mut p.sender_burst,
                &mut p.sender_per_second,
                &mut p.global_burst,
                &mut p.global_per_second,
            ];
            *fields.into_iter().nth(index).unwrap() = invalid;
            for api in 0..4 {
                let config = tls.hub_config.clone().with_broadcast_rate_policy(p);
                let result = match api {
                    0 => ScHub::start("invalid bind address", config, [16; 6], [16; 16]).await,
                    1 => {
                        ScHub::start_with_uuid("invalid bind address", config, [16; 6], [16; 16])
                            .await
                    }
                    2 => {
                        ScHub::start_with_uuid_and_timeouts(
                            "invalid bind address",
                            config,
                            [16; 6],
                            [16; 16],
                            ScHubHandshakeTimeouts::default(),
                        )
                        .await
                    }
                    _ => {
                        ScHub::start_with_tls_config(
                            "invalid bind address",
                            config,
                            [16; 6],
                            [16; 16],
                            ScHubHandshakeTimeouts::default(),
                        )
                        .await
                    }
                };
                assert!(
                    matches!(result, Err(Error::Encoding(message)) if message.starts_with("hub broadcast "))
                );
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn cloned_configuration_gives_independent_hub_buckets_and_counters() {
    let tls = TestTls::new();
    let config = tls
        .hub_config
        .clone()
        .with_broadcast_rate_policy(ScHubBroadcastRatePolicy {
            sender_burst: 1,
            sender_per_second: 1,
            global_burst: 1,
            global_per_second: 1,
        });
    let mut first = ScHub::start("127.0.0.1:0", config.clone(), [16; 6], [16; 16])
        .await
        .unwrap();
    let mut second = ScHub::start("127.0.0.1:0", config, [17; 6], [17; 16])
        .await
        .unwrap();
    for hub in [&mut first, &mut second] {
        let mut a = connect(&tls, hub, 42).await;
        let mut b = connect(&tls, hub, 43).await;
        broadcast(&mut a, 2).await;
        barrier(&mut a).await;
        assert_eq!(
            recv(&mut b).await,
            wire(1, 0, Some([42; 6]), Some(BROADCAST_VMAC))
        );
        barrier(&mut b).await;
        assert_eq!(
            hub.broadcast_drop_counts(),
            ScHubBroadcastDropCounts {
                sender_exhausted: 1,
                global_exhausted: 0,
            }
        );
        poll_io(hub.stop()).await;
    }
}
