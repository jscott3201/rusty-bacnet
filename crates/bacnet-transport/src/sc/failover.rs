//! BACnet/SC primary/failover hub switching helpers.

use std::sync::Arc;

use bacnet_types::error::Error;
use bytes::BytesMut;
use tokio::sync::Mutex;
use tracing::warn;

use crate::sc_frame::encode_sc_message;

use super::{handshake::perform_handshake, ScConnection, WebSocketPort};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActiveHub {
    Primary,
    Failover,
}

pub(super) async fn attempt_primary_restore<W: WebSocketPort>(
    primary_ws: &Arc<W>,
    current_ws: &Arc<W>,
    active_ws: &Arc<Mutex<Arc<W>>>,
    conn: &Arc<Mutex<ScConnection>>,
    connect_timeout_ms: u64,
) -> Result<(), Error> {
    let probe_conn = Arc::new(Mutex::new(primary_probe_connection(conn).await));
    if let Err(e) = perform_handshake(&**primary_ws, &probe_conn, connect_timeout_ms).await {
        absorb_failed_probe(conn, &probe_conn).await;
        return Err(e);
    }

    send_disconnect_request(&**current_ws, conn).await;

    let restored = probe_conn.lock().await.clone();
    let mut current = active_ws.lock().await;
    let mut c = conn.lock().await;
    *c = restored;
    *current = primary_ws.clone();
    Ok(())
}

async fn primary_probe_connection(conn: &Arc<Mutex<ScConnection>>) -> ScConnection {
    let c = conn.lock().await;
    let mut probe = ScConnection::new(c.local_vmac, c.device_uuid);
    probe.max_bvlc_length = c.max_bvlc_length;
    probe.max_apdu_length = c.max_apdu_length;
    probe
}

async fn absorb_failed_probe(
    conn: &Arc<Mutex<ScConnection>>,
    probe_conn: &Arc<Mutex<ScConnection>>,
) {
    let probe = probe_conn.lock().await;
    let mut c = conn.lock().await;
    c.local_vmac = probe.local_vmac;
    if !probe.connect_retry_allowed {
        c.connect_retry_allowed = false;
    }
}

async fn send_disconnect_request<W: WebSocketPort>(ws: &W, conn: &Arc<Mutex<ScConnection>>) {
    let disconnect_msg = {
        let c = conn.lock().await;
        let mut snapshot = c.clone();
        snapshot.build_disconnect_request().ok()
    };
    if let Some(msg) = disconnect_msg {
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        if let Err(e) = ws.send(&buf).await {
            warn!(%e, "BACnet/SC failover disconnect request failed during primary restore");
        }
    }
}
