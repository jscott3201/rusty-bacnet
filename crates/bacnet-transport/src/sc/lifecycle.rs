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
    /// The stop, through `&self` (#2617). The direct links' shutdown and the
    /// listener's seal are idempotent, so the shared state they hang off
    /// stays until the transport drops; the hub socket is taken out of its
    /// slot once the receive task, the only other publisher, is joined.
    pub(super) async fn stop_owned(&self) -> Result<(), Error> {
        let budget = tokio::time::Instant::now() + STOP_SEND_BUDGET;
        self.direct_membership.retire_all();
        self.seal_direct_listener();
        let direct = self.direct.clone();
        let direct = async {
            if let Some(shared) = direct {
                shared.shutdown(budget).await;
            }
        };
        let hub = self.say_goodbye_to_hub(budget);
        // A primary restore's goodbye to the failover hub finishes in the
        // same budget rather than being cut off (#2520).
        let restore = self
            .restore_disconnect_task
            .lock()
            .ok()
            .and_then(|mut task| task.take());
        let restore = async {
            if let Some(mut task) = restore
                && tokio::time::timeout_at(budget, &mut task).await.is_err()
            {
                task.abort();
                let _ = task.await;
            }
        };
        tokio::join!(direct, hub, restore);

        let (recv_task, restore_task) = self.abort_background_tasks();
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

        // A stop before start lets go of the sockets it was given.
        drop(lock_unstarted(&self.ws).take());
        drop(lock_unstarted(&self.failover_ws).take());
        // Nothing publishes a socket now: take it out, so every later send
        // fails, and let it go once the lock is free.
        if let Some(slot) = &self.ws_shared {
            let socket = slot.lock().await.take();
            drop(socket);
        }
        if let Some(conn) = &self.connection {
            let mut c = conn.lock().await;
            c.state = ScConnectionState::Disconnected;
            self.state_tx.send_replace(c.state);
        }
        Ok(())
    }

    /// The orderly end of the hub connection, all before `budget`: a
    /// Disconnect-Request when connected, a wait of at most the close grace
    /// for its Disconnect-ACK (or NAK), which the receive task reads, then a
    /// close with 1000, as the requester does in Annex AB.6.2.2. The
    /// answering close is not awaited. The close also goes out when no
    /// Disconnect-Request does, as mid-reconnect, so the hub learns the
    /// socket is done with.
    async fn say_goodbye_to_hub(&self, budget: tokio::time::Instant) {
        let (Some(ws), Some(conn)) = (&self.ws_shared, &self.connection) else {
            return;
        };
        let mut states = self.state_tx.subscribe();
        let (ws, disconnect_msg) = {
            let ws = ws.lock().await;
            // Already taken out: a stop said goodbye before.
            let Some(ws) = ws.as_ref() else {
                return;
            };
            let mut c = conn.lock().await;
            let disconnect_msg = c.build_disconnect_request().ok();
            if disconnect_msg.is_some() {
                self.state_tx.send_replace(c.state);
            }
            (std::sync::Arc::clone(ws), disconnect_msg)
        };
        if let Some(msg) = disconnect_msg {
            let mut buf = BytesMut::new();
            encode_sc_message(&mut buf, &msg);
            let sent = tokio::time::timeout_at(budget, ws.send(&buf)).await;
            if matches!(sent, Ok(Ok(()))) && ws.awaits_disconnect_ack() {
                let answered = states.wait_for(|state| *state == ScConnectionState::Disconnected);
                let wait = budget.min(tokio::time::Instant::now() + super::CLOSE_GRACE);
                let _ = tokio::time::timeout_at(wait, answered).await;
            }
        }
        let grace = budget.min(tokio::time::Instant::now() + super::CLOSE_GRACE);
        let _ = tokio::time::timeout_at(grace, ws.close(Some(super::CLOSE_NORMAL))).await;
    }

    /// Seal the direct listener; sealing again changes nothing.
    pub(super) fn seal_direct_listener(&self) {
        #[cfg(feature = "sc-tls")]
        if let Some(shutdown) = &self.direct_listener_shutdown {
            shutdown.send_replace(true);
        }
    }

    /// Abort the receive and restore tasks and close the connection's
    /// record, through `&self`: what a stop and an abort both do. Returns the
    /// tasks for a stop to join.
    pub(super) fn abort_background_tasks(
        &self,
    ) -> (Option<JoinHandle<()>>, Option<JoinHandle<()>>) {
        self.direct_membership.retire_all();
        let task = self
            .recv_task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
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
        // Pending Address-Resolution waiters observe closure via their hub
        // send or timeout and fall back to the hub path.
        if let Some(shared) = &self.direct {
            shared.disable();
        }
        (task, restore_task)
    }

    /// [`Self::abort_background_tasks`], then drop the sockets and the
    /// direct discovery state, for `abort` and drop.
    pub(super) fn abort_background_task_and_drop_sockets(&mut self) {
        self.seal_direct_listener();
        #[cfg(feature = "sc-tls")]
        {
            self.direct_listener_shutdown = None;
        }
        self.direct_intake = advertisement::DirectIntake::default();
        let _ = self.abort_background_tasks();
        self.ws_shared = None;
        self.connection = None;
        *self
            .ws
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        *self
            .failover_ws
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.direct = None;
    }
}

/// An unstarted socket's slot, locked.
fn lock_unstarted<W>(slot: &std::sync::Mutex<Option<W>>) -> std::sync::MutexGuard<'_, Option<W>> {
    slot.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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

#[cfg(test)]
#[path = "disconnect_wait_tests.rs"]
mod disconnect_wait_tests;
