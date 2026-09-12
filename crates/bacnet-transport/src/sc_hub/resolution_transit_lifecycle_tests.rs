//! Selected family through the existing supervised retirement/deadline owners.
use super::deadline_test_support::*;
use super::heartbeat_test_support::ClockIo;
use super::response_silence_tests::Snapshot;
use super::shutdown_blocked_tests::ControlledPeer;
use super::unknown_transit_tests::{
    barrier, connect_limits, matrix_pair, raw, recv, send, stopped,
};
use super::*;
use crate::sc_frame::BROADCAST_VMAC;
use std::sync::atomic::AtomicU16;
use std::time::Duration;

#[tokio::test]
async fn resolution_transit_activity_rejection_purity_and_original_ack_timeout() {
    let tls = TestTls::new();
    let (mut hub, mut a, mut observer) = matrix_pair(&tls, true, 0x42, 0x44).await;
    let mut b = ControlledPeer::open(&tls, &hub).await;
    connect_limits(&mut b, 0x43, 1600, 1).await;
    let activity = hub
        .clients
        .lock()
        .await
        .get(&[0x42; 6])
        .unwrap()
        .last_activity
        .clone();
    activity.store(0, Ordering::Release);
    let ids = AtomicU16::new(0x2233);
    // Local rejection cannot defer the first idle probe.
    send(&mut a, raw(2, 0, None, None, 0, &[])).await;
    assert_eq!(
        recv(&mut a).await,
        raw(0, 0, None, None, 0, &[2, 1, 0, 0, 7, 0, 150])
    );
    assert_eq!(activity.load(Ordering::Acquire), 0);
    heartbeat::sweep(&hub.clients, &ids, &ClockIo(AtomicU64::new(100))).await;
    assert_eq!(recv(&mut a).await, [10, 0, 0x22, 0x33]);
    let pending = Some(heartbeat::PendingHeartbeat {
        message_id: 0x2233,
        published_at: 100,
    });
    let before = Snapshot::capture(hub.clients.lock().await.get(&[0x42; 6]).unwrap());
    for function in [2, 3] {
        for (origin, dest) in [
            (None, None),
            (Some([0x43; 6]), Some([0x43; 6])),
            (Some([0; 6]), None),
            (None, Some([0x42; 6])),
            (None, Some(BROADCAST_VMAC)),
        ] {
            send(&mut a, raw(function, 0x2233, origin, dest, 0, &[])).await;
            if function == 2 && origin.is_none() && dest.is_none() {
                assert_eq!(
                    recv(&mut a).await,
                    raw(0, 0x2233, None, None, 0, &[2, 1, 0, 0, 7, 0, 150])
                );
            }
            barrier(&mut a).await;
            barrier(&mut b).await;
            before.unchanged(hub.clients.lock().await.get(&[0x42; 6]).unwrap());
        }
    }
    drop(before);
    for function in [2, 3] {
        for (dest, body) in [
            ([0x77; 6], vec![]),
            ([0; 6], vec![]),
            ([0x43; 6], vec![b'x'; 2000]),
            ([0x43; 6], vec![]),
        ] {
            activity.store(0, Ordering::Release);
            send(&mut a, raw(function, 0x2233, None, Some(dest), 0, &body)).await;
            if dest == [0x43; 6] && body.is_empty() {
                assert_eq!(
                    recv(&mut b).await,
                    raw(function, 0x2233, Some([0x42; 6]), None, 0, &body)
                );
            }
            barrier(&mut a).await;
            barrier(&mut b).await;
            assert!(activity.load(Ordering::Acquire) > 0);
            assert_eq!(
                hub.clients
                    .lock()
                    .await
                    .get(&[0x42; 6])
                    .unwrap()
                    .heartbeat
                    .pending,
                pending
            );
        }
    }
    heartbeat::sweep(&hub.clients, &ids, &ClockIo(AtomicU64::new(105))).await;
    assert_eq!(
        hub.clients
            .lock()
            .await
            .get(&[0x42; 6])
            .unwrap()
            .heartbeat
            .pending,
        pending
    );
    heartbeat::sweep(&hub.clients, &ids, &ClockIo(AtomicU64::new(106))).await;
    until(|| a.deadline.close_started.load(Ordering::Acquire)).await;
    until(|| hub.active.load(Ordering::Acquire) == 2).await;
    assert_eq!(ids.load(Ordering::Acquire), 0x2234);
    barrier(&mut b).await;
    barrier(&mut observer).await;
    stopped(&mut hub).await;
}

fn operation(kind: u8) -> Vec<u8> {
    match kind {
        0 => raw(2, 23, None, None, 0, &[]), // held local Request NAK
        1 => raw(2, 23, None, Some([0x43; 6]), 0, &[]),
        2 => raw(3, 23, None, Some([0x43; 6]), 0, &[]), // empty URI list
        3 => raw(0, 23, None, Some([0x43; 6]), 0, &[2, 0]),
        _ => raw(0, 23, None, Some([0x43; 6]), 0, &[2, 1, 0, 0, 7, 0, 150]),
    }
}

#[tokio::test]
async fn resolution_transit_held_source_target_retirement_and_replacement() {
    let mut cases = 0;
    for kind in 0..5 {
        for source in [true, false] {
            if kind == 0 && !source {
                continue;
            }
            for replace in [false, true] {
                let tls = TestTls::new();
                let (mut hub, mut a, b) = matrix_pair(&tls, true, 0x42, 0x43).await;
                let mut c = ControlledPeer::open(&tls, &hub).await;
                c.connect(0x44).await;
                let held = if kind == 0 {
                    a.sink.clone()
                } else {
                    b.sink.clone()
                }
                .lock_owned()
                .await;
                send(&mut a, operation(kind)).await;
                until(|| a.deadline.received.load(Ordering::Acquire) == 2).await;
                if source {
                    send(&mut a, vec![10, 0, 0, 25]).await;
                }
                let (retired, id) = if source { (&a, 0x42) } else { (&b, 0x43) };
                let mut replacement = if replace {
                    let mut peer = ControlledPeer::open(&tls, &hub).await;
                    peer.connect(id).await;
                    Some(peer)
                } else {
                    assert!(
                        heartbeat::retire(
                            &hub.clients,
                            &heartbeat::Attempt {
                                vmac: [id; 6],
                                sink: retired.sink.clone(),
                                generation: 0,
                            },
                            heartbeat::Retirement::SendFailed,
                            &ClockIo(AtomicU64::new(106))
                        )
                        .await
                    );
                    None
                };
                until(|| retired.deadline.close_started.load(Ordering::Acquire)).await;
                if source {
                    assert_eq!(a.deadline.received.load(Ordering::Acquire), 2);
                } else {
                    // Target retirement wakes the source before the existing 5s Result timeout.
                    barrier(&mut a).await;
                }
                barrier(&mut c).await;
                if let Some(peer) = &mut replacement {
                    barrier(peer).await;
                }
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(6)).await;
                until(|| hub.active.load(Ordering::Acquire) == if replace { 3 } else { 2 }).await;
                drop(held);
                let (old, mut survivor) = if source { (a, b) } else { (b, a) };
                let ControlledPeer { mut ws, sink, .. } = old;
                drop(sink);
                assert!(!matches!(
                    poll_io(ws.next()).await,
                    Some(Ok(Message::Binary(_)))
                ));
                barrier(&mut survivor).await; // no stale relay flush after unlock
                if let Some(peer) = &mut replacement {
                    assert!(Arc::ptr_eq(
                        &hub.clients.lock().await.get(&[id; 6]).unwrap().sink,
                        &peer.sink
                    ));
                    if !source {
                        send(&mut survivor, operation(kind)).await;
                        let wire = operation(kind);
                        assert_eq!(
                            recv(peer).await,
                            raw(wire[0], 23, Some([0x42; 6]), None, 0, &wire[10..])
                        );
                    }
                }
                stopped(&mut hub).await;
                tokio::time::resume();
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 18);
}

#[tokio::test]
async fn resolution_transit_stop_cancels_held_paths_and_joins_workers() {
    for kind in 0..5 {
        let tls = TestTls::new();
        let (mut hub, mut a, b) = matrix_pair(&tls, true, 0x42, 0x43).await;
        let sink = if kind == 0 {
            a.sink.clone()
        } else {
            b.sink.clone()
        };
        let weak = Arc::downgrade(&sink);
        let held = sink.lock_owned().await;
        send(&mut a, operation(kind)).await;
        until(|| a.deadline.received.load(Ordering::Acquire) == 2).await;
        stopped(&mut hub).await;
        drop(a.sink);
        drop(b.sink);
        assert_eq!(weak.strong_count(), 1);
        drop(held);
        assert!(weak.upgrade().is_none());
    }
}

#[tokio::test]
async fn resolution_transit_prereg_request_nak_absolute_deadline_and_capacity_release() {
    let tls = TestTls::new();
    let (mut hub, mut a, mut b) = matrix_pair(&tls, false, 0x42, 0x43).await;
    let before = Snapshot::capture(hub.clients.lock().await.get(&[0x43; 6]).unwrap());
    let expires = a.deadline.expires();
    tokio::time::pause();
    for _ in 0..3 {
        send(&mut a, raw(2, 0, Some([0x43; 6]), Some([0x43; 6]), 0, &[])).await;
        assert_eq!(
            recv(&mut a).await,
            raw(0, 0, None, Some([0x43; 6]), 0, &[2, 1, 0, 0, 7, 0, 150])
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(a.deadline.expires(), expires);
    }
    let held = a.sink.clone().lock_owned().await;
    send(&mut a, operation(1)).await;
    until(|| a.deadline.received.load(Ordering::Acquire) == 4).await;
    a.ws.send(request([0x42; 6], [0x42; 16])).await.unwrap();
    tokio::time::advance(expires.saturating_duration_since(tokio::time::Instant::now())).await;
    assert!(a.deadline.expired());
    // Existing millisecond timer-wheel wake allowance, after exact expiry proof.
    tokio::time::advance(Duration::from_millis(2)).await;
    until(|| a.deadline.close_started.load(Ordering::Acquire)).await;
    assert!(!a.deadline.is_committed());
    assert_eq!(a.deadline.received.load(Ordering::Acquire), 4);
    assert_eq!(hub.active.load(Ordering::Acquire), 2);
    before.unchanged(hub.clients.lock().await.get(&[0x43; 6]).unwrap());
    tokio::time::advance(Duration::from_secs(2)).await;
    until(|| hub.active.load(Ordering::Acquire) == 1).await;
    drop(held);
    drop(a.sink);
    assert!(!matches!(
        poll_io(a.ws.next()).await,
        Some(Ok(Message::Binary(_)))
    ));
    drop(before);
    tokio::time::resume();
    let mut healthy = tls.websocket(hub.address).await;
    healthy.send(request([0x42; 6], [0x42; 16])).await.unwrap();
    assert!(
        matches!(poll_io(healthy.next()).await, Some(Ok(Message::Binary(data))) if data[0] == 7)
    );
    barrier(&mut b).await;
    stopped(&mut hub).await;
}
