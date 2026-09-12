//! Transport lifecycle teardown shared by stop and drop paths.
//!
//! Moved out of the transport loop file to keep that file within the
//! repository file-size cap; behavior is unchanged.

use std::sync::atomic::Ordering;

use tokio::task::JoinHandle;

use crate::port::TransportPort;

use super::{ScConnectionState, ScTransport, WebSocketPort, DEFAULT_MAX_APDU_LENGTH};

impl<W: WebSocketPort> ScTransport<W> {
    pub(super) fn abort_background_task_and_drop_sockets(
        &mut self,
    ) -> (Option<JoinHandle<()>>, Option<JoinHandle<()>>) {
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
        if let Some(conn) = &self.connection {
            if let Ok(mut c) = conn.try_lock() {
                c.state = ScConnectionState::Disconnected;
            }
        }
        self.effective_max_apdu_length
            .store(DEFAULT_MAX_APDU_LENGTH, Ordering::Relaxed);
        self.state_tx.send_replace(ScConnectionState::Disconnected);
        self.ws_shared = None;
        self.connection = None;
        self.ws = None;
        self.failover_ws = None;
        // Direct discovery state is dropped with the transport; pending
        // Address-Resolution waiters observe closure via their hub send or
        // timeout and fall back to the hub path.
        self.direct = None;
        (task, restore_task)
    }
}

impl<W: WebSocketPort> Drop for ScTransport<W> {
    fn drop(&mut self) {
        self.abort();
    }
}
