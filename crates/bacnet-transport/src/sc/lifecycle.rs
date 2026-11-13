//! Transport lifecycle teardown shared by stop and drop paths.
//!
//! Moved out of the transport loop file to keep that file within the
//! repository file-size cap. Registered listener shutdown shares transport teardown.

use std::time::Duration;

use tokio::task::JoinHandle;

use crate::port::TransportPort;

use super::{ScConnectionState, ScTransport, WebSocketPort, advertisement};
use crate::sc_frame::encode_sc_message;
use bacnet_types::error::Error;
use bytes::BytesMut;

/// How long [`ScTransport::stop`](crate::port::TransportPort::stop) spends
/// saying goodbye before it drops the sockets: the hub's Disconnect-Request
/// and close, and each outbound direct link's, all at once. A hub or peer
/// that never reads cannot hold `stop` longer (#2520).
pub(super) const STOP_SEND_BUDGET: Duration = Duration::from_secs(2);

impl<W: WebSocketPort> ScTransport<W> {
    pub(super) async fn stop_owned(&mut self) -> Result<(), Error> {
        let budget = tokio::time::Instant::now() + STOP_SEND_BUDGET;
        self.direct_membership.retire_all();
        self.seal_direct_listener();
        let direct = self.direct.take();
        let direct = async {
            if let Some(shared) = direct {
                shared.shutdown(budget).await;
            }
        };
        let hub = self.say_goodbye_to_hub(budget);
        tokio::join!(direct, hub);
        self.direct_intake = advertisement::DirectIntake::default();

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

    /// The orderly end of the hub connection: a Disconnect-Request when
    /// connected, then a close with 1000 (Annex AB.7.5.5), all before
    /// `budget`. Neither the Disconnect-ACK nor the answering close is
    /// awaited. The close also goes out when no Disconnect-Request does, as
    /// mid-reconnect, so the hub learns the socket is done with.
    async fn say_goodbye_to_hub(&self, budget: tokio::time::Instant) {
        let (Some(ws), Some(conn)) = (&self.ws_shared, &self.connection) else {
            return;
        };
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
            let _ = tokio::time::timeout_at(budget, ws.send(&buf)).await;
        }
        let grace = budget.min(tokio::time::Instant::now() + super::CLOSE_GRACE);
        let _ = tokio::time::timeout_at(grace, ws.close(Some(super::CLOSE_NORMAL))).await;
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

#[cfg(all(test, feature = "sc-tls"))]
#[path = "close_wire_tests.rs"]
mod close_wire_tests;

#[cfg(test)]
#[path = "close_rule_tests.rs"]
mod close_rule_tests;
