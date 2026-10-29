//! Transport lifecycle teardown shared by stop and drop paths.
//!
//! Moved out of the transport loop file to keep that file within the
//! repository file-size cap. Registered listener shutdown shares transport teardown.

use tokio::task::JoinHandle;

use crate::port::TransportPort;

use super::{ScConnectionState, ScTransport, WebSocketPort, advertisement};
use crate::sc_frame::encode_sc_message;
use bacnet_types::error::Error;
use bytes::BytesMut;

impl<W: WebSocketPort> ScTransport<W> {
    pub(super) async fn stop_owned(&mut self) -> Result<(), Error> {
        self.direct_membership.retire_all();
        self.seal_direct_listener();
        if let Some(shared) = self.direct.take() {
            shared.shutdown().await;
        }
        self.direct_intake = advertisement::DirectIntake::default();
        // Attempt clean disconnect: send DisconnectRequest via the WebSocket
        if let (Some(ws), Some(conn)) = (&self.ws_shared, &self.connection) {
            let (ws, disconnect_msg) = {
                let ws = ws.lock().await;
                let mut c = conn.lock().await;
                let disconnect_msg = c.build_disconnect_request().ok();
                if disconnect_msg.is_some() {
                    self.state_tx.send_replace(c.state);
                }
                (ws.clone(), disconnect_msg)
            };
            if let Some(msg) = disconnect_msg {
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &msg);
                // Best-effort send — don't block indefinitely
                let _ =
                    tokio::time::timeout(std::time::Duration::from_secs(2), ws.send(&buf)).await;
            }
        }

        let conn_for_state = self.connection.clone();
        let (recv_task, restore_task) = self.abort_background_task_and_drop_sockets();
        if let Some(task) = recv_task {
            let _ = task.await;
        }
        if let Some(task) = restore_task {
            let _ = task.await;
        }
        // A reconnect finishing on another worker can publish a connection
        // after the abort above; once the task is gone, close the record again.
        self.observer
            .disconnected(super::ScDisconnectCause::LocalClose);

        if let Some(conn) = conn_for_state {
            let mut c = conn.lock().await;
            c.state = ScConnectionState::Disconnected;
            self.state_tx.send_replace(c.state);
        }
        Ok(())
    }

    pub(super) fn seal_direct_listener(&mut self) {
        #[cfg(feature = "sc-tls")]
        if let Some(shutdown) = self.direct_listener_shutdown.take() {
            shutdown.send_replace(true);
        }
    }

    pub(super) fn abort_background_task_and_drop_sockets(
        &mut self,
    ) -> (Option<JoinHandle<()>>, Option<JoinHandle<()>>) {
        self.direct_membership.retire_all();
        let task = self.recv_task.take();
        if let Some(task) = &task {
            task.abort();
        }
        let restore_task = self
            .restore_disconnect_task
            .lock()
            .ok()
            .and_then(|mut task| task.take());
        if let Some(task) = &restore_task {
            task.abort();
        }
        if let Some(conn) = &self.connection
            && let Ok(mut c) = conn.try_lock()
        {
            c.state = ScConnectionState::Disconnected;
        }
        self.hub_limits.reset_apdu();
        self.state_tx.send_replace(ScConnectionState::Disconnected);
        self.observer
            .disconnected(super::ScDisconnectCause::LocalClose);
        self.ws_shared = None;
        self.connection = None;
        self.ws = None;
        self.failover_ws = None;
        // Direct discovery state is dropped with the transport; pending
        // Address-Resolution waiters observe closure via their hub send or
        // timeout and fall back to the hub path.
        if let Some(shared) = self.direct.take() {
            shared.disable();
        }
        (task, restore_task)
    }
}

impl<W: WebSocketPort> Drop for ScTransport<W> {
    fn drop(&mut self) {
        self.abort();
    }
}
