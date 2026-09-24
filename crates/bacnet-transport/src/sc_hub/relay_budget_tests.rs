//! Real TLS exercises each transit budget with independently blocked recipients.
use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::shutdown_blocked_tests::ControlledPeer;
use super::unknown_transit_tests::{barrier, raw, recv, send, stopped};
use super::*;
use crate::sc_frame::BROADCAST_VMAC;
use std::time::Duration;

async fn start(tls: &TestTls, budget: Duration) -> CountedHub {
    let config = tls
        .hub_config
        .clone()
        .with_relay_send_budget(budget)
        .unwrap();
    let hub = ScHub::start("127.0.0.1:0", config, [0x10; 6], [0x10; 16])
        .await
        .unwrap();
    CountedHub {
        address: hub.local_addr().unwrap(),
        active: hub.active.clone(),
        clients: hub.clients.clone(),
        admission: hub.admission.clone(),
        hub,
    }
}

async fn broadcast_budget(function: u8) {
    let tls = TestTls::new();
    let budget = Duration::from_millis(41);
    let mut hub = start(&tls, budget).await;
    let mut source = ControlledPeer::open(&tls, &hub).await;
    source.connect(0x42).await;
    let mut first = ControlledPeer::open(&tls, &hub).await;
    first.connect(0x43).await;
    let mut second = ControlledPeer::open(&tls, &hub).await;
    second.connect(0x44).await;
    let mut healthy = ControlledPeer::open(&tls, &hub).await;
    healthy.connect(0x45).await;
    let sockets = [
        Arc::downgrade(&source.sink),
        Arc::downgrade(&first.sink),
        Arc::downgrade(&second.sink),
        Arc::downgrade(&healthy.sink),
    ];
    let first_held = first.sink.clone().lock_owned().await;
    let second_held = second.sink.clone().lock_owned().await;
    tokio::time::pause();
    let started = tokio::time::Instant::now();
    poll_io(send(
        &mut source,
        raw(function, 23, None, Some(BROADCAST_VMAC), 0, &[1, 0]),
    ))
    .await;
    assert_eq!(
        recv(&mut healthy).await,
        raw(
            function,
            23,
            Some([0x42; 6]),
            Some(BROADCAST_VMAC),
            0,
            &[1, 0]
        )
    );
    assert_eq!(
        tokio::time::Instant::now(),
        started,
        "healthy fanout waited for held sinks"
    );
    poll_io(send(&mut source, vec![8, 0, 0x55, 0x66, 0x42])).await;
    tokio::time::advance(budget - Duration::from_millis(1)).await;
    assert_eq!(source.deadline.received.load(Ordering::Acquire), 2);
    tokio::time::advance(Duration::from_millis(2)).await;
    until(|| source.deadline.received.load(Ordering::Acquire) == 3).await;
    assert_eq!(
        recv(&mut source).await,
        [0, 0, 0x55, 0x66, 8, 1, 0, 0, 7, 0, 7]
    );
    // One concurrent budget, not two serial budgets; no timeout retirement.
    for peer in [&first, &second] {
        let map = hub.clients.lock().await;
        assert!(map
            .values()
            .any(|c| Arc::ptr_eq(&c.sink, &peer.sink) && !c.closed.load(Ordering::Acquire)));
    }
    drop((first_held, second_held));
    poll_io(barrier(&mut first)).await;
    poll_io(barrier(&mut second)).await; // no replay from either timed-out attempt
    poll_io(send(
        &mut source,
        raw(function, 24, None, Some(BROADCAST_VMAC), 0, &[1, 1]),
    ))
    .await;
    let expected = raw(
        function,
        24,
        Some([0x42; 6]),
        Some(BROADCAST_VMAC),
        0,
        &[1, 1],
    );
    for peer in [&mut first, &mut second, &mut healthy] {
        assert_eq!(recv(peer).await, expected);
    }
    poll_io(barrier(&mut source)).await;
    assert_eq!(
        hub.hub.status().await.outcomes,
        ScHubOutcomeCounts::default(),
        "broadcast remains outside unicast counters"
    );
    stopped(&mut hub).await;
    drop((source, first, second, healthy));
    assert!(sockets.iter().all(|s| s.upgrade().is_none()));
}

#[tokio::test]
async fn npdu_broadcast_budget_preserves_parallel_fanout_and_later_delivery() {
    broadcast_budget(1).await;
}

#[tokio::test]
async fn opaque_broadcast_budget_preserves_parallel_fanout_and_later_delivery() {
    broadcast_budget(13).await;
}

#[tokio::test]
async fn forwarded_result_budget_releases_source_without_replay_or_retirement() {
    let tls = TestTls::new();
    let budget = Duration::from_millis(37);
    let mut hub = start(&tls, budget).await;
    let mut source = ControlledPeer::open(&tls, &hub).await;
    source.connect(0x42).await;
    let mut target = ControlledPeer::open(&tls, &hub).await;
    target.connect(0x43).await;
    let sockets = [Arc::downgrade(&source.sink), Arc::downgrade(&target.sink)];
    let held = target.sink.clone().lock_owned().await;
    tokio::time::pause();
    poll_io(send(
        &mut source,
        raw(0, 23, None, Some([0x43; 6]), 0, &[2, 0]),
    ))
    .await;
    until(|| source.deadline.received.load(Ordering::Acquire) == 2).await;
    poll_io(send(&mut source, vec![8, 0, 0x55, 0x66, 0x42])).await;
    tokio::time::advance(budget - Duration::from_millis(1)).await;
    assert_eq!(source.deadline.received.load(Ordering::Acquire), 2);
    tokio::time::advance(Duration::from_millis(2)).await;
    until(|| source.deadline.received.load(Ordering::Acquire) == 3).await;
    assert_eq!(
        recv(&mut source).await,
        [0, 0, 0x55, 0x66, 8, 1, 0, 0, 7, 0, 7]
    );
    {
        let map = hub.clients.lock().await;
        let current = map.get(&[0x43; 6]).unwrap();
        assert!(Arc::ptr_eq(&current.sink, &target.sink));
        assert!(!current.closed.load(Ordering::Acquire));
    }
    drop(held);
    poll_io(barrier(&mut target)).await;
    poll_io(send(
        &mut source,
        raw(0, 24, None, Some([0x43; 6]), 0, &[2, 0]),
    ))
    .await;
    assert_eq!(
        recv(&mut target).await,
        raw(0, 24, Some([0x42; 6]), None, 0, &[2, 0])
    );
    poll_io(barrier(&mut source)).await;
    assert_eq!(
        hub.hub.status().await.outcomes,
        ScHubOutcomeCounts::default(),
        "Result remains outside unicast counters"
    );
    stopped(&mut hub).await;
    drop((source, target));
    assert!(sockets.iter().all(|s| s.upgrade().is_none()));
}

#[tokio::test]
async fn broadcast_capture_cannot_relay_into_or_retire_replacement_during_sink_wait() {
    let tls = TestTls::new();
    let mut hub = start(&tls, Duration::from_millis(41)).await;
    let mut source = ControlledPeer::open(&tls, &hub).await;
    source.connect(0x42).await;
    let mut old = ControlledPeer::open(&tls, &hub).await;
    old.connect(0x43).await;
    let mut healthy = ControlledPeer::open(&tls, &hub).await;
    healthy.connect(0x44).await;
    let old_socket = Arc::downgrade(&old.sink);
    let held = old.sink.clone().lock_owned().await;
    tokio::time::pause();
    let started = tokio::time::Instant::now();
    poll_io(send(
        &mut source,
        raw(1, 23, None, Some(BROADCAST_VMAC), 0, &[1, 0]),
    ))
    .await;
    assert_eq!(
        recv(&mut healthy).await,
        raw(1, 23, Some([0x42; 6]), Some(BROADCAST_VMAC), 0, &[1, 0])
    );
    let mut replacement = poll_io(ControlledPeer::open(&tls, &hub)).await;
    poll_io(replacement.connect(0x43)).await;
    poll_io(barrier(&mut source)).await; // retirement notification releases old capture
    assert_eq!(tokio::time::Instant::now(), started);
    drop(held);
    poll_io(barrier(&mut replacement)).await; // no first frame leaked to the replacement
    {
        let map = hub.clients.lock().await;
        let current = map.get(&[0x43; 6]).unwrap();
        assert!(Arc::ptr_eq(&current.sink, &replacement.sink));
        assert!(!current.closed.load(Ordering::Acquire));
    }
    poll_io(send(
        &mut source,
        raw(1, 24, None, Some(BROADCAST_VMAC), 0, &[1, 1]),
    ))
    .await;
    let expected = raw(1, 24, Some([0x42; 6]), Some(BROADCAST_VMAC), 0, &[1, 1]);
    assert_eq!(recv(&mut replacement).await, expected);
    assert_eq!(recv(&mut healthy).await, expected);
    poll_io(barrier(&mut source)).await;
    assert_eq!(
        hub.hub.status().await.outcomes,
        ScHubOutcomeCounts {
            uuid_replacements: 1,
            ..ScHubOutcomeCounts::default()
        }
    );
    stopped(&mut hub).await;
    drop((source, old, replacement, healthy));
    assert!(old_socket.upgrade().is_none());
}

#[tokio::test]
async fn concurrent_tls_peer_close_wave_reclaims_capacity_and_joins_shutdown() {
    const PEERS: usize = 16;
    let tls = TestTls::new();
    let mut hub = CountedHub::start_with_limits(
        &tls,
        ScHubHandshakeTimeouts::default(),
        ScHubAdmissionLimits::new(PEERS, PEERS).unwrap(),
    )
    .await;
    let mut peers = Vec::new();
    for id in 0x40..0x40 + PEERS as u8 {
        let mut peer = tls.websocket(hub.address).await;
        peer.send(request([id; 6], [id; 16])).await.unwrap();
        assert!(
            matches!(poll_io(peer.next()).await, Some(Ok(Message::Binary(data))) if data[0] == 7)
        );
        peers.push(peer);
    }
    assert_eq!(hub.active.load(Ordering::Acquire), PEERS);
    let sockets: Vec<_> = {
        let map = hub.clients.lock().await;
        assert_eq!(map.len(), PEERS);
        map.values().map(|c| Arc::downgrade(&c.sink)).collect()
    };
    // Poll all peer-initiated Close sends together. The oracle is server-owned
    // resource reclamation, not a graceful TLS close-notify exchange.
    poll_io(futures_util::future::join_all(peers.iter_mut().map(
        |peer| async move {
            peer.close(None).await.unwrap();
        },
    )))
    .await;
    until(|| hub.active.load(Ordering::Acquire) == 0).await;
    until(|| hub.hub.tasks.len() == 1).await; // only the Hub probe worker remains
    assert!(hub.clients.lock().await.is_empty());
    assert!(sockets.iter().all(|s| s.upgrade().is_none()));
    let mut recovery = tls.websocket(hub.address).await;
    recovery.send(request([0x70; 6], [0x70; 16])).await.unwrap();
    assert!(
        matches!(poll_io(recovery.next()).await, Some(Ok(Message::Binary(data))) if data[0] == 7)
    );
    assert_eq!(hub.hub.status().await.client_count, 1);
    recovery
        .send(Message::Binary(vec![10, 0, 0x66, 0x77].into()))
        .await
        .unwrap();
    assert!(
        matches!(poll_io(recovery.next()).await, Some(Ok(Message::Binary(data))) if data.as_ref() == [11, 0, 0x66, 0x77])
    );
    stopped(&mut hub).await;
    assert_eq!(
        hub.hub.status().await.outcomes,
        ScHubOutcomeCounts::default()
    );
}
