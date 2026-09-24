//! Configured policy through public startup, real TLS handlers and paused time.
use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::shutdown_blocked_tests::ControlledPeer;
use super::*;
use std::time::Duration;

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}
fn policy(scan: u64, idle: u64, ack: u64, send: u64) -> ScHubProbePolicy {
    ScHubProbePolicy::new(ms(scan), ms(idle), ms(ack), ms(send)).unwrap()
}

async fn start(tls: &TestTls, probe: ScHubProbePolicy) -> CountedHub {
    let config = tls
        .hub_config
        .clone()
        .with_probe_policy(probe)
        .with_graceful_timeouts(
            ScHubGracefulTimeouts::new(
                Duration::from_secs(1),
                Duration::from_secs(1),
                Duration::from_secs(5),
            )
            .unwrap(),
        );
    let hub = poll_io(ScHub::start("127.0.0.1:0", config, [0x10; 6], [0x10; 16]))
        .await
        .unwrap();
    let counted = CountedHub {
        address: hub.local_addr().unwrap(),
        active: hub.active.clone(),
        clients: hub.clients.clone(),
        admission: hub.admission.clone(),
        hub,
    };
    until(|| counted.hub.tasks.probe_scans.load(Ordering::Acquire) == 1).await;
    counted
}

#[test]
fn checked_policy_rejects_precision_zero_and_overflow_without_normative_bounds() {
    assert_eq!(
        ScHubProbePolicy::default(),
        policy(30_000, 60_000, 5_000, 5_000)
    );
    for bad in [
        Duration::ZERO,
        Duration::from_nanos(1),
        Duration::from_nanos(1_000_001),
        Duration::MAX,
    ] {
        for index in 0..4 {
            let mut values = [ms(1); 4];
            values[index] = bad;
            assert!(ScHubProbePolicy::new(values[0], values[1], values[2], values[3]).is_err());
        }
        assert!(ScHubTlsConfig::validate_relay_send_budget(bad).is_err());
    }
    // Local probes are not constrained by initiating-node 3–300s timing.
    assert_eq!(policy(1, 300_001, 1, 1).idle_age(), ms(300_001));
    assert!(ScHubTlsConfig::validate_relay_send_budget(ms(1)).is_ok());
    assert!(ScHubTlsConfig::validate_relay_send_budget(ms(u64::MAX)).is_err());
}

#[tokio::test(start_paused = true)]
async fn configured_scan_idle_and_ack_age_are_precise_strict_exceeds() {
    let tls = TestTls::new();
    let mut hub = start(&tls, policy(10, 20, 20, 15)).await;
    let mut peer = poll_io(ControlledPeer::open(&tls, &hub)).await;
    poll_io(peer.connect(0x42)).await;
    assert_eq!(
        hub.clients.lock().await[&[0x42; 6]]
            .last_activity
            .load(Ordering::Acquire),
        0
    );
    tokio::time::advance(ms(20)).await;
    until(|| hub.hub.tasks.probe_scans.load(Ordering::Acquire) == 2).await;
    assert!(hub.clients.lock().await[&[0x42; 6]]
        .heartbeat
        .pending
        .is_none());
    tokio::time::advance(ms(10)).await;
    let frame = poll_io(peer.ws.next()).await.unwrap().unwrap();
    let Message::Binary(bytes) = frame else {
        panic!("expected probe")
    };
    let request = decode_sc_message(&bytes).unwrap();
    assert_eq!(request.function, ScFunction::HeartbeatRequest);
    assert_eq!(
        hub.clients.lock().await[&[0x42; 6]]
            .heartbeat
            .pending
            .unwrap()
            .published_at,
        30
    );
    // Wrong ACK cannot refresh activity or clear pending.
    let wrong = request.message_id.wrapping_add(1).to_be_bytes();
    poll_io(
        peer.ws
            .send(Message::Binary(vec![11, 0, wrong[0], wrong[1]].into())),
    )
    .await
    .unwrap();
    until(|| peer.deadline.received.load(Ordering::Acquire) == 2).await;
    assert_eq!(
        hub.clients.lock().await[&[0x42; 6]]
            .last_activity
            .load(Ordering::Acquire),
        0
    );
    tokio::time::advance(ms(20)).await;
    until(|| hub.hub.tasks.probe_scans.load(Ordering::Acquire) == 3).await;
    assert!(
        hub.clients.lock().await.contains_key(&[0x42; 6]),
        "exact ACK age is retained"
    );
    tokio::time::advance(ms(10)).await;
    poll_io(async {
        while hub.clients.lock().await.contains_key(&[0x42; 6]) {
            tokio::task::yield_now().await;
        }
    })
    .await;
    poll_io(hub.hub.stop()).await;
    assert_eq!(hub.hub.tasks.len(), 0);
}

#[tokio::test(start_paused = true)]
async fn slow_sweep_skips_missed_ticks_instead_of_catch_up_burst() {
    let tls = TestTls::new();
    let mut hub = start(&tls, policy(10, 20, 500, 65)).await;
    let mut peer = poll_io(ControlledPeer::open(&tls, &hub)).await;
    poll_io(peer.connect(0x42)).await;
    let held = peer.sink.clone().lock_owned().await;
    tokio::time::advance(ms(30)).await;
    until(|| hub.hub.tasks.probe_scans.load(Ordering::Acquire) == 2).await;
    assert!(hub.clients.lock().await[&[0x42; 6]]
        .heartbeat
        .pending
        .is_some());
    tokio::time::advance(ms(64)).await;
    assert!(hub.clients.lock().await.contains_key(&[0x42; 6]));
    tokio::time::advance(ms(36)).await;
    until(|| hub.hub.tasks.probe_scans.load(Ordering::Acquire) >= 3).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        hub.hub.tasks.probe_scans.load(Ordering::Acquire),
        3,
        "missed ticks were replayed"
    );
    assert!(!hub.clients.lock().await.contains_key(&[0x42; 6]));
    tokio::time::advance(ms(10)).await;
    until(|| hub.hub.tasks.probe_scans.load(Ordering::Acquire) == 4).await;
    poll_io(hub.hub.stop()).await;
    drop(held);
}

#[tokio::test(start_paused = true)]
async fn configured_blocked_probe_is_owned_by_forceful_and_graceful_shutdown() {
    for graceful in [false, true] {
        let tls = TestTls::new();
        let mut hub = start(&tls, policy(10, 20, 20_000, 10_000)).await;
        let mut peer = poll_io(ControlledPeer::open(&tls, &hub)).await;
        poll_io(peer.connect(0x42)).await;
        let held = peer.sink.clone().lock_owned().await;
        tokio::time::advance(ms(30)).await;
        until(|| hub.hub.tasks.probe_scans.load(Ordering::Acquire) == 2).await;
        assert!(hub.clients.lock().await[&[0x42; 6]]
            .heartbeat
            .pending
            .is_some());
        if graceful {
            let stop = hub.hub.shutdown_gracefully();
            tokio::pin!(stop);
            assert!(futures_util::poll!(&mut stop).is_pending());
            for _ in 0..20 {
                tokio::task::yield_now().await;
            }
            tokio::time::advance(Duration::from_secs(5)).await;
            assert_eq!(poll_io(stop).await, ScHubShutdownOutcome::Forced);
        } else {
            poll_io(hub.hub.stop()).await;
        }
        assert_eq!(hub.active.load(Ordering::Acquire), 0);
        assert_eq!(hub.hub.tasks.len(), 0);
        assert!(hub.clients.lock().await.is_empty());
        drop(held);
    }
}
