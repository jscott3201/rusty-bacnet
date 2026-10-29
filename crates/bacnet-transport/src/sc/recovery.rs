//! Reconnect policy and bounded ownership of retired node sockets.

use super::connector::dial_reconnect_ws;
use super::observability::ScObserver;
use super::*;
use crate::sc_frame::Vmac;

async fn connect_probe_from(conn: &Arc<Mutex<ScConnection>>) -> Arc<Mutex<ScConnection>> {
    Arc::new(Mutex::new(conn.lock().await.connect_probe()))
}

async fn absorb_failed_connect_probe(
    conn: &Arc<Mutex<ScConnection>>,
    probe_conn: &Arc<Mutex<ScConnection>>,
) {
    let probe = probe_conn.lock().await;
    let mut c = conn.lock().await;
    c.absorb_failed_probe(&probe);
}

/// Jitter a nominal backoff from validated reconnect settings without changing
/// its doubling progression. Both the configured initial floor and cap apply.
fn jittered_backoff(backoff: Duration, initial: Duration, maximum: Duration) -> Duration {
    let lower = initial.max(backoff / 2);
    let upper = maximum.min(backoff + backoff / 2);
    let mut random = [0; 16];
    if getrandom::fill(&mut random).is_err() {
        // Preserve recovery and its bounds if OS entropy is unavailable.
        return backoff;
    }
    let nanos = u128::from_le_bytes(random) % ((upper - lower).as_nanos() + 1);
    lower
        + Duration::new(
            (nanos / 1_000_000_000) as u64,
            (nanos % 1_000_000_000) as u32,
        )
}

pub(super) async fn retire<W: WebSocketPort>(
    current_ws: &Arc<W>,
    primary_ws: &mut Option<Arc<W>>,
    conn: &Arc<Mutex<ScConnection>>,
    state_tx: &watch::Sender<ScConnectionState>,
    restore_disconnect_task: &Arc<StdMutex<Option<JoinHandle<()>>>>,
) {
    // Seal new public send/stop admission before any recovery await. The NAK
    // future has already been dropped. Previously admitted application sends
    // may finish; neither their writes nor buffered NAK bytes are rolled back.
    {
        let mut c = conn.lock().await;
        c.state = ScConnectionState::Disconnected;
        state_tx.send_replace(c.state);
    }
    if primary_ws
        .as_ref()
        .is_some_and(|ws| Arc::ptr_eq(ws, current_ws))
    {
        *primary_ws = None;
    }
    // A restore task targets the previous failover, not the current socket
    // (connector freshness is a caller contract). Terminate any outstanding
    // task as well, so it cannot initiate deferred cleanup after retirement.
    let task = restore_disconnect_task
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(task) = task {
        task.abort();
        let _ = task.await;
    }
}

async fn publish_connected_ws<W: WebSocketPort>(
    conn: &Arc<Mutex<ScConnection>>,
    active_ws: &Arc<Mutex<Arc<W>>>,
    ws: &Arc<W>,
    probe_conn: &Arc<Mutex<ScConnection>>,
    state_tx: &watch::Sender<ScConnectionState>,
    hub_limits: &HubLimits,
) -> Option<Vmac> {
    let restored = probe_conn.lock().await.clone();
    let mut current = active_ws.lock().await;
    let mut c = conn.lock().await;
    *c = restored;
    *current = ws.clone();
    hub_limits.publish(&c);
    state_tx.send_replace(c.state);
    c.hub_vmac
}

enum Step<W> {
    Up(Arc<W>),
    Failed,
    /// This target can never be dialled in this episode.
    Dead,
    Stop,
}

struct Attempt<'a, W> {
    target: ScActiveHub,
    /// The lost socket, when it is the target and may be reused.
    reuse: Option<&'a Arc<W>>,
    /// Whether `target` is the hub whose connection was lost.
    lost: bool,
    attempt: u32,
    delay: Duration,
    backoff: Duration,
    /// Set to `attempt` once a dial actually starts.
    dialled: &'a mut u32,
}

pub(super) struct Recovery<'a, W: WebSocketPort> {
    pub config: &'a ScReconnectConfig,
    /// Transport-task owned: retries and reconnect episodes share one log window.
    pub diagnostic_throttle: &'a mut diagnostic_throttle::DiagnosticThrottle,
    pub primary_connector: &'a Option<WebSocketConnector<W>>,
    pub failover_connector: &'a Option<WebSocketConnector<W>>,
    pub failover_ws: &'a mut Option<Arc<W>>,
    pub conn: &'a Arc<Mutex<ScConnection>>,
    pub active_ws: &'a Arc<Mutex<Arc<W>>>,
    pub state_tx: &'a watch::Sender<ScConnectionState>,
    pub connect_timeout_ms: u64,
    pub hub_limits: &'a HubLimits,
    pub observer: &'a ScObserver,
}

impl<W: WebSocketPort> Recovery<'_, W> {
    pub(super) async fn reconnect(
        &mut self,
        current_ws: &Arc<W>,
        active_hub: ScActiveHub,
        current_reusable: bool,
    ) -> Option<(Arc<W>, ScActiveHub)> {
        warn!("SC transport disconnected, attempting reconnection");
        let initial_backoff = Duration::from_millis(self.config.initial_delay_ms);
        let mut backoff = initial_backoff;
        let max_backoff = Duration::from_millis(self.config.max_delay_ms);
        let mut attempts = 0;
        // Losing the failover hub sends each attempt to the primary first,
        // then back to the failover hub, so the connector keeps working toward
        // the primary (Annex AB.5.2). Losing the primary retries the primary.
        let mut targets = match active_hub {
            ScActiveHub::Primary => vec![ScActiveHub::Primary],
            ScActiveHub::Failover => vec![ScActiveHub::Primary, ScActiveHub::Failover],
        };
        'attempts: for attempt in 1..=self.config.max_retries {
            let delay = jittered_backoff(backoff, initial_backoff, max_backoff);
            tokio::time::sleep(delay).await;
            if !self.conn.lock().await.connect_retry_allowed {
                if let Some(suppressed) = self.retry_diagnostic() {
                    warn!(
                        attempt,
                        suppressed,
                        "SC reconnection skipped without retry eligibility (suppressed {suppressed} reconnect diagnostics)"
                    );
                }
                break;
            }
            for target in targets.clone() {
                let lost = target == active_hub;
                let try_hub = Attempt {
                    target,
                    reuse: (lost && current_reusable).then_some(current_ws),
                    lost,
                    attempt,
                    delay,
                    backoff,
                    dialled: &mut attempts,
                };
                match self.try_hub(try_hub).await {
                    Step::Up(ws) => return Some((ws, target)),
                    Step::Failed => {}
                    // The lost hub can't be dialled again; any other target
                    // (the primary, after losing the failover) still can.
                    Step::Dead => targets.retain(|t| *t != target),
                    Step::Stop => break 'attempts,
                }
            }
            if targets.is_empty() {
                break;
            }
            backoff = (backoff * 2).min(max_backoff);
        }
        // Preserve the existing order: only primary exhaustion tries failover.
        // An untouched preconfigured failover is consumed once, never poisoned
        // merely because a different (primary) socket was retired.
        if active_hub == ScActiveHub::Primary
            && self.conn.lock().await.connect_retry_allowed
            && let Some(failover) = dial_failover_ws(
                self.failover_connector,
                self.failover_ws,
                self.connect_timeout_ms,
            )
            .await
        {
            warn!("SC primary reconnection exhausted, attempting failover hub");
            self.observer.failover_engaged();
            {
                let mut c = self.conn.lock().await;
                c.reset_for_connect_retry();
                self.state_tx.send_replace(c.state);
            }
            let probe_conn = connect_probe_from(self.conn).await;
            match perform_handshake(&*failover, &probe_conn, None, self.connect_timeout_ms).await {
                Ok(()) => {
                    self.publish(&failover, &probe_conn, ScActiveHub::Failover)
                        .await;
                    let suppressed = self.diagnostic_throttle.take_suppressed();
                    info!(
                        suppressed,
                        "SC connected to failover hub after primary reconnect exhaustion (suppressed {suppressed} reconnect diagnostics)"
                    );
                    return Some((failover, ScActiveHub::Failover));
                }
                Err(e) => {
                    self.observer.connect_failed(ScActiveHub::Failover, &e);
                    absorb_failed_connect_probe(self.conn, &probe_conn).await;
                    // The terminal exhaustion notice below reports suppression.
                    warn!(%e, "SC failover connection failed");
                }
            }
        }
        let suppressed = self.diagnostic_throttle.take_suppressed();
        warn!(
            max_retries = self.config.max_retries,
            suppressed,
            "SC reconnection: max retries exhausted, giving up (suppressed {suppressed} reconnect diagnostics)"
        );
        {
            let mut c = self.conn.lock().await;
            c.state = ScConnectionState::Disconnected;
            self.state_tx.send_replace(c.state);
        }
        self.observer.gave_up(attempts);
        None
    }

    /// One dial and handshake to one hub within a reconnect attempt.
    async fn try_hub(&mut self, at: Attempt<'_, W>) -> Step<W> {
        let Attempt {
            target,
            reuse,
            lost,
            attempt,
            delay,
            backoff,
            dialled,
        } = at;
        let connector = match target {
            ScActiveHub::Primary => self.primary_connector,
            ScActiveHub::Failover => self.failover_connector,
        };
        if connector.is_none() && reuse.is_none() {
            // Without a connector only the lost hub's own socket can be
            // reused. Another hub without one is simply skipped.
            if lost {
                if let Some(suppressed) = self.retry_diagnostic() {
                    warn!(
                        suppressed,
                        "SC retired socket cannot be reused without a fresh connector (suppressed {suppressed} reconnect diagnostics)"
                    );
                }
                return Step::Dead;
            }
            return Step::Failed;
        }
        *dialled = attempt;
        self.observer.reconnect_attempt(target, attempt, delay);
        {
            let mut c = self.conn.lock().await;
            c.reset_for_connect_retry();
            self.state_tx.send_replace(c.state);
        }
        let ws = match dial_reconnect_ws(
            target,
            self.primary_connector,
            self.failover_connector,
            self.connect_timeout_ms,
        )
        .await
        {
            Ok(Some(ws)) => ws,
            Ok(None) => reuse.expect("checked above").clone(),
            Err(e) => {
                self.observer.connect_failed(target, &e);
                if let Some(suppressed) = self.retry_diagnostic() {
                    warn!(%e, attempt, suppressed, "SC reconnection redial failed (suppressed {suppressed} reconnect diagnostics)");
                }
                return Step::Failed;
            }
        };
        let probe_conn = connect_probe_from(self.conn).await;
        match perform_handshake(&*ws, &probe_conn, None, self.connect_timeout_ms).await {
            Ok(()) => {
                self.publish(&ws, &probe_conn, target).await;
                let suppressed = self.diagnostic_throttle.take_suppressed();
                info!(
                    attempt,
                    suppressed,
                    "SC reconnected after backoff (suppressed {suppressed} reconnect diagnostics)"
                );
                Step::Up(ws)
            }
            Err(e) => {
                self.observer.connect_failed(target, &e);
                absorb_failed_connect_probe(self.conn, &probe_conn).await;
                if !self.conn.lock().await.connect_retry_allowed {
                    if let Some(suppressed) = self.retry_diagnostic() {
                        warn!(%e, attempt, suppressed, "SC reconnection failed without retry eligibility (suppressed {suppressed} reconnect diagnostics)");
                    }
                    return Step::Stop;
                }
                if let Some(suppressed) = self.retry_diagnostic() {
                    warn!(%e, attempt, suppressed, "SC reconnection failed, retrying in {:?} (suppressed {suppressed} reconnect diagnostics)", backoff);
                }
                Step::Failed
            }
        }
    }

    /// Gate only per-attempt diagnostics, never recovery or wire decisions.
    /// Lifecycle transitions/outcomes bypass the gate; taking their summary
    /// does not reset the window, so a flap cannot replenish its allowance.
    fn retry_diagnostic(&mut self) -> Option<u64> {
        // Monotonic in production and follows the paused runtime clock in tests.
        self.diagnostic_throttle
            .should_emit(tokio::time::Instant::now().into_std())
            .then(|| self.diagnostic_throttle.take_suppressed())
    }

    async fn publish(&self, ws: &Arc<W>, probe: &Arc<Mutex<ScConnection>>, hub: ScActiveHub) {
        let hub_vmac = publish_connected_ws(
            self.conn,
            self.active_ws,
            ws,
            probe,
            self.state_tx,
            self.hub_limits,
        )
        .await;
        self.observer.connected(hub, hub_vmac);
    }
}

#[cfg(test)]
#[path = "reconnect_diagnostic_tests.rs"]
mod diagnostic_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_jitter_default_samples_stay_in_each_capped_window() {
        let config = ScReconnectConfig::default();
        let initial = Duration::from_millis(config.initial_delay_ms);
        let maximum = Duration::from_millis(config.max_delay_ms);
        // Independent bounds for each nominal step, including the capped step.
        for (backoff, lower, upper) in [
            (10, 10, 15),
            (20, 10, 30),
            (40, 20, 60),
            (80, 40, 120),
            (160, 80, 240),
            (320, 160, 480),
            (600, 300, 600),
        ] {
            for _ in 0..256 {
                let sleep = jittered_backoff(Duration::from_secs(backoff), initial, maximum);
                assert!(sleep >= Duration::from_secs(lower));
                assert!(sleep <= Duration::from_secs(upper));
            }
        }
    }

    #[test]
    fn reconnect_jitter_samples_respect_small_equal_and_delay_cap_config_bounds() {
        for (initial_delay_ms, max_delay_ms) in [
            (1, 1),
            (1, 2),
            (3, 7),
            (1, 86_400_000),
            (86_399_999, 86_400_000),
            (86_400_000, 86_400_000),
        ] {
            let config = ScReconnectConfig {
                initial_delay_ms,
                max_delay_ms,
                max_retries: 10,
            };
            config.validate().unwrap();
            let initial = Duration::from_millis(initial_delay_ms);
            let maximum = Duration::from_millis(max_delay_ms);
            let mut backoff = initial;
            for _ in 0..config.max_retries {
                for _ in 0..256 {
                    // Duration calculations only: never sleep on large values.
                    let sleep = jittered_backoff(backoff, initial, maximum);
                    assert!(sleep >= initial);
                    assert!(sleep <= maximum);
                }
                backoff = (backoff * 2).min(maximum);
            }
        }
    }
}
