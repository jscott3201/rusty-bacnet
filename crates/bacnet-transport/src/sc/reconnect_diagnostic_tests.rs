use super::*;
use crate::sc_frame::connect_test_support::valid_connect;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicUsize;
use tracing::{span, Event, Metadata, Subscriber};

#[derive(Clone, Debug)]
struct Record {
    at: tokio::time::Instant,
    level: tracing::Level,
    fields: BTreeMap<String, String>,
}

#[derive(Clone)]
struct Capture(Arc<StdMutex<Vec<Record>>>);

impl Default for Capture {
    fn default() -> Self {
        // Match the dcc_trace_tests scoped-subscriber precedent: retain a
        // disabled registration peer so parallel cold callsites cannot cache
        // "never" from a thread without our subscriber. No global default.
        static PEER: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
        PEER.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
        Self(Arc::default())
    }
}

impl Subscriber for Capture {
    fn register_callsite(&self, _: &'static Metadata<'static>) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }
    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::INFO)
    }
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "bacnet_transport::sc::recovery"
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        if !self.enabled(event.metadata()) {
            return;
        }
        struct Visitor(BTreeMap<String, String>);
        impl tracing::field::Visit for Visitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.insert(field.name().into(), format!("{value:?}"));
            }
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                self.0.insert(field.name().into(), value.into());
            }
        }
        let mut visitor = Visitor(BTreeMap::new());
        event.record(&mut visitor);
        self.0.lock().unwrap().push(Record {
            at: tokio::time::Instant::now(),
            level: *event.metadata().level(),
            fields: visitor.0,
        });
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

impl Capture {
    fn matching(&self, message: &str) -> Vec<Record> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|record| record.fields["message"].starts_with(message))
            .cloned()
            .collect()
    }
}

fn assert_summary(record: &Record, suppressed: u64) {
    assert_eq!(record.fields["suppressed"], suppressed.to_string());
    assert!(record.fields["message"]
        .ends_with(&format!("(suppressed {suppressed} reconnect diagnostics)")));
}

fn config(max_retries: u32) -> ScReconnectConfig {
    ScReconnectConfig {
        initial_delay_ms: 1,
        max_delay_ms: 1,
        max_retries,
    }
}

fn transport(ws: LoopbackWebSocket, max_retries: u32) -> ScTransport<LoopbackWebSocket> {
    ScTransport::new(ws, [0x22; 6])
        .with_device_uuid([1; 16])
        .with_reconnect(config(max_retries))
}

async fn accept(hub: &LoopbackWebSocket) {
    let request = tokio::time::timeout(Duration::from_secs(1), hub.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request[0], 6);
    let mut response = valid_connect(7, [0x10; 6]);
    response[2..4].copy_from_slice(&request[2..4]);
    hub.send(&response).await.unwrap();
}

async fn wait_connected(transport: &ScTransport<LoopbackWebSocket>) {
    let mut state = transport.connection_state_changes();
    tokio::time::timeout(Duration::from_secs(1), async {
        while *state.borrow_and_update() != ScConnectionState::Connected {
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}

async fn wait_exhausted(transport: &mut ScTransport<LoopbackWebSocket>) {
    tokio::time::timeout(
        Duration::from_secs(10),
        transport.recv_task.as_mut().unwrap(),
    )
    .await
    .expect("recovery must exhaust its budget")
    .unwrap();
    let _ = transport.recv_task.take();
    assert_eq!(
        transport.connection().unwrap().lock().await.state,
        ScConnectionState::Disconnected
    );
}

// These tests use Tokio's current-thread runtime: the scoped subscriber also
// sees the transport receive task, without installing a global subscriber.
#[tokio::test(start_paused = true)]
async fn outage_diagnostics_scale_with_windows_not_dial_attempts() {
    for max_retries in [0, 1, 100, 2_500] {
        let capture = Capture::default();
        let _subscriber = tracing::subscriber::set_default(capture.clone());
        let (client, hub) = LoopbackWebSocket::pair();
        let dials = Arc::new(AtomicUsize::new(0));
        let mut transport = transport(client, max_retries).with_connector({
            let dials = dials.clone();
            move || {
                dials.fetch_add(1, Ordering::SeqCst);
                async { Err(Error::Encoding("hub outage".into())) }
            }
        });
        let (started, ()) = tokio::join!(transport.start(), accept(&hub));
        let _rx = started.unwrap();
        let start = tokio::time::Instant::now();
        drop(hub);
        wait_exhausted(&mut transport).await;
        assert_eq!(dials.load(Ordering::SeqCst), max_retries as usize);

        let disconnect = capture.matching("SC transport disconnected");
        let attempts = capture.matching("SC reconnection redial failed");
        let terminal = capture.matching("SC reconnection: max retries exhausted");
        assert_eq!(disconnect.len(), 1);
        assert_eq!(disconnect[0].level, tracing::Level::WARN);
        assert_eq!(terminal.len(), 1);
        assert_eq!(terminal[0].fields["max_retries"], max_retries.to_string());
        assert_eq!(terminal[0].level, tracing::Level::WARN);
        if max_retries <= 100 {
            assert_eq!(attempts.len(), usize::from(max_retries > 0));
            assert_summary(&terminal[0], max_retries.saturating_sub(1) as u64);
        } else {
            assert!(attempts.len() > 1, "must exercise window rollover");
            assert!(attempts.len() <= 1 + start.elapsed().as_secs() as usize);
        }
        for pair in attempts.windows(2) {
            assert!(pair[1].at - pair[0].at >= diagnostic_throttle::DIAGNOSTIC_WINDOW);
        }
        if let Some(first) = attempts.first() {
            assert_eq!(first.fields["attempt"], "1");
            assert_summary(first, 0);
        }
        let mut total_suppressed = 0;
        for record in attempts.iter().chain(&terminal) {
            assert_eq!(record.level, tracing::Level::WARN);
            let suppressed = record.fields["suppressed"].parse::<u64>().unwrap();
            assert_summary(record, suppressed);
            total_suppressed += suppressed;
        }
        assert_eq!(attempts.len() as u64 + total_suppressed, max_retries as u64);
        assert_eq!(capture.0.lock().unwrap().len(), attempts.len() + 2);
        transport.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn reconnect_flaps_preserve_the_transport_task_diagnostic_window() {
    let capture = Capture::default();
    let _subscriber = tracing::subscriber::set_default(capture.clone());
    let (client, hub) = LoopbackWebSocket::pair();
    let (hub_tx, mut hubs) = mpsc::unbounded_channel();
    let dials = Arc::new(AtomicUsize::new(0));
    let mut transport = transport(client, 3).with_connector({
        let dials = dials.clone();
        move || {
            let dial = dials.fetch_add(1, Ordering::SeqCst) + 1;
            let result = if matches!(dial, 3 | 6) {
                let (client, hub) = LoopbackWebSocket::pair();
                hub_tx.send(hub).unwrap();
                Ok(client)
            } else {
                Err(Error::Encoding("flapping hub".into()))
            };
            async { result }
        }
    });
    let (started, ()) = tokio::join!(transport.start(), accept(&hub));
    let _rx = started.unwrap();
    let start = tokio::time::Instant::now();
    drop(hub);
    for expected_dials in [3, 6] {
        let hub = tokio::time::timeout(Duration::from_secs(1), hubs.recv())
            .await
            .unwrap()
            .unwrap();
        accept(&hub).await;
        wait_connected(&transport).await;
        assert_eq!(dials.load(Ordering::SeqCst), expected_dials);
        // Confirm the replacement socket is published to the application path.
        transport
            .send_unicast(&[1, 2, 3], &[0x33; 6])
            .await
            .unwrap();
        let data = hub.recv().await.unwrap();
        assert_eq!(
            decode_sc_message(&data).unwrap().function,
            ScFunction::EncapsulatedNpdu
        );
        drop(hub);
    }
    wait_exhausted(&mut transport).await;
    assert!(start.elapsed() < diagnostic_throttle::DIAGNOSTIC_WINDOW);
    assert_eq!(dials.load(Ordering::SeqCst), 9);
    assert_eq!(capture.matching("SC transport disconnected").len(), 3);
    assert_eq!(capture.matching("SC reconnection redial failed").len(), 1);
    let successes = capture.matching("SC reconnected after backoff");
    assert_eq!(successes.len(), 2);
    assert_summary(&successes[0], 1);
    assert_summary(&successes[1], 2);
    let terminal = capture.matching("SC reconnection: max retries exhausted");
    assert_eq!(terminal.len(), 1);
    assert_summary(&terminal[0], 3);
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn handshake_retry_warnings_share_the_redial_diagnostic_budget() {
    let capture = Capture::default();
    let _subscriber = tracing::subscriber::set_default(capture.clone());
    let (client, hub) = LoopbackWebSocket::pair();
    let dials = Arc::new(AtomicUsize::new(0));
    let mut transport = transport(client, 100).with_connector({
        let dials = dials.clone();
        move || {
            let attempt = dials.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt == 0 {
                    Err(Error::Encoding("dial failed".into()))
                } else {
                    let (client, hub) = LoopbackWebSocket::pair();
                    drop(hub); // Dial succeeds; sending Connect-Request fails.
                    Ok(client)
                }
            }
        }
    });
    let (started, ()) = tokio::join!(transport.start(), accept(&hub));
    let _rx = started.unwrap();
    drop(hub);
    wait_exhausted(&mut transport).await;
    assert_eq!(dials.load(Ordering::SeqCst), 100);
    assert_eq!(capture.matching("SC reconnection redial failed").len(), 1);
    assert!(capture
        .matching("SC reconnection failed, retrying")
        .is_empty());
    let terminal = capture.matching("SC reconnection: max retries exhausted");
    assert_eq!(terminal.len(), 1);
    assert_summary(&terminal[0], 99);
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn failover_transition_and_outcomes_bypass_a_full_diagnostic_window() {
    for succeeds in [false, true] {
        let capture = Capture::default();
        let _subscriber = tracing::subscriber::set_default(capture.clone());
        let (client, hub) = LoopbackWebSocket::pair();
        let (failover, failover_hub) = LoopbackWebSocket::pair();
        let mut transport = transport(client, 100)
            .with_failover(failover)
            .with_connector(|| async { Err(Error::Encoding("primary outage".into())) });
        let (started, ()) = tokio::join!(transport.start(), accept(&hub));
        let _rx = started.unwrap();
        drop(hub);
        if succeeds {
            accept(&failover_hub).await;
            wait_connected(&transport).await;
            let outcome = capture.matching("SC connected to failover hub after primary");
            assert_eq!(outcome.len(), 1);
            assert_eq!(outcome[0].level, tracing::Level::INFO);
            assert_summary(&outcome[0], 99);
            assert!(capture
                .matching("SC reconnection: max retries exhausted")
                .is_empty());
        } else {
            drop(failover_hub);
            wait_exhausted(&mut transport).await;
            let outcome = capture.matching("SC failover connection failed");
            assert_eq!(outcome.len(), 1);
            assert_eq!(outcome[0].level, tracing::Level::WARN);
            let terminal = capture.matching("SC reconnection: max retries exhausted");
            assert_eq!(terminal.len(), 1);
            assert_summary(&terminal[0], 99);
        }
        assert_eq!(
            capture
                .matching("SC primary reconnection exhausted, attempting failover")
                .len(),
            1
        );
        assert_eq!(capture.matching("SC transport disconnected").len(), 1);
        assert_eq!(capture.matching("SC reconnection redial failed").len(), 1);
        transport.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn skipped_retries_are_gated_but_disconnect_and_exhaustion_always_emit() {
    for eligible in [false, true] {
        for already_emitted in [false, true] {
            let capture = Capture::default();
            let _subscriber = tracing::subscriber::set_default(capture.clone());
            let (client, _hub) = LoopbackWebSocket::pair();
            let ws = Arc::new(client);
            let conn = Arc::new(Mutex::new(ScConnection::new([0x22; 6], [1; 16])));
            conn.lock().await.connect_retry_allowed = eligible;
            let active_ws = Arc::new(Mutex::new(ws.clone()));
            let (state_tx, _) = watch::channel(ScConnectionState::Disconnected);
            let mut throttle = diagnostic_throttle::DiagnosticThrottle::new();
            if already_emitted {
                assert!(throttle.should_emit(tokio::time::Instant::now().into_std()));
            }
            let config = config(10);
            let mut failover_ws = None;
            let maximum = AtomicU16::new(DEFAULT_MAX_APDU_LENGTH);
            let mut recovery = Recovery {
                config: &config,
                diagnostic_throttle: &mut throttle,
                primary_connector: &None,
                failover_connector: &None,
                failover_ws: &mut failover_ws,
                conn: &conn,
                active_ws: &active_ws,
                state_tx: &state_tx,
                connect_timeout_ms: 10,
                effective_max_apdu_length: &maximum,
            };
            assert!(recovery
                .reconnect(&ws, ActiveHub::Primary, false)
                .await
                .is_none());
            let skipped = capture.matching(if eligible {
                "SC retired socket cannot be reused"
            } else {
                "SC reconnection skipped without retry eligibility"
            });
            assert_eq!(skipped.len(), usize::from(!already_emitted));
            if let Some(record) = skipped.first() {
                assert_summary(record, 0);
            }
            assert_eq!(capture.matching("SC transport disconnected").len(), 1);
            let terminal = capture.matching("SC reconnection: max retries exhausted");
            assert_eq!(terminal.len(), 1);
            assert_summary(&terminal[0], u64::from(already_emitted));
            assert_eq!(throttle.suppressed(), 0);
            assert!(!throttle.should_emit(tokio::time::Instant::now().into_std()));
        }
    }
}
