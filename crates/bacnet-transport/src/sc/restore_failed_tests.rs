//! Failed primary-restore probes as events (#1828), on tokio's paused clock.

use tokio::time::Instant;

use super::*;

/// On the failover hub with the primary refusing, each restore tick probes
/// the primary once and reports that probe's failure once, with the cause
/// sorted as `ConnectFailed` sorts it. A probe that succeeds still reads
/// `PrimaryRestored`, then `Connected`.
#[tokio::test(start_paused = true)]
async fn each_failed_restore_probe_is_one_event_per_interval() {
    const INTERVAL: Duration = Duration::from_millis(25);
    let (client, primary_hub) = LoopbackWebSocket::pair();
    let primary_up = Arc::new(AtomicBool::new(false));
    let (primary_tx, mut primary_dials) = mpsc::unbounded_channel();
    let (failover_tx, mut failover_dials) = mpsc::unbounded_channel();
    let mut transport = ScTransport::new(client, [0x01; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(100)
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: INTERVAL.as_millis() as u64,
            max_delay_ms: INTERVAL.as_millis() as u64,
            max_retries: 1,
        })
        .with_connector(connector(primary_up.clone(), primary_tx))
        .with_failover_connector(connector(Arc::new(AtomicBool::new(true)), failover_tx));
    let mut events = transport.connection_events();
    let (rx, ()) = tokio::join!(transport.start(), hub_accept(&primary_hub, PRIMARY));
    let _rx = rx.unwrap();
    assert_eq!(
        next(&mut events).await,
        connected(ScActiveHub::Primary, PRIMARY)
    );
    hub_closes(primary_hub, &mut events).await;
    assert!(matches!(
        next(&mut events).await,
        ScEvent::ReconnectAttempt { .. }
    ));
    let ScEvent::ConnectFailed { cause: refused, .. } = next(&mut events).await else {
        panic!("the primary refuses the reconnect");
    };
    assert_eq!(next(&mut events).await, ScEvent::FailoverEngaged);
    let failover_hub = failover_dials.recv().await.unwrap();
    hub_accept(&failover_hub, FAILOVER).await;
    assert_eq!(
        next(&mut events).await,
        connected(ScActiveHub::Failover, FAILOVER)
    );

    let mut last = Instant::now();
    for _ in 0..3 {
        // Nothing before the tick: just short of it, the stream is empty.
        tokio::time::sleep_until(last + INTERVAL - Duration::from_millis(1)).await;
        assert_eq!(events.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(
            next(&mut events).await,
            ScEvent::PrimaryRestoreFailed {
                cause: refused.clone(),
            }
        );
        // The refusal is immediate, so the event comes on the tick itself.
        assert_eq!(Instant::now(), last + INTERVAL);
        last = Instant::now();
        assert_eq!(events.try_recv(), Err(TryRecvError::Empty));
    }
    // The failover connection stayed in use through every failed probe.
    let status = transport.connection_status();
    assert_eq!(status.state, ScConnectionState::Connected);
    assert_eq!(status.active_hub, Some(ScActiveHub::Failover));

    primary_up.store(true, Ordering::SeqCst);
    let primary_hub = primary_dials.recv().await.unwrap();
    hub_accept(&primary_hub, PRIMARY).await;
    assert_eq!(next(&mut events).await, ScEvent::PrimaryRestored);
    assert_eq!(
        next(&mut events).await,
        connected(ScActiveHub::Primary, PRIMARY)
    );
    transport.stop().await.unwrap();
    drop((primary_hub, failover_hub));
}
