//! Real mTLS proves caller progress and pre-lock cancellation without replay.
use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::shutdown_blocked_tests::ControlledPeer;
use super::unknown_transit_tests::{barrier, raw, recv, send, stopped};
use super::*;
use std::time::Duration;

async fn blocked_unicast(function: u8, budget: Duration) {
    let tls = TestTls::new();
    let config = tls
        .hub_config
        .clone()
        .with_unicast_send_budget(budget)
        .unwrap();
    let running = ScHub::start("127.0.0.1:0", config, [0x10; 6], [0x10; 16])
        .await
        .unwrap();
    let mut hub = CountedHub {
        address: running.local_addr().unwrap(),
        active: running.active.clone(),
        clients: running.clients.clone(),
        admission: running.admission.clone(),
        hub: running,
    };
    let mut source = ControlledPeer::open(&tls, &hub).await;
    source.connect(0x42).await;
    let mut blocked = ControlledPeer::open(&tls, &hub).await;
    blocked.connect(0x43).await;
    let mut healthy = ControlledPeer::open(&tls, &hub).await;
    healthy.connect(0x44).await;
    let sockets = [
        Arc::downgrade(&source.sink),
        Arc::downgrade(&blocked.sink),
        Arc::downgrade(&healthy.sink),
    ];
    let held = blocked.sink.clone().lock_owned().await;
    tokio::time::pause();
    poll_io(send(
        &mut source,
        raw(function, 23, None, Some([0x43; 6]), 0, &[1, 0]),
    ))
    .await;
    // Current-thread dispatch reaches the held sink before yielding after this
    // counter. The pending future includes both sink acquisition and wire send.
    until(|| source.deadline.received.load(Ordering::Acquire) == 2).await;
    poll_io(send(
        &mut source,
        raw(function, 24, None, Some([0x44; 6]), 0, &[1, 1]),
    ))
    .await;
    tokio::time::advance(budget - Duration::from_millis(1)).await;
    assert_eq!(source.deadline.received.load(Ordering::Acquire), 2);
    tokio::time::advance(Duration::from_millis(2)).await;
    assert_eq!(
        recv(&mut healthy).await,
        raw(function, 24, Some([0x42; 6]), None, 0, &[1, 1])
    );
    {
        let clients = hub.clients.lock().await;
        let target = clients
            .get(&[0x43; 6])
            .expect("timeout alone must not retire target");
        assert!(Arc::ptr_eq(&target.sink, &blocked.sink));
        assert!(!target.closed.load(Ordering::Acquire));
    }
    // The source's ordered response detects any unsolicited Result for the
    // timed-out relay before the known barrier response.
    poll_io(barrier(&mut source)).await;
    drop(held);
    // After releasing the sink, its own ordered barrier detects a replayed first
    // frame before the expected response. This claim is only pre-lock timeout;
    // cancellation after a WebSocket write cannot retract buffered bytes.
    poll_io(barrier(&mut blocked)).await;
    poll_io(send(
        &mut source,
        raw(function, 25, None, Some([0x43; 6]), 0, &[1, 2]),
    ))
    .await;
    assert_eq!(
        recv(&mut blocked).await,
        raw(function, 25, Some([0x42; 6]), None, 0, &[1, 2])
    );
    poll_io(barrier(&mut blocked)).await;
    assert_eq!(
        hub.hub.status().await.outcomes,
        ScHubOutcomeCounts {
            unicast_send_timeout: 1,
            ..ScHubOutcomeCounts::default()
        }
    );
    stopped(&mut hub).await;
    drop((source, blocked, healthy));
    assert!(
        sockets.iter().all(|socket| socket.upgrade().is_none()),
        "stopped hub retained a sink"
    );
}

#[tokio::test]
async fn npdu_unicast_deadline_releases_source_without_retiring_or_replaying_target() {
    blocked_unicast(0x01, Duration::from_secs(5)).await;
}

#[tokio::test]
async fn opaque_unicast_deadline_releases_source_without_retiring_or_replaying_target() {
    blocked_unicast(0x0D, Duration::from_secs(5)).await;
}

#[tokio::test]
async fn npdu_unicast_custom_budget_covers_sink_acquisition() {
    blocked_unicast(0x01, Duration::from_millis(37)).await;
}

#[tokio::test]
async fn opaque_unicast_custom_budget_covers_sink_acquisition() {
    blocked_unicast(0x0D, Duration::from_millis(73)).await;
}
