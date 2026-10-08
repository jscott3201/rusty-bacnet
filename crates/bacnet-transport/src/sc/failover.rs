//! BACnet/SC primary/failover hub switching helpers.

use std::future::Future;
use std::pin::Pin;
use std::sync::{atomic::AtomicU16, Arc, Mutex as StdMutex};
use std::time::Duration;

use bacnet_types::error::Error;
use bytes::BytesMut;
use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;
use tokio::time::{Instant, Interval, MissedTickBehavior};
use tracing::warn;

use crate::sc_frame::{encode_sc_message, ScMessage};

use super::{
    connector::dial_connector, handshake::perform_handshake, publish_effective_max_apdu_length,
    ScConnection, ScConnectionState, WebSocketConnector, WebSocketPort,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActiveHub {
    Primary,
    Failover,
}

/// The cadence of primary-restore attempts while the failover hub is active,
/// first due one `period` from now (#1555).
///
/// The receive loop resets it after every attempt and after every reconnect,
/// so an attempt is always one `period` after the last attempt ended or the
/// connection came up. Without the resets, a tick the loop didn't take (a
/// dial that ran past the period, or time spent on the primary hub, when
/// restore ticks are off) would be due at once. `Delay` is a guard on top: a
/// tick that is late anyway brings one attempt, never a burst of them.
pub(super) fn restore_interval(period: Duration) -> Interval {
    let mut interval = tokio::time::interval_at(Instant::now() + period, period);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    interval
}

/// A candidate has no authority over the live connection. Its future is
/// owned by one receive-loop connection epoch and cancellation drops its I/O.
pub(super) struct PrimaryCandidate<W> {
    socket: Result<Arc<W>, Error>,
    connection: ScConnection,
}

pub(super) type PrimaryAttempt<W> = Pin<Box<dyn Future<Output = PrimaryCandidate<W>> + Send>>;

pub(super) async fn prepare_primary_restore<W: WebSocketPort>(
    primary_ws: Option<Arc<W>>,
    primary_connector: Option<WebSocketConnector<W>>,
    probe: ScConnection,
    connect_timeout_ms: u64,
) -> PrimaryCandidate<W> {
    let socket = if let Some(connector) = primary_connector {
        dial_connector(&connector, connect_timeout_ms)
            .await
            .map(Arc::new)
    } else {
        primary_ws
            .ok_or_else(|| Error::Encoding("SC retired primary requires a fresh connector".into()))
    };
    let probe = Arc::new(Mutex::new(probe));
    let socket = match socket {
        Ok(ws) => perform_handshake(&*ws, &probe, None, connect_timeout_ms)
            .await
            .map(|()| ws),
        Err(e) => Err(e),
    };
    let connection = probe.lock().await.clone();
    PrimaryCandidate { socket, connection }
}

/// Live publication belongs exclusively to the receive loop.
pub(super) struct PrimaryRestoreContext<'a, W> {
    pub(super) active_ws: &'a Arc<Mutex<Arc<W>>>,
    pub(super) conn: &'a Arc<Mutex<ScConnection>>,
    pub(super) restore_disconnect_task: &'a Arc<StdMutex<Option<JoinHandle<()>>>>,
    pub(super) state_tx: &'a watch::Sender<ScConnectionState>,
    pub(super) connect_timeout_ms: u64,
    pub(super) effective_max_apdu_length: &'a AtomicU16,
}

pub(super) async fn publish_primary_restore<W: WebSocketPort>(
    ctx: &PrimaryRestoreContext<'_, W>,
    current_ws: &Arc<W>,
    candidate: PrimaryCandidate<W>,
) -> Result<Option<Arc<W>>, Error> {
    let mut current = ctx.active_ws.lock().await;
    let mut c = ctx.conn.lock().await;
    // Stop can seal the live state while a candidate is finishing. Never
    // publish its success or failed-probe retry metadata after that boundary.
    if c.state != ScConnectionState::Connected || !Arc::ptr_eq(&current, current_ws) {
        return Ok(None);
    }
    let restored_ws = match candidate.socket {
        Ok(ws) => ws,
        Err(e) => {
            c.absorb_failed_probe(&candidate.connection);
            return Err(e);
        }
    };
    let disconnect_msg = c.clone().build_disconnect_request().ok();
    *c = candidate.connection;
    *current = restored_ws.clone();
    publish_effective_max_apdu_length(ctx.effective_max_apdu_length, &c);
    ctx.state_tx.send_replace(c.state);
    drop(c);
    drop(current);

    let disconnect_ws = current_ws.clone();
    let timeout = ctx.connect_timeout_ms;
    let task = tokio::spawn(async move {
        send_disconnect_request(&*disconnect_ws, disconnect_msg, timeout).await;
    });
    match ctx.restore_disconnect_task.lock() {
        Ok(mut slot) => {
            if let Some(previous) = slot.replace(task) {
                previous.abort();
            }
        }
        Err(_) => task.abort(),
    }
    Ok(Some(restored_ws))
}

async fn send_disconnect_request<W: WebSocketPort>(
    ws: &W,
    disconnect_msg: Option<ScMessage>,
    timeout_ms: u64,
) {
    if let Some(msg) = disconnect_msg {
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        match tokio::time::timeout(Duration::from_millis(timeout_ms), ws.send(&buf)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                warn!(%e, "BACnet/SC failover disconnect request failed during primary restore");
            }
            Err(_) => {
                warn!("BACnet/SC failover disconnect request timed out during primary restore");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sc::LoopbackWebSocket;

    /// Stop marks the connection Disconnecting before its best-effort send.
    /// A restore already ready in that window must not resurrect it, and a
    /// failed probe must not apply its VMAC/retry metadata to that connection.
    #[tokio::test]
    async fn primary_restore_completion_after_shutdown_cannot_publish() {
        for success in [true, false] {
            let (live, _hub) = LoopbackWebSocket::pair();
            let live = Arc::new(live);
            let active = Arc::new(Mutex::new(live.clone()));
            let mut connection = ScConnection::new([1; 6], [1; 16]);
            connection.state = ScConnectionState::Connected;
            connection.hub_vmac = Some([0x20; 6]);
            connection.build_disconnect_request().unwrap();
            let conn = Arc::new(Mutex::new(connection));
            let (state, _) = watch::channel(ScConnectionState::Disconnecting);
            let disconnect = Arc::new(StdMutex::new(None));
            let effective = AtomicU16::new(1000);
            let mut probe = ScConnection::new([2; 6], [1; 16]);
            probe.state = ScConnectionState::Connected;
            probe.hub_vmac = Some([0x10; 6]);
            probe.connect_retry_allowed = false;
            let (candidate, _primary_hub) = LoopbackWebSocket::pair();
            let candidate = PrimaryCandidate {
                socket: if success {
                    Ok(Arc::new(candidate))
                } else {
                    Err(Error::Encoding("refused probe".into()))
                },
                connection: probe,
            };
            let result = publish_primary_restore(
                &PrimaryRestoreContext {
                    active_ws: &active,
                    conn: &conn,
                    restore_disconnect_task: &disconnect,
                    state_tx: &state,
                    connect_timeout_ms: 500,
                    effective_max_apdu_length: &effective,
                },
                &live,
                candidate,
            )
            .await
            .unwrap();
            assert!(result.is_none());
            let c = conn.lock().await;
            assert_eq!(c.state, ScConnectionState::Disconnecting);
            assert_eq!(c.local_vmac, [1; 6]);
            assert_eq!(c.hub_vmac, Some([0x20; 6]));
            assert!(c.connect_retry_allowed);
            assert!(Arc::ptr_eq(&*active.lock().await, &live));
            assert_eq!(*state.borrow(), ScConnectionState::Disconnecting);
            assert_eq!(effective.load(std::sync::atomic::Ordering::Relaxed), 1000);
            assert!(disconnect.lock().unwrap().is_none());
        }
    }
}
