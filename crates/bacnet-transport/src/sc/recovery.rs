//! Reconnect policy and bounded ownership of retired node sockets.

use super::connector::dial_reconnect_ws;
use super::observability::ScObserver;
use super::*;
use crate::sc_frame::Vmac;
use bacnet_types::sync::MutexExt;

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

/// Whether recovery tries its next Connect-Request on the lost `hub`'s own
/// socket rather than dialling a fresh one: only with reconnect on and no
/// connector for that hub (`connectors` holds whether the primary and the
/// failover hub have one). Any other socket the transport stops using is
/// given up and gets its close frame at once (#2520).
pub(super) fn retries_lost_socket(
    reconnect: bool,
    hub: ScActiveHub,
    connectors: [bool; 2],
) -> bool {
    let [primary, failover] = connectors;
    reconnect
        && !match hub {
            ScActiveHub::Primary => primary,
            ScActiveHub::Failover => failover,
        }
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
    let task = restore_disconnect_task.locked().take();
    if let Some(task) = task {
        task.abort();
        let _ = task.await;
    }
}

async fn publish_connected_ws<W: WebSocketPort>(
    conn: &Arc<Mutex<ScConnection>>,
    active_ws: &super::HubSlot<W>,
    ws: &Arc<W>,
    probe_conn: &Arc<Mutex<ScConnection>>,
    state_tx: &watch::Sender<ScConnectionState>,
    hub_limits: &HubLimits,
) -> Option<Vmac> {
    let restored = probe_conn.lock().await.clone();
    let mut current = active_ws.lock().await;
    let mut c = conn.lock().await;
    *c = restored;
    // A stop takes the slot only once this task has been joined.
    *current = Some(ws.clone());
    hub_limits.publish(&c);
    hub_limits.returned();
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

/// Why a recovery episode ended without a connection.
enum GaveUp {
    /// `max_retries` rounds were made.
    Budget,
    /// No hub is left that can be dialled.
    NoHub,
    /// Further retries are forbidden: a hub's answer, a failed VMAC reseed,
    /// or anything else that cleared the connection's retry eligibility.
    Refused,
}

/// The round counter of one recovery episode.
struct Rounds {
    /// `None` for unlimited recovery.
    max: Option<u32>,
    /// Rounds started so far; also the number of the latest one.
    done: u32,
}

impl Rounds {
    fn new(max: Option<u32>) -> Self {
        Self { max, done: 0 }
    }

    /// Start the next round and return its number, from 1, or `None` once a
    /// bounded budget is used up. Unlimited recovery outlives any count: its
    /// round number stays at `u32::MAX` rather than wrapping or stopping.
    fn next(&mut self) -> Option<u32> {
        if self.max.is_some_and(|max| self.done >= max) {
            return None;
        }
        self.done = self.done.saturating_add(1);
        Some(self.done)
    }
}

pub(super) struct Recovery<'a, W: WebSocketPort> {
    pub config: &'a ScReconnectConfig,
    /// Transport-task owned: retries and reconnect episodes share one log window.
    pub diagnostic_throttle: &'a mut diagnostic_throttle::DiagnosticThrottle,
    pub primary_connector: &'a Option<WebSocketConnector<W>>,
    pub failover_connector: &'a Option<WebSocketConnector<W>>,
    pub failover_ws: &'a mut Option<Arc<W>>,
    /// The kept primary socket a restore probe reuses when there is no
    /// primary connector. Cleared when recovery closes it.
    pub primary_ws: &'a mut Option<Arc<W>>,
    pub conn: &'a Arc<Mutex<ScConnection>>,
    pub active_ws: &'a super::HubSlot<W>,
    pub state_tx: &'a watch::Sender<ScConnectionState>,
    pub connect_timeout_ms: u64,
    pub hub_limits: &'a HubLimits,
    pub observer: &'a ScObserver,
}

impl<W: WebSocketPort> Recovery<'_, W> {
    /// Recover after losing `active_hub`, in rounds (see [`ScReconnectConfig`]):
    /// each waits its jittered backoff, then tries the primary and, when that
    /// fails, the failover hub. The primary goes first whichever hub was lost,
    /// and the failover hub gets its turn from the first round on (Annex
    /// AB.5.2). Returns the new socket and its hub, or `None` once recovery
    /// gives up.
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
        let mut targets = vec![ScActiveHub::Primary, ScActiveHub::Failover];
        let mut rounds = Rounds::new(self.config.max_retries);
        let ended = 'rounds: loop {
            // A hub other than the lost one is dialled only through its
            // connector or, for the failover hub, its unused preconfigured
            // socket; once neither is left it drops out of the episode. The
            // lost hub stays until its own attempt finds it can't be dialled.
            targets.retain(|hub| *hub == active_hub || self.can_dial(*hub));
            if targets.is_empty() {
                break GaveUp::NoHub;
            }
            let Some(round) = rounds.next() else {
                break GaveUp::Budget;
            };
            let delay = jittered_backoff(backoff, initial_backoff, max_backoff);
            tokio::time::sleep(delay).await;
            if !self.conn.lock().await.connect_retry_allowed {
                if let Some(suppressed) = self.retry_diagnostic() {
                    warn!(
                        attempt = round,
                        suppressed,
                        "SC reconnection skipped without retry eligibility (suppressed {suppressed} reconnect diagnostics)"
                    );
                }
                break GaveUp::Refused;
            }
            for target in targets.clone() {
                let lost = target == active_hub;
                let try_hub = Attempt {
                    target,
                    reuse: (lost && current_reusable).then_some(current_ws),
                    lost,
                    attempt: round,
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
                    Step::Stop => break 'rounds GaveUp::Refused,
                }
            }
            backoff = (backoff * 2).min(max_backoff);
        };
        let round = rounds.done;
        let suppressed = self.diagnostic_throttle.take_suppressed();
        match ended {
            GaveUp::Budget => warn!(
                max_retries = round,
                suppressed,
                "SC reconnection: max retries exhausted, giving up (suppressed {suppressed} reconnect diagnostics)"
            ),
            GaveUp::NoHub => warn!(
                attempt = round,
                suppressed,
                "SC reconnection: no hub can be retried, giving up (suppressed {suppressed} reconnect diagnostics)"
            ),
            GaveUp::Refused => warn!(
                attempt = round,
                suppressed,
                "SC reconnection: a hub refused further retries, giving up (suppressed {suppressed} reconnect diagnostics)"
            ),
        }
        {
            let mut c = self.conn.lock().await;
            c.state = ScConnectionState::Disconnected;
            self.state_tx.send_replace(c.state);
        }
        self.observer.gave_up(attempts);
        None
    }

    /// Whether `hub` has a fresh socket to offer: its connector, or for the
    /// failover hub an unused preconfigured socket.
    fn can_dial(&self, hub: ScActiveHub) -> bool {
        match hub {
            ScActiveHub::Primary => self.primary_connector.is_some(),
            ScActiveHub::Failover => {
                self.failover_connector.is_some() || self.failover_ws.is_some()
            }
        }
    }

    /// One dial and handshake to one hub within a reconnect round.
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
        // The preconfigured failover socket is the fallback when the failover
        // connector is missing or its dial fails. It is used once.
        let spare_available = target == ScActiveHub::Failover && self.failover_ws.is_some();
        if connector.is_none() && reuse.is_none() && !spare_available {
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
        let dialled_ws = match dial_reconnect_ws(
            target,
            self.primary_connector,
            self.failover_connector,
            self.connect_timeout_ms,
        )
        .await
        {
            Ok(ws) => ws,
            Err(e) => {
                // Reported even when the spare is tried next: each failed
                // socket gets its own `ConnectFailed` (see `ScEvent`).
                self.observer.connect_failed(target, &e);
                if let Some(suppressed) = self.retry_diagnostic() {
                    warn!(%e, attempt, suppressed, "SC reconnection redial failed (suppressed {suppressed} reconnect diagnostics)");
                }
                if !spare_available {
                    return Step::Failed;
                }
                None
            }
        };
        let fresh_ws = dialled_ws.or_else(|| {
            if spare_available {
                self.failover_ws.take()
            } else {
                None
            }
        });
        // A reused socket gets one Connect: a failed one is closed like a
        // fresh one (Annex AB.6.2.2), so the lost hub then drops out.
        let (ws, fresh) = match fresh_ws {
            Some(ws) => (ws, true),
            None => (reuse.expect("checked above").clone(), false),
        };
        if target == ScActiveHub::Failover && !lost {
            self.observer.failover_engaged();
        }
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
                handshake::close_failed_connect(&*ws, &e).await;
                // A closed kept primary is no longer one a restore probe
                // can reuse.
                if self
                    .primary_ws
                    .as_ref()
                    .is_some_and(|kept| Arc::ptr_eq(kept, &ws))
                {
                    *self.primary_ws = None;
                    // Said once and outside the reconnect throttle: with no
                    // primary connector this is the last way back.
                    warn!(%e, "the kept primary SC socket was closed after a failed connect and is not reused; without a primary connector this node cannot return to the primary hub until it is restarted");
                }
                absorb_failed_connect_probe(self.conn, &probe_conn).await;
                if !self.conn.lock().await.connect_retry_allowed {
                    if let Some(suppressed) = self.retry_diagnostic() {
                        warn!(%e, attempt, suppressed, "SC reconnection failed without retry eligibility (suppressed {suppressed} reconnect diagnostics)");
                    }
                    return Step::Stop;
                }
                if !fresh {
                    if let Some(suppressed) = self.retry_diagnostic() {
                        warn!(%e, attempt, suppressed, "SC reconnection failed on the retired socket, which is not retried (suppressed {suppressed} reconnect diagnostics)");
                    }
                    return Step::Dead;
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
    fn rounds_stop_at_the_budget_and_unlimited_ones_saturate() {
        let mut none = Rounds::new(Some(0));
        assert_eq!(none.next(), None);
        assert_eq!(none.done, 0);

        let mut two = Rounds::new(Some(2));
        assert_eq!(
            [two.next(), two.next(), two.next()],
            [Some(1), Some(2), None]
        );

        let mut full = Rounds::new(Some(u32::MAX));
        full.done = u32::MAX - 1;
        assert_eq!([full.next(), full.next()], [Some(u32::MAX), None]);

        // Unlimited recovery past u32::MAX rounds keeps going on the last number.
        let mut unlimited = Rounds::new(None);
        assert_eq!(unlimited.next(), Some(1));
        unlimited.done = u32::MAX - 1;
        for _ in 0..3 {
            assert_eq!(unlimited.next(), Some(u32::MAX));
        }
    }

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
            (1, 600_000),
            (599_999, 600_000),
            (600_000, 600_000),
        ] {
            let config = ScReconnectConfig {
                initial_delay_ms,
                max_delay_ms,
                max_retries: Some(10),
            };
            config.validate().unwrap();
            let initial = Duration::from_millis(initial_delay_ms);
            let maximum = Duration::from_millis(max_delay_ms);
            let mut backoff = initial;
            for _ in 0..10 {
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
