//! Unknown-only liveness and held-sink cancellation under the existing owners.
use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::heartbeat_test_support::ClockIo;
use super::response_silence_tests::Snapshot;
use super::shutdown_blocked_tests::ControlledPeer;
use super::unknown_transit_tests::{barrier, connect_limits, raw, recv, send, stopped};
use super::*;
use crate::sc_frame::BROADCAST_VMAC;
use std::sync::atomic::AtomicU16;
use std::time::Duration;

#[tokio::test]
async fn unknown_transit_activity_local_idle_and_independent_pending_ack_timeout() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut a = ControlledPeer::open(&tls, &hub).await;
    a.connect(0x42).await;
    let mut b = ControlledPeer::open(&tls, &hub).await;
    connect_limits(&mut b, 0x43, 1600, 1).await;
    hub.clients
        .lock()
        .await
        .get(&[0x42; 6])
        .unwrap()
        .last_activity
        .store(0, Ordering::Release);
    let ids = AtomicU16::new(0x2233);
    for now in [60, 100] {
        send(&mut a, raw(0x42, 0, None, None, 0, &[])).await;
        assert_eq!(
            recv(&mut a).await,
            raw(0, 0, None, None, 0, &[0x42, 1, 0, 0, 7, 0, 143])
        );
        barrier(&mut a).await;
        assert_eq!(
            hub.clients
                .lock()
                .await
                .get(&[0x42; 6])
                .unwrap()
                .last_activity
                .load(Ordering::Acquire),
            0
        );
        heartbeat::sweep(&hub.clients, &ids, &ClockIo(AtomicU64::new(now))).await;
    }
    assert_eq!(recv(&mut a).await, [10, 0, 0x22, 0x33]);
    let pending = Some(heartbeat::PendingHeartbeat {
        message_id: 0x2233,
        published_at: 100,
    });
    // Local/invalid/self paths leave the entire snapshot unchanged, even pending.
    let before = Snapshot::capture(hub.clients.lock().await.get(&[0x42; 6]).unwrap());
    for (origin, dest) in [
        (Some([0x43; 6]), Some([0x43; 6])),
        (Some([0; 6]), None),
        (None, Some([0x42; 6])),
    ] {
        send(&mut a, raw(0x0D, 0, origin, dest, 0, &[])).await;
        barrier(&mut a).await;
        before.unchanged(hub.clients.lock().await.get(&[0x42; 6]).unwrap());
    }
    drop(before);
    // Well-formed transit refreshes activity even with missing/zero/too-small
    // targets. Its opaque body never clears or republishes an outstanding probe.
    for (dest, body) in [
        ([0x77; 6], vec![]),
        ([0; 6], vec![]),
        ([0x43; 6], vec![0xFF; 2000]),
        ([0x43; 6], vec![]),
        (BROADCAST_VMAC, vec![0xFF]),
    ] {
        hub.clients
            .lock()
            .await
            .get(&[0x42; 6])
            .unwrap()
            .last_activity
            .store(0, Ordering::Release);
        send(&mut a, raw(0xFF, 0, None, Some(dest), 0, &body)).await;
        if (dest == [0x43; 6] && body.is_empty()) || dest == BROADCAST_VMAC {
            assert_eq!(
                recv(&mut b).await,
                raw(
                    0xFF,
                    0,
                    Some([0x42; 6]),
                    (dest == BROADCAST_VMAC).then_some(dest),
                    0,
                    &body
                )
            );
        }
        barrier(&mut a).await;
        barrier(&mut b).await;
        let map = hub.clients.lock().await;
        let client = map.get(&[0x42; 6]).unwrap();
        assert!(client.last_activity.load(Ordering::Acquire) > 0);
        assert_eq!(client.heartbeat.pending, pending);
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
    until(|| hub.active.load(Ordering::Acquire) == 1).await;
    assert_eq!(ids.load(Ordering::Acquire), 0x2234);
    barrier(&mut b).await;
    stopped(&mut hub).await;
}

fn operation(kind: u8) -> Vec<u8> {
    match kind {
        0 => raw(0x42, 23, None, None, 0, &[]), // held local NAK
        1 => raw(0x42, 23, None, Some([0x43; 6]), 0, &[0xFF]),
        2 => raw(0x42, 23, None, Some(BROADCAST_VMAC), 0, &[0xFF]),
        3 => raw(0, 23, None, Some([0x43; 6]), 0, &[0x42, 0]),
        _ => raw(0, 23, None, Some([0x43; 6]), 0, &[0x42, 1, 0, 0, 7, 0, 143]),
    }
}

async fn retire(hub: &CountedHub, peer: &ControlledPeer, id: u8) {
    assert!(
        heartbeat::retire(
            &hub.clients,
            &heartbeat::Attempt {
                vmac: [id; 6],
                sink: peer.sink.clone(),
                generation: 0
            },
            heartbeat::Retirement::SendFailed,
            &ClockIo(AtomicU64::new(106))
        )
        .await
    );
}

#[tokio::test]
async fn unknown_transit_held_local_transit_reply_source_retirement_or_replacement() {
    for kind in 0..5 {
        for replace in [false, true] {
            let tls = TestTls::new();
            let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
            let mut a = ControlledPeer::open(&tls, &hub).await;
            a.connect(0x42).await;
            let mut b = ControlledPeer::open(&tls, &hub).await;
            b.connect(0x43).await;
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
            if kind == 2 {
                assert_eq!(
                    recv(&mut c).await,
                    raw(0x42, 23, Some([0x42; 6]), Some(BROADCAST_VMAC), 0, &[0xFF])
                );
            }
            // A queued frame must not be dispatched after source retirement.
            send(&mut a, vec![10, 0, 0, 25]).await;
            let mut replacement = if replace {
                let mut peer = ControlledPeer::open(&tls, &hub).await;
                peer.connect(0x42).await;
                Some(peer)
            } else {
                retire(&hub, &a, 0x42).await;
                None
            };
            until(|| a.deadline.close_started.load(Ordering::Acquire)).await;
            assert_eq!(a.deadline.received.load(Ordering::Acquire), 2);
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(6)).await;
            until(|| hub.active.load(Ordering::Acquire) == if replace { 3 } else { 2 }).await;
            if let Some(peer) = &mut replacement {
                assert!(Arc::ptr_eq(
                    &hub.clients.lock().await.get(&[0x42; 6]).unwrap().sink,
                    &peer.sink
                ));
                barrier(peer).await;
            }
            drop(held);
            barrier(&mut b).await; // no cancelled stale relay flushed on release
            barrier(&mut c).await;
            drop(a.sink);
            assert!(!matches!(
                poll_io(a.ws.next()).await,
                Some(Ok(Message::Binary(_)))
            ));
            stopped(&mut hub).await;
            tokio::time::resume();
        }
    }
}

#[tokio::test]
async fn unknown_transit_held_target_retirement_replacement_preserves_source_and_other_targets() {
    for kind in 1..5 {
        for replace in [false, true] {
            let tls = TestTls::new();
            let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
            let mut a = ControlledPeer::open(&tls, &hub).await;
            a.connect(0x42).await;
            let mut b = ControlledPeer::open(&tls, &hub).await;
            b.connect(0x43).await;
            let mut c = ControlledPeer::open(&tls, &hub).await;
            c.connect(0x44).await;
            let held = b.sink.clone().lock_owned().await;
            send(&mut a, operation(kind)).await;
            until(|| a.deadline.received.load(Ordering::Acquire) == 2).await;
            if kind == 2 {
                assert_eq!(
                    recv(&mut c).await,
                    raw(0x42, 23, Some([0x42; 6]), Some(BROADCAST_VMAC), 0, &[0xFF])
                );
            }
            let mut replacement = if replace {
                let mut peer = ControlledPeer::open(&tls, &hub).await;
                peer.connect(0x43).await;
                Some(peer)
            } else {
                retire(&hub, &b, 0x43).await;
                None
            };
            until(|| b.deadline.close_started.load(Ordering::Acquire)).await;
            // Target retirement interrupts the wait, not just the 5s timeout.
            barrier(&mut a).await;
            barrier(&mut c).await;
            if let Some(peer) = &mut replacement {
                barrier(peer).await;
            }
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(6)).await;
            until(|| hub.active.load(Ordering::Acquire) == if replace { 3 } else { 2 }).await;
            drop(held);
            drop(b.sink);
            assert!(!matches!(
                poll_io(b.ws.next()).await,
                Some(Ok(Message::Binary(_)))
            ));
            if let Some(peer) = &mut replacement {
                assert!(Arc::ptr_eq(
                    &hub.clients.lock().await.get(&[0x43; 6]).unwrap().sink,
                    &peer.sink
                ));
                // A new operation routes to the new identity; the cancelled one did not.
                send(&mut a, raw(0xFF, 24, None, Some([0x43; 6]), 0, &[])).await;
                assert_eq!(
                    recv(peer).await,
                    raw(0xFF, 24, Some([0x42; 6]), None, 0, &[])
                );
            }
            stopped(&mut hub).await;
            tokio::time::resume();
        }
    }
}

#[tokio::test]
async fn unknown_transit_stop_cancels_all_held_paths_and_joins_workers() {
    for kind in 0..5 {
        let tls = TestTls::new();
        let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
        let mut a = ControlledPeer::open(&tls, &hub).await;
        a.connect(0x42).await;
        let mut b = ControlledPeer::open(&tls, &hub).await;
        b.connect(0x43).await;
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
async fn unknown_transit_prereg_held_nak_absolute_expiry_capacity_and_later_connect() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut a = ControlledPeer::open(&tls, &hub).await;
    let mut b = ControlledPeer::open(&tls, &hub).await;
    b.connect(0x43).await;
    let before = Snapshot::capture(hub.clients.lock().await.get(&[0x43; 6]).unwrap());
    let expires = a.deadline.expires();
    tokio::time::pause();
    for _ in 0..3 {
        send(
            &mut a,
            raw(0xFF, 0, Some([0x43; 6]), Some([0x43; 6]), 0, &[]),
        )
        .await;
        assert_eq!(
            recv(&mut a).await,
            raw(0, 0, None, Some([0x43; 6]), 0, &[0xFF, 1, 0, 0, 7, 0, 143])
        );
        barrier(&mut b).await;
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(a.deadline.expires(), expires);
    }
    let held = a.sink.clone().lock_owned().await;
    send(&mut a, operation(1)).await; // preregistration: local NAK, never transit
    until(|| a.deadline.received.load(Ordering::Acquire) == 4).await;
    a.ws.send(request([0x42; 6], [0x42; 16])).await.unwrap();
    tokio::time::advance(expires.saturating_duration_since(tokio::time::Instant::now())).await;
    assert!(a.deadline.expired());
    // Match deadline_commit_tests: Tokio's millisecond timer wheel may wake on
    // the next tick. The absolute predicate is already expired at the boundary.
    tokio::time::advance(Duration::from_millis(2)).await;
    until(|| a.deadline.close_started.load(Ordering::Acquire)).await;
    assert!(!a.deadline.is_committed());
    assert_eq!(a.deadline.received.load(Ordering::Acquire), 4);
    assert_eq!(hub.active.load(Ordering::Acquire), 2); // existing cleanup still owns slot
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
