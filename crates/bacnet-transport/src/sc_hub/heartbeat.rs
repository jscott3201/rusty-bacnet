//! Ownership of the existing hub-originated liveness probe (a local extension,
//! not a substitute for the initiating node's Annex AB.6.3 keepalive duty).
//!
//! Wire ACK matching is only 16 bits (AB.2.14–15). Local generations protect
//! asynchronous work, not a delayed wire ACK after reuse of the same message ID.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures_util::SinkExt;
use tokio::sync::{Mutex, Notify};
use tokio_tungstenite::tungstenite::{Error, Message};
use tracing::warn;

use super::helpers::now_secs;
use super::{Clients, HubClient, Vmac, WsSink};
use crate::sc_frame::{encode_sc_message, ScFunction, ScMessage};

const IDLE_THRESHOLD_SECS: u64 = 60;
const ACK_TIMEOUT_SECS: u64 = 5;
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

// Private I/O boundary: tests use the real TLS/WebSocket send, but control when
// its completion becomes visible to the sweep and supply the wall-clock value.
pub(super) trait HeartbeatIo: Sync {
    fn now_secs(&self) -> u64;
    fn send(
        &self,
        sink: &mut WsSink,
        frame: Message,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}

pub(super) struct SocketIo;

impl HeartbeatIo for SocketIo {
    fn now_secs(&self) -> u64 {
        now_secs()
    }

    async fn send(&self, sink: &mut WsSink, frame: Message) -> Result<(), Error> {
        sink.send(frame).await
    }
}

pub(super) async fn sweep(clients: &Clients, next_msg_id: &AtomicU16, io: &impl HeartbeatIo) {
    let (timed_out, idle): (Vec<_>, Vec<_>) = snapshot(clients, io.now_secs())
        .await
        .into_iter()
        .partition(|(_, decision)| *decision == HubHeartbeatSweepDecision::RemoveTimedOut);
    // Retire expired clients before potentially waiting on an idle client's sink.
    // Consume their tokens here: do not retain expired sockets during later sends.
    for (attempt, _) in timed_out {
        retire(clients, &attempt, Retirement::AckTimeout, io).await;
    }
    for (candidate, _) in idle {
        send_request(clients, &candidate, next_msg_id, io).await;
    }
}

#[derive(Clone)]
pub(super) struct Attempt {
    pub(super) vmac: Vmac,
    pub(super) sink: Arc<Mutex<WsSink>>,
    pub(super) generation: u64,
}

impl Attempt {
    fn matches(&self, client: &HubClient) -> bool {
        Arc::ptr_eq(&self.sink, &client.sink) && self.generation == client.heartbeat.generation
    }
}

pub(super) async fn snapshot(
    clients: &Clients,
    now: u64,
) -> Vec<(Attempt, HubHeartbeatSweepDecision)> {
    clients
        .lock()
        .await
        .iter()
        .filter_map(|(vmac, client)| {
            if client.closed.load(Ordering::Acquire) {
                return None;
            }
            let decision = decision(client, now);
            (decision != HubHeartbeatSweepDecision::Keep).then(|| {
                (
                    Attempt {
                        vmac: *vmac,
                        sink: client.sink.clone(),
                        generation: client.heartbeat.generation,
                    },
                    decision,
                )
            })
        })
        .collect()
}

fn decision(client: &HubClient, now: u64) -> HubHeartbeatSweepDecision {
    hub_heartbeat_sweep_decision(
        now,
        client.last_activity.load(Ordering::Acquire),
        client.heartbeat.pending,
        IDLE_THRESHOLD_SECS,
        ACK_TIMEOUT_SECS,
    )
}

pub(super) async fn reserve(
    clients: &Clients,
    candidate: &Attempt,
    message_id: u16,
    io: &impl HeartbeatIo,
) -> Option<Attempt> {
    let mut map = clients.lock().await;
    let now = io.now_secs(); // fresh per attempt, not the earlier sweep snapshot
    let client = map.get_mut(&candidate.vmac).filter(|client| {
        Arc::ptr_eq(&client.sink, &candidate.sink)
            && !client.closed.load(Ordering::Acquire)
            && decision(client, now) == HubHeartbeatSweepDecision::SendRequest
    })?;
    let mut attempt = Attempt {
        vmac: candidate.vmac,
        sink: client.sink.clone(),
        generation: client.heartbeat.generation,
    };
    let Some(generation) = client.heartbeat.generation.checked_add(1) else {
        // Never recycle a local generation within this registration.
        let notify = retire_locked(&mut map, &attempt, Retirement::GenerationExhausted, now);
        drop(map);
        if let Some(notify) = notify {
            notify.notify_one();
            warn!(vmac = ?attempt.vmac, "Hub: heartbeat generation exhausted, removing client");
        }
        return None;
    };
    client.heartbeat = HubHeartbeat {
        generation,
        pending: Some(PendingHeartbeat {
            message_id,
            published_at: now,
        }),
    };
    attempt.generation = generation;
    Some(attempt)
}

pub(super) async fn send_request(
    clients: &Clients,
    candidate: &Attempt,
    next_msg_id: &AtomicU16,
    io: &impl HeartbeatIo,
) {
    let message_id = next_msg_id.fetch_add(1, Ordering::Relaxed); // wire IDs wrap, including zero
    let hb = ScMessage {
        function: ScFunction::HeartbeatRequest,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &hb);
    let frame = Message::Binary(buf.to_vec().into());
    // Encode first; publish atomically before any externally observable send.
    let Some(attempt) = reserve(clients, candidate, message_id, io).await else {
        return;
    };
    let result = tokio::time::timeout(SEND_TIMEOUT, async {
        let mut sink = attempt.sink.lock().await;
        // Only sink -> Clients nesting is permitted. Never hold Clients while
        // waiting for a sink or doing I/O. Revalidate after the sink wait.
        let current = {
            let map = clients.lock().await;
            map.get(&attempt.vmac).is_some_and(|client| {
                attempt.matches(client)
                    && client.heartbeat.pending.is_some()
                    && !client.closed.load(Ordering::Acquire)
            })
        };
        if !current {
            return Ok(());
        }
        // Replacement can still happen after this check. Old-sink work must
        // not mutate its replacement; this is not a no-more-bytes guarantee.
        io.send(&mut sink, frame).await
    })
    .await;
    if let Err(_) | Ok(Err(_)) = result {
        retire(clients, &attempt, Retirement::SendFailed, io).await;
    }
    // Success deliberately does nothing: an immediate ACK may already have
    // cleared pending, or another sweep may already have retired this attempt.
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Retirement {
    SendFailed,
    AckTimeout,
    GenerationExhausted,
}

pub(super) async fn retire(
    clients: &Clients,
    attempt: &Attempt,
    reason: Retirement,
    io: &impl HeartbeatIo,
) -> bool {
    let notify = {
        let mut map = clients.lock().await;
        retire_locked(&mut map, attempt, reason, io.now_secs())
    };
    if let Some(notify) = notify {
        // One owning reader. notify_one retains a permit if close precedes its
        // next select; closed is the authoritative predicate, even after cancellation.
        notify.notify_one();
        warn!(vmac = ?attempt.vmac, ?reason, "Hub: retiring heartbeat client");
        true
    } else {
        false
    }
}

fn retire_locked(
    map: &mut HashMap<Vmac, HubClient>,
    attempt: &Attempt,
    reason: Retirement,
    now: u64,
) -> Option<Arc<Notify>> {
    let client = map
        .get(&attempt.vmac)
        .filter(|client| attempt.matches(client))?;
    if matches!(reason, Retirement::AckTimeout) {
        // A stale timeout snapshot must lose if ACK or a newer attempt won the map.
        if decision(client, now) != HubHeartbeatSweepDecision::RemoveTimedOut {
            return None;
        }
    }
    let mut client = map.remove(&attempt.vmac)?;
    client.heartbeat.pending = None;
    client.closed.store(true, Ordering::Release);
    Some(client.close_notify)
}

// Both fields are owned by the Clients mutex. ACK retains generation so late
// local completion cannot confuse an ACKed newer attempt with an older one.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct HubHeartbeat {
    pub(super) generation: u64,
    pub(super) pending: Option<PendingHeartbeat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingHeartbeat {
    pub(super) message_id: u16,
    pub(super) published_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HubHeartbeatSweepDecision {
    Keep,
    SendRequest,
    RemoveTimedOut,
}

pub(super) fn hub_heartbeat_sweep_decision(
    now_secs: u64,
    last_activity_secs: u64,
    pending: Option<PendingHeartbeat>,
    idle_threshold_secs: u64,
    ack_timeout_secs: u64,
) -> HubHeartbeatSweepDecision {
    if let Some(pending) = pending {
        return if now_secs.saturating_sub(pending.published_at) > ack_timeout_secs {
            HubHeartbeatSweepDecision::RemoveTimedOut
        } else {
            HubHeartbeatSweepDecision::Keep
        };
    }

    if now_secs.saturating_sub(last_activity_secs) > idle_threshold_secs {
        HubHeartbeatSweepDecision::SendRequest
    } else {
        HubHeartbeatSweepDecision::Keep
    }
}

pub(super) fn heartbeat_ack_matches_pending(
    message_id: u16,
    pending: Option<PendingHeartbeat>,
) -> bool {
    pending.is_some_and(|pending| pending.message_id == message_id)
}

pub(super) async fn clear_matching_heartbeat_ack(
    clients: &Clients,
    registered_vmac: Vmac,
    sink: &Arc<Mutex<WsSink>>,
    message_id: u16,
) {
    let mut map = clients.lock().await;
    let Some(client) = map
        .get_mut(&registered_vmac)
        .filter(|client| Arc::ptr_eq(&client.sink, sink))
    else {
        return;
    };

    if heartbeat_ack_matches_pending(message_id, client.heartbeat.pending) {
        client.heartbeat.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_keeps_active_client_without_pending_heartbeat() {
        assert_eq!(
            hub_heartbeat_sweep_decision(100, 80, None, 60, 5),
            HubHeartbeatSweepDecision::Keep
        );
    }

    #[test]
    fn sweep_sends_request_to_idle_client_without_pending_heartbeat() {
        assert_eq!(
            hub_heartbeat_sweep_decision(100, 39, None, 60, 5),
            HubHeartbeatSweepDecision::SendRequest
        );
    }

    #[test]
    fn sweep_keeps_client_while_pending_heartbeat_is_within_ack_timeout() {
        assert_eq!(
            hub_heartbeat_sweep_decision(104, 30, pending(0x8000), 60, 5),
            HubHeartbeatSweepDecision::Keep
        );
    }

    #[test]
    fn sweep_removes_client_when_pending_heartbeat_exceeds_ack_timeout() {
        assert_eq!(
            hub_heartbeat_sweep_decision(106, 30, pending(0x8000), 60, 5),
            HubHeartbeatSweepDecision::RemoveTimedOut
        );
    }

    #[test]
    fn heartbeat_ack_must_match_pending_message_id() {
        assert!(heartbeat_ack_matches_pending(0x8000, pending(0x8000)));
        assert!(!heartbeat_ack_matches_pending(0x8001, pending(0x8000)));
        assert!(!heartbeat_ack_matches_pending(0x8000, None));
    }

    fn pending(message_id: u16) -> Option<PendingHeartbeat> {
        Some(PendingHeartbeat {
            message_id,
            published_at: 100,
        })
    }
}
