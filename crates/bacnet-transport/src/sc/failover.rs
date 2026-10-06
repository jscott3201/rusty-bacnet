//! BACnet/SC primary/failover hub switching helpers.

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

/// Borrowed state a primary-restore attempt reads and updates.
pub(super) struct PrimaryRestoreContext<'a, W> {
    /// Retired primary socket, when it can be reused as-is.
    pub(super) primary_ws: Option<&'a Arc<W>>,
    /// Connector used to dial a fresh primary socket.
    pub(super) primary_connector: Option<&'a WebSocketConnector<W>>,
    /// Socket slot shared with the send path.
    pub(super) active_ws: &'a Arc<Mutex<Arc<W>>>,
    /// Shared connection state machine.
    pub(super) conn: &'a Arc<Mutex<ScConnection>>,
    /// Slot holding the in-flight disconnect of the replaced failover socket.
    pub(super) restore_disconnect_task: &'a Arc<StdMutex<Option<JoinHandle<()>>>>,
    /// Connection-state publisher.
    pub(super) state_tx: &'a watch::Sender<ScConnectionState>,
    /// Connect and handshake timeout in milliseconds.
    pub(super) connect_timeout_ms: u64,
    /// Published effective maximum APDU length.
    pub(super) effective_max_apdu_length: &'a AtomicU16,
}

pub(super) async fn attempt_primary_restore<W: WebSocketPort>(
    ctx: &PrimaryRestoreContext<'_, W>,
    current_ws: &Arc<W>,
) -> Result<Arc<W>, Error> {
    let PrimaryRestoreContext {
        primary_ws,
        primary_connector,
        active_ws,
        conn,
        restore_disconnect_task,
        state_tx,
        connect_timeout_ms,
        effective_max_apdu_length,
    } = *ctx;
    let restored_ws = if let Some(connector) = primary_connector {
        Arc::new(dial_connector(connector, connect_timeout_ms).await?)
    } else if let Some(primary_ws) = primary_ws {
        primary_ws.clone()
    } else {
        return Err(Error::Encoding(
            "SC retired primary requires a fresh connector".into(),
        ));
    };
    let probe_conn = Arc::new(Mutex::new(primary_probe_connection(conn).await));
    if let Err(e) = perform_handshake(&*restored_ws, &probe_conn, None, connect_timeout_ms).await {
        absorb_failed_probe(conn, &probe_conn).await;
        return Err(e);
    }

    let disconnect_msg = disconnect_request_from(conn).await;
    let restored = probe_conn.lock().await.clone();
    let mut current = active_ws.lock().await;
    let mut c = conn.lock().await;
    *c = restored;
    *current = restored_ws.clone();
    publish_effective_max_apdu_length(effective_max_apdu_length, &c);
    state_tx.send_replace(c.state);
    drop(c);
    drop(current);

    let disconnect_ws = current_ws.clone();
    let task = tokio::spawn(async move {
        send_disconnect_request(&*disconnect_ws, disconnect_msg, connect_timeout_ms).await;
    });
    match restore_disconnect_task.lock() {
        Ok(mut slot) => {
            if let Some(previous) = slot.replace(task) {
                previous.abort();
            }
        }
        Err(_) => task.abort(),
    }
    Ok(restored_ws)
}

async fn primary_probe_connection(conn: &Arc<Mutex<ScConnection>>) -> ScConnection {
    let c = conn.lock().await;
    c.connect_probe()
}

async fn absorb_failed_probe(
    conn: &Arc<Mutex<ScConnection>>,
    probe_conn: &Arc<Mutex<ScConnection>>,
) {
    let probe = probe_conn.lock().await;
    let mut c = conn.lock().await;
    c.absorb_failed_probe(&probe);
}

async fn disconnect_request_from(conn: &Arc<Mutex<ScConnection>>) -> Option<ScMessage> {
    let c = conn.lock().await;
    let mut snapshot = c.clone();
    snapshot.build_disconnect_request().ok()
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
