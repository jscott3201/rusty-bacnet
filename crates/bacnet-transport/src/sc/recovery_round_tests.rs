//! Observed reconnect rounds: each waits once, then tries the primary and
//! the failover hub, bounded or unlimited.

use super::*;
use crate::sc::recovery::Recovery;
use crate::sc::{HubLimits, ScConnection, WebSocketConnector};
use std::sync::atomic::AtomicUsize;
use tokio::sync::{Mutex, watch};

/// The delay of every round here, unlimited recovery's floor.
const ROUND: Duration = Duration::from_millis(1_000);

fn policy(max_retries: Option<u32>) -> ScReconnectConfig {
    ScReconnectConfig {
        initial_delay_ms: 1_000,
        max_delay_ms: 1_000,
        max_retries,
    }
}

/// The hubs one [`episode`] can reach.
#[derive(Default)]
struct Hubs {
    primary: Option<WebSocketConnector<LoopbackWebSocket>>,
    failover: Option<WebSocketConnector<LoopbackWebSocket>>,
    spare: Option<Arc<LoopbackWebSocket>>,
}

/// What one [`episode`] saw: its events, and whether the spare is left.
struct Episode {
    events: Vec<ScEvent>,
    spare_left: bool,
    reused: bool,
}

/// Run one reconnect episode after losing `lost` on `ws` and collect its
/// events. `reusable` says whether the lost socket may get a Connect again.
async fn episode(
    lost: ScActiveHub,
    ws: Arc<LoopbackWebSocket>,
    reusable: bool,
    hubs: Hubs,
    max_retries: Option<u32>,
    conn: Arc<Mutex<ScConnection>>,
) -> Episode {
    let Hubs {
        primary,
        failover,
        mut spare,
    } = hubs;
    let active_ws = Arc::new(Mutex::new(Some(ws.clone())));
    let (state_tx, _) = watch::channel(ScConnectionState::Disconnected);
    let mut throttle = crate::sc::diagnostic_throttle::DiagnosticThrottle::new();
    let config = policy(max_retries);
    let observer = ScObserver::new();
    let mut events = observer.subscribe();
    let limits = HubLimits::new();
    let mut recovery = Recovery {
        config: &config,
        diagnostic_throttle: &mut throttle,
        primary_connector: &primary,
        failover_connector: &failover,
        failover_ws: &mut spare,
        primary_ws: &mut None,
        conn: &conn,
        active_ws: &active_ws,
        state_tx: &state_tx,
        connect_timeout_ms: 100,
        hub_limits: &limits,
        observer: &observer,
    };
    let reused = match recovery.reconnect(&ws, lost, reusable).await {
        Some((replacement, _)) => Arc::ptr_eq(&replacement, &ws),
        None => false,
    };
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event);
    }
    Episode {
        events: seen,
        spare_left: spare.is_some(),
        reused,
    }
}

fn fresh_conn() -> Arc<Mutex<ScConnection>> {
    Arc::new(Mutex::new(ScConnection::new([0x22; 6], [1; 16])))
}

/// A lost socket that can't be reused, with nothing else to dial.
fn retired() -> Arc<LoopbackWebSocket> {
    Arc::new(LoopbackWebSocket::pair().0)
}

/// A connector that fails every dial and counts them.
fn down(dials: &Arc<AtomicUsize>) -> WebSocketConnector<LoopbackWebSocket> {
    let dials = dials.clone();
    Arc::new(move || -> Dial {
        dials.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Err(Error::Transport(std::io::Error::other(
                "fixture hub is down",
            )))
        })
    })
}

fn attempt_event(hub: ScActiveHub, round: u32) -> ScEvent {
    ScEvent::ReconnectAttempt {
        hub,
        attempt: round,
        backoff: ROUND,
    }
}

async fn attempt(events: &mut broadcast::Receiver<ScEvent>, hub: ScActiveHub, round: u32) {
    assert_eq!(next(events).await, attempt_event(hub, round));
}

async fn failed(events: &mut broadcast::Receiver<ScEvent>, expected: ScActiveHub) {
    assert!(matches!(next(events).await, ScEvent::ConnectFailed { hub, .. } if hub == expected));
}

fn is_failed(event: &ScEvent, expected: ScActiveHub) -> bool {
    matches!(event, ScEvent::ConnectFailed { hub, .. } if *hub == expected)
}

/// A lost failover hub that can't be redialled drops out of the episode;
/// the primary keeps getting every round (Annex AB.5.2).
#[tokio::test(start_paused = true)]
async fn a_dead_failover_target_leaves_the_primary_in_every_round() {
    let dials = Arc::new(AtomicUsize::new(0));
    let hubs = Hubs {
        primary: Some(down(&dials)),
        ..Hubs::default()
    };
    let seen = episode(
        ScActiveHub::Failover,
        retired(),
        false,
        hubs,
        Some(3),
        fresh_conn(),
    )
    .await
    .events;
    assert_eq!(seen.len(), 7, "{seen:?}");
    for round in 1..=3u32 {
        let at = 2 * (round as usize - 1);
        assert_eq!(seen[at], attempt_event(ScActiveHub::Primary, round));
        assert!(is_failed(&seen[at + 1], ScActiveHub::Primary), "{seen:?}");
    }
    assert_eq!(seen[6], ScEvent::ReconnectGaveUp { attempts: 3 });
    assert_eq!(dials.load(Ordering::SeqCst), 3);
}

/// With nothing to dial, the episode gives up with no attempt counted.
#[tokio::test(start_paused = true)]
async fn giving_up_without_a_dial_counts_no_attempts() {
    for max_retries in [Some(3), None] {
        let seen = episode(
            ScActiveHub::Primary,
            retired(),
            false,
            Hubs::default(),
            max_retries,
            fresh_conn(),
        )
        .await
        .events;
        assert_eq!(seen, [ScEvent::ReconnectGaveUp { attempts: 0 }]);
    }
}

/// Bounded recovery after losing the primary: every round tries the primary,
/// then the failover hub, with one sleep per round, so the failover is tried
/// in round 1 (Annex AB.5.2), and the episode ends after `max_retries`.
#[tokio::test(start_paused = true)]
async fn bounded_rounds_try_both_hubs_from_the_first_round() {
    let primary_dials = Arc::new(AtomicUsize::new(0));
    let failover_dials = Arc::new(AtomicUsize::new(0));
    let hubs = Hubs {
        primary: Some(down(&primary_dials)),
        failover: Some(down(&failover_dials)),
        spare: None,
    };
    let start = tokio::time::Instant::now();
    let seen = episode(
        ScActiveHub::Primary,
        retired(),
        false,
        hubs,
        Some(2),
        fresh_conn(),
    )
    .await
    .events;
    assert_eq!(start.elapsed(), 2 * ROUND);
    assert_eq!(seen.len(), 9, "{seen:?}");
    for round in 1..=2u32 {
        let at = 4 * (round as usize - 1);
        assert_eq!(seen[at], attempt_event(ScActiveHub::Primary, round));
        assert!(is_failed(&seen[at + 1], ScActiveHub::Primary), "{seen:?}");
        assert_eq!(seen[at + 2], attempt_event(ScActiveHub::Failover, round));
        assert!(is_failed(&seen[at + 3], ScActiveHub::Failover), "{seen:?}");
    }
    assert_eq!(seen[8], ScEvent::ReconnectGaveUp { attempts: 2 });
    assert_eq!(primary_dials.load(Ordering::SeqCst), 2);
    assert_eq!(failover_dials.load(Ordering::SeqCst), 2);
}

/// `Some(0)` turns recovery off: no sleep, no dial, and the terminal event
/// at once.
#[tokio::test(start_paused = true)]
async fn zero_rounds_give_up_at_once_without_a_dial() {
    let (tx, mut hubs) = mpsc::unbounded_channel();
    let up = Arc::new(AtomicBool::new(true));
    let (transport, _rx, hub, mut events) = started(policy(Some(0)), connector(up, tx)).await;
    hub_closes(hub, &mut events).await;
    let start = tokio::time::Instant::now();
    assert_eq!(
        next(&mut events).await,
        ScEvent::ReconnectGaveUp { attempts: 0 }
    );
    assert_eq!(start.elapsed(), Duration::ZERO);
    tokio::time::advance(60 * ROUND).await;
    assert!(hubs.try_recv().is_err(), "Some(0) must not dial");
    transport.stop().await.unwrap();
}

/// Both bounded and unlimited recovery: the failover hub comes up late and
/// serves in the round where its dial first succeeds.
#[tokio::test(start_paused = true)]
async fn each_round_tries_both_hubs_and_a_late_failover_serves() {
    for max_retries in [Some(10), None] {
        late_failover_serves(max_retries).await;
    }
}

async fn late_failover_serves(max_retries: Option<u32>) {
    use ScActiveHub::{Failover, Primary};
    let (client, hub) = LoopbackWebSocket::pair();
    let (primary_tx, _primary_hubs) = mpsc::unbounded_channel();
    let (failover_tx, mut failover_hubs) = mpsc::unbounded_channel();
    let available = Arc::new(AtomicBool::new(false));
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_reconnect(policy(max_retries))
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
        assert_eq!(start.elapsed(), ROUND * round);
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

/// Unlimited recovery: in round 1 the primary fails and the failover hub,
/// already up, serves, with no wait beyond the round's one backoff.
#[tokio::test(start_paused = true)]
async fn the_failover_serves_in_round_one() {
    use ScActiveHub::{Failover, Primary};
    for max_retries in [Some(1), None] {
        let (client, hub) = LoopbackWebSocket::pair();
        let (primary_tx, _primary_hubs) = mpsc::unbounded_channel();
        let (failover_tx, mut failover_hubs) = mpsc::unbounded_channel();
        let mut transport = ScTransport::new(client, [1; 6])
            .with_device_uuid([1; 16])
            .with_reconnect(policy(max_retries))
            .with_connector(connector(Arc::new(AtomicBool::new(false)), primary_tx))
            .with_failover_connector(connector(Arc::new(AtomicBool::new(true)), failover_tx));
        let mut events = transport.connection_events();
        let (rx, ()) = tokio::join!(transport.start(), hub_accept(&hub, PRIMARY));
        let _rx = rx.unwrap();
        assert_eq!(next(&mut events).await, connected(Primary, PRIMARY));
        hub_closes(hub, &mut events).await;
        let start = tokio::time::Instant::now();
        attempt(&mut events, Primary, 1).await;
        failed(&mut events, Primary).await;
        attempt(&mut events, Failover, 1).await;
        assert_eq!(next(&mut events).await, ScEvent::FailoverEngaged);
        let hub = failover_hubs.recv().await.unwrap();
        hub_accept(&hub, FAILOVER).await;
        assert_eq!(next(&mut events).await, connected(Failover, FAILOVER));
        assert_eq!(start.elapsed(), ROUND);
        transport.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn primary_only_recovers_after_more_rounds_than_a_bounded_budget() {
    use ScActiveHub::Primary;
    let (tx, mut hubs) = mpsc::unbounded_channel();
    let up = Arc::new(AtomicBool::new(false));
    let (transport, _rx, hub, mut events) = started(policy(None), connector(up.clone(), tx)).await;
    hub_closes(hub, &mut events).await;
    for round in 1..=12 {
        attempt(&mut events, Primary, round).await;
        failed(&mut events, Primary).await;
    }
    up.store(true, Ordering::SeqCst);
    attempt(&mut events, Primary, 13).await;
    let hub = hubs.recv().await.unwrap();
    hub_accept(&hub, PRIMARY).await;
    assert_eq!(next(&mut events).await, connected(Primary, PRIMARY));
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn stopping_during_backoff_prevents_further_dials() {
    let (tx, mut hubs) = mpsc::unbounded_channel();
    let (transport, _rx, hub, mut events) =
        started(policy(None), connector(Arc::new(AtomicBool::new(true)), tx)).await;
    hub_closes(hub, &mut events).await;
    transport.stop().await.unwrap();
    tokio::time::advance(60 * ROUND).await;
    assert!(hubs.try_recv().is_err());
}

/// Unlimited recovery with an initial delay under one second is refused at
/// `start`, before any dial or state change.
#[tokio::test(start_paused = true)]
async fn start_refuses_unlimited_recovery_under_one_second() {
    let (client, _hub) = LoopbackWebSocket::pair();
    let dials = Arc::new(AtomicUsize::new(0));
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 999,
            ..policy(None)
        })
        .with_connector({
            let dials = dials.clone();
            move || -> Dial {
                dials.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Err(Error::Transport(std::io::Error::other("unexpected dial"))) })
            }
        });
    let state = transport.connection_state_changes();
    match transport.start().await {
        Err(Error::InvalidConfig(message)) => {
            assert!(message.to_string().contains("at least 1000"), "{message}")
        }
        other => panic!("expected the unlimited floor, got {:?}", other.err()),
    }
    assert_eq!(dials.load(Ordering::SeqCst), 0);
    assert!(!state.has_changed().unwrap());
}

/// The spare is used once: a retired primary and an unanswered spare leave
/// nothing to dial, so even unlimited recovery ends.
#[tokio::test(start_paused = true)]
async fn a_retired_primary_and_one_unanswered_spare_leave_no_targets() {
    let (spare, _silent_hub) = LoopbackWebSocket::pair();
    let hubs = Hubs {
        spare: Some(Arc::new(spare)),
        ..Hubs::default()
    };
    let seen = timeout(
        Duration::from_secs(3_600),
        episode(
            ScActiveHub::Primary,
            retired(),
            false,
            hubs,
            None,
            fresh_conn(),
        ),
    )
    .await
    .expect("recovery must finish once it consumes the spare");
    assert!(!seen.spare_left);
    assert!(
        matches!(
            seen.events.as_slice(),
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
        ),
        "{:?}",
        seen.events
    );
}

#[tokio::test(start_paused = true)]
async fn a_retired_failover_can_connect_on_its_unused_spare() {
    let (spare, hub) = LoopbackWebSocket::pair();
    let hubs = Hubs {
        spare: Some(Arc::new(spare)),
        ..Hubs::default()
    };
    let (seen, ()) = timeout(Duration::from_secs(3_600), async {
        tokio::join!(
            episode(
                ScActiveHub::Failover,
                retired(),
                false,
                hubs,
                None,
                fresh_conn()
            ),
            hub_accept(&hub, FAILOVER)
        )
    })
    .await
    .expect("the unused spare must receive a Connect-Request");
    assert!(!seen.spare_left);
    assert_eq!(
        seen.events,
        [
            attempt_event(ScActiveHub::Failover, 1),
            connected(ScActiveHub::Failover, FAILOVER)
        ]
    );
}

/// With both a failover connector and a spare, a failed connector dial falls
/// back to the spare in the same attempt. The dial's failure is reported on
/// its own, so one attempt reads two `ConnectFailed` events only when the
/// spare fails too; here the spare serves.
#[tokio::test(start_paused = true)]
async fn a_failed_failover_dial_falls_back_to_the_spare() {
    for max_retries in [Some(1), None] {
        let primary_dials = Arc::new(AtomicUsize::new(0));
        let failover_dials = Arc::new(AtomicUsize::new(0));
        let (spare, hub) = LoopbackWebSocket::pair();
        let spare = Arc::new(spare);
        let hubs = Hubs {
            primary: Some(down(&primary_dials)),
            failover: Some(down(&failover_dials)),
            spare: Some(spare.clone()),
        };
        let (seen, ()) = timeout(Duration::from_secs(3_600), async {
            tokio::join!(
                episode(
                    ScActiveHub::Primary,
                    retired(),
                    false,
                    hubs,
                    max_retries,
                    fresh_conn()
                ),
                hub_accept(&hub, FAILOVER)
            )
        })
        .await
        .expect("the spare must receive a Connect-Request");
        assert!(!seen.spare_left);
        let events = seen.events;
        assert_eq!(events.len(), 6, "{events:?}");
        assert_eq!(events[0], attempt_event(ScActiveHub::Primary, 1));
        assert!(is_failed(&events[1], ScActiveHub::Primary));
        assert_eq!(events[2], attempt_event(ScActiveHub::Failover, 1));
        assert!(is_failed(&events[3], ScActiveHub::Failover));
        assert_eq!(events[4], ScEvent::FailoverEngaged);
        assert_eq!(events[5], connected(ScActiveHub::Failover, FAILOVER));
        assert_eq!(primary_dials.load(Ordering::SeqCst), 1);
        assert_eq!(failover_dials.load(Ordering::SeqCst), 1);
    }
}

/// A hub that forbids further retries while a round waits its backoff ends
/// recovery before that round dials anything.
#[tokio::test(start_paused = true)]
async fn retry_eligibility_is_checked_after_the_backoff() {
    for max_retries in [Some(10), None] {
        let dials = Arc::new(AtomicUsize::new(0));
        let hubs = Hubs {
            primary: Some(down(&dials)),
            ..Hubs::default()
        };
        let conn = fresh_conn();
        let (seen, ()) = tokio::join!(
            episode(
                ScActiveHub::Primary,
                retired(),
                false,
                hubs,
                max_retries,
                conn.clone()
            ),
            async {
                tokio::time::sleep(ROUND / 2).await;
                conn.lock().await.connect_retry_allowed = false;
            }
        );
        assert_eq!(seen.events, [ScEvent::ReconnectGaveUp { attempts: 0 }]);
        assert_eq!(dials.load(Ordering::SeqCst), 0);
    }
}

/// A lost socket recovery may reuse gets one Connect. When the hub has
/// closed it, that Connect fails and the socket is not tried again, so a
/// raw transport with no connector gives up instead of retrying a dead
/// socket forever.
#[tokio::test(start_paused = true)]
async fn a_failed_reused_socket_is_not_retried() {
    for max_retries in [Some(5), None] {
        let (client, hub) = LoopbackWebSocket::pair();
        drop(hub);
        let seen = timeout(
            Duration::from_secs(3_600),
            episode(
                ScActiveHub::Primary,
                Arc::new(client),
                true,
                Hubs::default(),
                max_retries,
                fresh_conn(),
            ),
        )
        .await
        .expect("a dead reused socket must end recovery");
        assert!(!seen.reused);
        assert_eq!(seen.events.len(), 3, "{:?}", seen.events);
        assert_eq!(seen.events[0], attempt_event(ScActiveHub::Primary, 1));
        assert!(is_failed(&seen.events[1], ScActiveHub::Primary));
        assert_eq!(seen.events[2], ScEvent::ReconnectGaveUp { attempts: 1 });
    }
}

/// With no primary connector the lost primary is kept for one Connect. When
/// that fails it is closed and the failover hub serves; the closed socket is
/// not offered to primary restore, so no restore probe runs and no
/// `PrimaryRestoreFailed` repeats. (A kept primary that is still good is
/// restored: `primary_restore_duplicate_vmac_reseed_is_reused_by_next_probe`.)
#[tokio::test(start_paused = true)]
async fn a_closed_kept_primary_is_not_probed_for_restore() {
    use ScActiveHub::{Failover, Primary};
    let (client, hub) = LoopbackWebSocket::pair();
    let (failover_tx, mut failover_hubs) = mpsc::unbounded_channel();
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(100)
        .with_reconnect(policy(Some(3)))
        .with_failover_connector(connector(Arc::new(AtomicBool::new(true)), failover_tx));
    let mut events = transport.connection_events();
    let (rx, ()) = tokio::join!(transport.start(), hub_accept(&hub, PRIMARY));
    let _rx = rx.unwrap();
    assert_eq!(next(&mut events).await, connected(Primary, PRIMARY));
    // The hub ends the link and closes; the kept socket's Connect fails.
    hub_closes(hub, &mut events).await;
    attempt(&mut events, Primary, 1).await;
    failed(&mut events, Primary).await;
    attempt(&mut events, Failover, 1).await;
    assert_eq!(next(&mut events).await, ScEvent::FailoverEngaged);
    let failover_hub = failover_hubs.recv().await.unwrap();
    hub_accept(&failover_hub, FAILOVER).await;
    assert_eq!(next(&mut events).await, connected(Failover, FAILOVER));
    // Ten restore intervals: nothing to probe, so nothing happens.
    tokio::time::sleep(10 * ROUND).await;
    assert_eq!(
        events.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    );
    assert_eq!(
        transport.connection_status().active_hub,
        Some(ScActiveHub::Failover)
    );
    transport.stop().await.unwrap();
}

/// A reused socket whose hub answers reconnects on that same socket.
#[tokio::test(start_paused = true)]
async fn a_live_reused_socket_reconnects() {
    let (client, hub) = LoopbackWebSocket::pair();
    let (seen, ()) = tokio::join!(
        episode(
            ScActiveHub::Primary,
            Arc::new(client),
            true,
            Hubs::default(),
            None,
            fresh_conn(),
        ),
        hub_accept(&hub, PRIMARY)
    );
    assert!(seen.reused);
    assert_eq!(
        seen.events,
        [
            attempt_event(ScActiveHub::Primary, 1),
            connected(ScActiveHub::Primary, PRIMARY)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_retry_forbidden_reseed_failure_ends_unlimited_recovery() {
    let _guard = crate::sc::set_test_random48_vmac_generator(|| {
        Err(Error::Transport(std::io::Error::other(
            "fixture entropy unavailable",
        )))
    });
    let (tx, mut hubs) = mpsc::unbounded_channel();
    let (transport, _rx, hub, mut events) =
        started(policy(None), connector(Arc::new(AtomicBool::new(true)), tx)).await;
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
