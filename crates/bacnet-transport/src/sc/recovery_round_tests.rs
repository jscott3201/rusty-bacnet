//! Observed reconnect rounds, including exhaustion and unlimited recovery.

use super::*;

/// Run one reconnect episode for a lost `hub` whose socket can't be reused,
/// with no failover connector, and collect its events.
async fn episode(
    hub: ScActiveHub,
    primary: Option<crate::sc::WebSocketConnector<LoopbackWebSocket>>,
    mut spare: Option<Arc<LoopbackWebSocket>>,
    retry_forever: bool,
) -> Vec<ScEvent> {
    use crate::sc::recovery::Recovery;
    use crate::sc::{HubLimits, ScConnection};
    use tokio::sync::{Mutex, watch};
    let (client, _hub) = LoopbackWebSocket::pair();
    let ws = Arc::new(client);
    let conn = Arc::new(Mutex::new(ScConnection::new([0x22; 6], [1; 16])));
    let active_ws = Arc::new(Mutex::new(Some(ws.clone())));
    let (state_tx, _) = watch::channel(ScConnectionState::Disconnected);
    let mut throttle = crate::sc::diagnostic_throttle::DiagnosticThrottle::new();
    let config = ScReconnectConfig {
        initial_delay_ms: 25,
        max_delay_ms: 25,
        max_retries: 3,
        retry_forever,
    };
    let observer = ScObserver::new();
    let mut events = observer.subscribe();
    let limits = HubLimits::new();
    let mut recovery = Recovery {
        config: &config,
        diagnostic_throttle: &mut throttle,
        primary_connector: &primary,
        failover_connector: &None,
        failover_ws: &mut spare,
        conn: &conn,
        active_ws: &active_ws,
        state_tx: &state_tx,
        connect_timeout_ms: 100,
        hub_limits: &limits,
        observer: &observer,
    };
    if let Some((replacement, _)) = recovery.reconnect(&ws, hub, false).await {
        assert!(!Arc::ptr_eq(&replacement, &ws), "reused a retired socket");
    }
    assert!(recovery.failover_ws.is_none());
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event);
    }
    seen
}

/// A lost failover hub that can't be redialled drops out of the episode;
/// the primary keeps getting every attempt (Annex AB.5.2).
#[tokio::test(start_paused = true)]
async fn a_dead_failover_target_leaves_the_primary_in_every_attempt() {
    let (tx, _hubs) = mpsc::unbounded_channel();
    let down = connector(Arc::new(AtomicBool::new(false)), tx);
    let seen = episode(ScActiveHub::Failover, Some(Arc::new(down)), None, false).await;
    let mut expected = Vec::new();
    for attempt in 1..=3 {
        expected.push(("attempt", attempt));
        expected.push(("failed", attempt));
    }
    let mut attempt = 0;
    let got: Vec<_> = seen
        .iter()
        .take(6)
        .map(|event| match event {
            ScEvent::ReconnectAttempt {
                hub: ScActiveHub::Primary,
                attempt: a,
                ..
            } => {
                attempt = *a;
                ("attempt", *a)
            }
            ScEvent::ConnectFailed {
                hub: ScActiveHub::Primary,
                ..
            } => ("failed", attempt),
            other => panic!("unexpected {other:?} in {seen:?}"),
        })
        .collect();
    assert_eq!(got, expected);
    assert_eq!(seen[6..], [ScEvent::ReconnectGaveUp { attempts: 3 }]);
}

/// With nothing to dial, the episode gives up with no attempt counted.
#[tokio::test(start_paused = true)]
async fn giving_up_without_a_dial_counts_no_attempts() {
    let seen = episode(ScActiveHub::Primary, None, None, false).await;
    assert_eq!(seen, [ScEvent::ReconnectGaveUp { attempts: 0 }]);
}

fn unlimited(max_retries: u32) -> ScReconnectConfig {
    ScReconnectConfig {
        initial_delay_ms: 25,
        max_delay_ms: 25,
        max_retries,
        retry_forever: true,
    }
}

async fn attempt(events: &mut broadcast::Receiver<ScEvent>, hub: ScActiveHub, round: u32) {
    assert_eq!(
        next(events).await,
        ScEvent::ReconnectAttempt {
            hub,
            attempt: round,
            backoff: Duration::from_millis(25),
        }
    );
}

async fn failed(events: &mut broadcast::Receiver<ScEvent>, expected: ScActiveHub) {
    assert!(matches!(next(events).await, ScEvent::ConnectFailed { hub, .. } if hub == expected));
}

#[tokio::test(start_paused = true)]
async fn each_round_tries_both_hubs_and_a_late_failover_serves() {
    use ScActiveHub::{Failover, Primary};
    let (client, hub) = LoopbackWebSocket::pair();
    let (primary_tx, _primary_hubs) = mpsc::unbounded_channel();
    let (failover_tx, mut failover_hubs) = mpsc::unbounded_channel();
    let available = Arc::new(AtomicBool::new(false));
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_reconnect(unlimited(u32::MAX))
        .with_connector(connector(Arc::new(AtomicBool::new(false)), primary_tx))
        .with_failover_connector(connector(available.clone(), failover_tx));
    let mut events = transport.connection_events();
    let (rx, ()) = tokio::join!(transport.start(), hub_accept(&hub, PRIMARY));
    let _rx = rx.unwrap();
    assert_eq!(next(&mut events).await, connected(Primary, PRIMARY));
    hub_closes(hub, &mut events).await;
    let start = tokio::time::Instant::now();
    for round in 1..=6 {
        attempt(&mut events, Primary, round).await;
        failed(&mut events, Primary).await;
        attempt(&mut events, Failover, round).await;
        failed(&mut events, Failover).await;
        assert_eq!(
            start.elapsed(),
            Duration::from_millis(25 * u64::from(round))
        );
    }
    available.store(true, Ordering::SeqCst);
    attempt(&mut events, Primary, 7).await;
    failed(&mut events, Primary).await;
    attempt(&mut events, Failover, 7).await;
    assert_eq!(next(&mut events).await, ScEvent::FailoverEngaged);
    let hub = failover_hubs.recv().await.unwrap();
    hub_accept(&hub, FAILOVER).await;
    assert_eq!(next(&mut events).await, connected(Failover, FAILOVER));
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn primary_only_recovers_after_more_rounds_than_the_numeric_budget() {
    use ScActiveHub::Primary;
    let (tx, mut hubs) = mpsc::unbounded_channel();
    let up = Arc::new(AtomicBool::new(false));
    let (transport, _rx, hub, mut events) = started(unlimited(0), connector(up.clone(), tx)).await;
    hub_closes(hub, &mut events).await;
    for round in 1..=4 {
        attempt(&mut events, Primary, round).await;
        failed(&mut events, Primary).await;
    }
    up.store(true, Ordering::SeqCst);
    attempt(&mut events, Primary, 5).await;
    let hub = hubs.recv().await.unwrap();
    hub_accept(&hub, PRIMARY).await;
    assert_eq!(next(&mut events).await, connected(Primary, PRIMARY));
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn stopping_during_backoff_prevents_further_dials() {
    let (tx, mut hubs) = mpsc::unbounded_channel();
    let (transport, _rx, hub, mut events) =
        started(unlimited(0), connector(Arc::new(AtomicBool::new(true)), tx)).await;
    hub_closes(hub, &mut events).await;
    transport.stop().await.unwrap();
    tokio::time::advance(Duration::from_secs(60)).await;
    assert!(hubs.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn a_retired_primary_and_one_unanswered_spare_leave_no_targets() {
    let (spare, _silent_hub) = LoopbackWebSocket::pair();
    let seen = timeout(
        Duration::from_secs(1),
        episode(ScActiveHub::Primary, None, Some(Arc::new(spare)), true),
    )
    .await
    .expect("recovery must finish once it consumes the spare");
    assert!(matches!(
        seen.as_slice(),
        [
            ScEvent::ReconnectAttempt {
                hub: ScActiveHub::Failover,
                attempt: 1,
                ..
            },
            ScEvent::FailoverEngaged,
            ScEvent::ConnectFailed {
                hub: ScActiveHub::Failover,
                ..
            },
            ScEvent::ReconnectGaveUp { attempts: 1 }
        ]
    ));
}

#[tokio::test(start_paused = true)]
async fn a_retired_failover_can_connect_on_its_unused_spare() {
    let (spare, hub) = LoopbackWebSocket::pair();
    let (seen, ()) = timeout(Duration::from_secs(1), async {
        tokio::join!(
            episode(ScActiveHub::Failover, None, Some(Arc::new(spare)), true),
            hub_accept(&hub, FAILOVER)
        )
    })
    .await
    .expect("the unused spare must receive a Connect-Request");
    assert_eq!(
        seen,
        [
            ScEvent::ReconnectAttempt {
                hub: ScActiveHub::Failover,
                attempt: 1,
                backoff: Duration::from_millis(25)
            },
            connected(ScActiveHub::Failover, FAILOVER)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_retry_forbidden_reseed_failure_ends_unlimited_recovery() {
    let _guard = crate::sc::set_test_random48_vmac_generator(|| {
        Err(Error::Encoding("fixture entropy unavailable".into()))
    });
    let (tx, mut hubs) = mpsc::unbounded_channel();
    let (transport, _rx, hub, mut events) =
        started(unlimited(0), connector(Arc::new(AtomicBool::new(true)), tx)).await;
    hub_closes(hub, &mut events).await;
    attempt(&mut events, ScActiveHub::Primary, 1).await;
    let hub = hubs.recv().await.unwrap();
    let request = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
    send(
        &hub,
        &nak(
            request.message_id,
            ScFunction::ConnectRequest,
            ErrorCode::NODE_DUPLICATE_VMAC,
        ),
    )
    .await;
    failed(&mut events, ScActiveHub::Primary).await;
    assert_eq!(
        next(&mut events).await,
        ScEvent::ReconnectGaveUp { attempts: 1 }
    );
    transport.stop().await.unwrap();
}
