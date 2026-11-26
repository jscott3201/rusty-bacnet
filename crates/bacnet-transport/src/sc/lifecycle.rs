//! Transport lifecycle teardown shared by stop and drop paths.
//!
//! Moved out of the transport loop file to keep that file within the
//! repository file-size cap. Registered listener shutdown shares transport teardown.

use std::time::Duration;

use tokio::task::JoinHandle;

use crate::port::TransportPort;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{ScConnectionState, ScTransport, WebSocketPort};
use crate::sc_frame::encode_sc_message;
use bacnet_types::error::Error;
use bacnet_types::sync::MutexExt;
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
        let restore = self.restore_disconnect_task.locked().take();
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
        self.end_at_once();
        let task = self.recv_task.locked().take();
        let restore_task = self.restore_disconnect_task.locked().take();
        (task, restore_task)
    }

    /// Abort the receive and restore tasks in place, where a stop finds and
    /// joins them, end the direct links and close the connection's record.
    fn end_at_once(&self) {
        self.direct_membership.retire_all();
        if let Some(task) = &*self.recv_task.locked() {
            task.abort();
        }
        if let Some(task) = &*self.restore_disconnect_task.locked() {
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
            // Direct NPDUs no longer reach the receiver, which can end.
            #[cfg(feature = "sc-tls")]
            shared.end_intake();
        }
    }

    /// The abort, through `&self` (#2617): nothing is said to the hub or a
    /// direct peer. The direct listener is sealed, the receive and restore
    /// tasks are aborted in place, for a later stop to join, the direct
    /// links end, and the sockets given before start are let go of. Every
    /// later send fails with [`crate::port::stopped_for_good`]'s error. The
    /// hub socket comes out of the send slot now, and again as the aborted
    /// receive task drops ([`ReleaseOnAbort`]), which also takes out one it
    /// published as it was aborted. A slot a send holds is emptied on the
    /// runtime: the current one, or, from a thread with none, the one the
    /// transport started on. The transport does not start again.
    pub(super) fn abort_and_release(&self) {
        self.abort_mark.set();
        self.stop_gate.abort();
        self.seal_direct_listener();
        self.end_at_once();
        drop(lock_unstarted(&self.ws).take());
        drop(lock_unstarted(&self.failover_ws).take());
        if let Some(slot) = &self.ws_shared {
            self.abort_mark.release(slot);
        }
    }
}

/// What an abort through `&self` shares with the receive task (#2617).
#[derive(Debug, Default)]
pub(super) struct AbortMark {
    aborted: AtomicBool,
    /// The runtime the transport started on, for an abort from a thread
    /// with none.
    runtime: std::sync::Mutex<Option<tokio::runtime::Handle>>,
}

impl AbortMark {
    pub(super) fn set(&self) {
        self.aborted.store(true, Ordering::Release);
    }

    pub(super) fn is_set(&self) -> bool {
        self.aborted.load(Ordering::Acquire)
    }

    /// Remember the runtime a start runs on.
    pub(super) fn started_on(&self, runtime: tokio::runtime::Handle) {
        *self.runtime.locked() = Some(runtime);
    }

    /// Take the hub socket out of `slot`: now, or, while something holds
    /// the slot, on a runtime as soon as it is free.
    pub(super) fn release<T: Send + Sync + 'static>(&self, slot: &super::HubSlot<T>) {
        if let Ok(mut hub) = slot.try_lock() {
            drop(hub.take());
            return;
        }
        let runtime = tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.runtime.locked().clone());
        if let Some(runtime) = runtime {
            let slot = Arc::clone(slot);
            runtime.spawn(async move { drop(slot.lock().await.take()) });
        }
    }
}

/// Held by the receive task: once the transport is aborted, its drop takes
/// out the hub socket, whatever the task published before it ended.
pub(super) struct ReleaseOnAbort<T: Send + Sync + 'static> {
    pub(super) slot: super::HubSlot<T>,
    pub(super) mark: Arc<AbortMark>,
}

impl<T: Send + Sync + 'static> Drop for ReleaseOnAbort<T> {
    fn drop(&mut self) {
        if self.mark.is_set() {
            self.mark.release(&self.slot);
        }
    }
}

/// An unstarted socket's slot, locked.
fn lock_unstarted<W>(slot: &std::sync::Mutex<Option<W>>) -> std::sync::MutexGuard<'_, Option<W>> {
    slot.locked()
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
