//! Connect-Request / Connect-Accept handshake for BACnet/SC transports.

use std::sync::Arc;
use std::time::Duration;

use bacnet_types::error::Error;
use bytes::BytesMut;
use tokio::sync::{watch, Mutex};
use tracing::{debug, warn};

use crate::sc_frame::{
    decode_sc_bvlc_result, decode_sc_message, encode_sc_message, ScBvlcResult, ScFunction,
};

use super::{heartbeat, ScConnection, ScConnectionState, WebSocketPort};

fn notify_state(state_tx: Option<&watch::Sender<ScConnectionState>>, state: ScConnectionState) {
    if let Some(state_tx) = state_tx {
        state_tx.send_replace(state);
    }
}

/// Perform the Connect-Request / Connect-Accept handshake on a WebSocket.
///
/// Used for both the initial connection and reconnection attempts.
pub(super) async fn perform_handshake<W: WebSocketPort>(
    ws: &W,
    conn: &Arc<Mutex<ScConnection>>,
    state_tx: Option<&watch::Sender<ScConnectionState>>,
    timeout_ms: u64,
) -> Result<(), Error> {
    {
        let mut c = conn.lock().await;
        let msg = c.build_connect_request();
        notify_state(state_tx, c.state);
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        if let Err(e) = ws.send(&buf).await {
            c.abort_connect();
            notify_state(state_tx, c.state);
            return Err(e);
        }
    }

    let timeout_dur = Duration::from_millis(timeout_ms);
    let accept_result = tokio::time::timeout(timeout_dur, async {
        loop {
            let data = match ws.recv().await {
                Ok(data) => data,
                Err(e) => {
                    let mut c = conn.lock().await;
                    c.abort_connect();
                    notify_state(state_tx, c.state);
                    return Err(e);
                }
            };
            if data.len() > conn.lock().await.max_bvlc_length as usize {
                warn!("BACnet/SC connect frame exceeds local Max-BVLC-Length, dropping");
                continue;
            }
            let msg = match decode_sc_message(&data) {
                Ok(msg) => msg,
                Err(e) if heartbeat::is_bvlc_result_wire(&data) => {
                    let mut c = conn.lock().await;
                    c.abort_connect();
                    notify_state(state_tx, c.state);
                    return Err(Error::Encoding(format!(
                        "malformed BACnet/SC BVLC-Result during connect: {e}"
                    )));
                }
                Err(e) => {
                    let mut c = conn.lock().await;
                    c.abort_connect();
                    notify_state(state_tx, c.state);
                    return Err(e);
                }
            };
            if msg.function == ScFunction::ConnectAccept {
                return Ok::<_, Error>(msg);
            }
            if msg.function == ScFunction::Result {
                let result = decode_sc_bvlc_result(&msg);
                let mut c = conn.lock().await;
                return match result {
                    Ok(result) => match &result {
                        ScBvlcResult::Nak {
                            result_for,
                            error_class,
                            error_code,
                            error_details,
                            ..
                        } => {
                            let duplicate_vmac =
                                c.handle_connect_result(msg.message_id, &result)?;
                            notify_state(state_tx, c.state);
                            let duplicate_note = if duplicate_vmac {
                                "; selected new Random-48 local VMAC"
                            } else {
                                ""
                            };
                            Err(Error::Encoding(format!(
                                "BACnet/SC BVLC-Result NAK during connect: function={:#x} \
                                 error_class={} error_code={} details={}{}",
                                result_for.to_raw(),
                                error_class,
                                error_code,
                                error_details,
                                duplicate_note
                            )))
                        }
                        ScBvlcResult::Ack { result_for } => {
                            let _ = c.handle_connect_result(msg.message_id, &result);
                            notify_state(state_tx, c.state);
                            Err(Error::Encoding(format!(
                                "unexpected BACnet/SC BVLC-Result ACK during connect: function={:#x}",
                                result_for.to_raw()
                            )))
                        }
                    },
                    Err(e) => {
                        c.abort_connect();
                        notify_state(state_tx, c.state);
                        Err(Error::Encoding(format!(
                            "malformed BACnet/SC BVLC-Result during connect: {e}"
                        )))
                    }
                };
            }
        }
    })
    .await;

    match accept_result {
        Ok(Ok(msg)) => {
            let mut c = conn.lock().await;
            if c.handle_connect_accept(&msg) {
                notify_state(state_tx, c.state);
                debug!("BACnet/SC connected");
                Ok(())
            } else {
                c.abort_connect();
                notify_state(state_tx, c.state);
                Err(Error::Encoding(
                    "BACnet/SC Connect-Accept did not match pending Connect-Request".into(),
                ))
            }
        }
        Ok(Err(e)) => {
            let mut c = conn.lock().await;
            c.abort_connect();
            notify_state(state_tx, c.state);
            Err(e)
        }
        Err(_) => {
            let mut c = conn.lock().await;
            c.abort_connect();
            notify_state(state_tx, c.state);
            Err(Error::Encoding("BACnet/SC connect timeout".into()))
        }
    }
}
