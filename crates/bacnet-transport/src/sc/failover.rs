//! BACnet/SC primary/failover hub switching helpers.

use std::sync::Arc;
use std::time::Duration;

use bacnet_types::error::Error;
use bytes::BytesMut;
use tokio::sync::Mutex;
use tracing::warn;

use crate::sc_frame::{encode_sc_message, ScMessage};

use super::{
    connector::dial_connector, handshake::perform_handshake, ScConnection, WebSocketConnector,
    WebSocketPort,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActiveHub {
    Primary,
    Failover,
}

pub(super) async fn attempt_primary_restore<W: WebSocketPort>(
    primary_ws: &Arc<W>,
    primary_connector: Option<&WebSocketConnector<W>>,
    current_ws: &Arc<W>,
    active_ws: &Arc<Mutex<Arc<W>>>,
    conn: &Arc<Mutex<ScConnection>>,
    connect_timeout_ms: u64,
) -> Result<Arc<W>, Error> {
    let restored_ws = if let Some(connector) = primary_connector {
        Arc::new(dial_connector(connector, connect_timeout_ms).await?)
    } else {
        primary_ws.clone()
    };
    let probe_conn = Arc::new(Mutex::new(primary_probe_connection(conn).await));
    if let Err(e) = perform_handshake(&*restored_ws, &probe_conn, connect_timeout_ms).await {
        absorb_failed_probe(conn, &probe_conn).await;
        return Err(e);
    }

    let disconnect_msg = disconnect_request_from(conn).await;
    let restored = probe_conn.lock().await.clone();
    let mut current = active_ws.lock().await;
    let mut c = conn.lock().await;
    *c = restored;
    *current = restored_ws.clone();
    drop(c);
    drop(current);

    let disconnect_ws = current_ws.clone();
    let _ = tokio::spawn(async move {
        send_disconnect_request(&*disconnect_ws, disconnect_msg, connect_timeout_ms).await;
    });
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
