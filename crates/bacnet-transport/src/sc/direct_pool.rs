//! Bounded outbound socket ownership, generation-aware reuse and retirement.
use super::WebSocketPort;
use super::direct_discovery::{DIRECT_POOL_IDLE_TTL, DIRECT_POOL_MAX_ENTRIES};
#[cfg(all(test, feature = "sc-tls"))]
use super::direct_egress::DirectSendError;
use super::direct_egress::DirectWrite;
use super::direct_membership::{Membership, bare_control, disconnect_request};
#[cfg(feature = "sc-tls")]
use super::direct_receive::direct_npdu;
use super::direct_receive::{DirectMuDecision, direct_must_understand_decision};
use super::direct_socket::{DirectFrame, DirectSocket};
use super::heartbeat;
#[cfg(feature = "sc-tls")]
use super::npdu_admission::DirectPeer;
use crate::sc_frame::{
    ControlRecipient, ScFunction, ScMessage, Vmac, decode_sc_message, encode_sc_message,
    validate_control,
};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::Ordering;
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;
use tokio::sync::{OwnedSemaphorePermit, mpsc};

#[cfg(feature = "sc-tls")]
#[derive(Clone)]
pub(crate) struct DirectIntake {
    pub(crate) tx: mpsc::Sender<crate::port::ReceivedNpdu>,
    pub(crate) admission: Arc<super::npdu_admission::ScNpduAdmission>,
}
#[derive(Clone)]
pub(crate) struct PooledDirect {
    pub(super) member: Arc<Membership>,
    idle_deadline: Arc<std::sync::Mutex<Instant>>,
}
struct Retire(Arc<Membership>);
impl Drop for Retire {
    fn drop(&mut self) {
        self.0.retire();
    }
}
impl PooledDirect {
    pub(super) fn start<W: WebSocketPort>(
        ws: DirectSocket<W>,
        member: Arc<Membership>,
        physical: OwnedSemaphorePermit,
        #[cfg(feature = "sc-tls")] intake: Option<DirectIntake>,
        local_limits: (u16, u16),
        heartbeat_ms: (u64, u64),
    ) -> (Self, tokio::task::JoinHandle<()>) {
        #[cfg(feature = "sc-tls")]
        let identity = ws
            .leaf()
            .map(|leaf| crate::port::DirectScIdentity::verified(leaf, member.generation));
        #[cfg(feature = "sc-tls")]
        let response = identity.map(|identity| crate::port::DirectResponse::new(&member, identity));
        #[cfg(feature = "sc-tls")]
        let provenance = identity.map(|identity| {
            crate::port::TransportProvenance::verified_direct_peer(identity, ws.leaf_certificate())
        });
        let wait = member.egress.wait;
        let mut recv = member.take_writes();
        let owner = member.clone();
        let retirement = Retire(owner.clone());
        let mut retired = owner.retirement();
        let idle_deadline = Arc::new(std::sync::Mutex::new(Instant::now() + DIRECT_POOL_IDLE_TTL));
        let activity = idle_deadline.clone();
        let task = tokio::spawn(async move {
            let (_physical, _retirement) = (physical, retirement);
            let mut prefer_send = true;
            // Two separate timers (#2127). The idle TTL reclaims a link nobody
            // uses: traffic in either direction refreshes it, heartbeats do
            // not, so probing alone cannot pin an unused socket. The worker
            // owns it on tokio's clock; the pool's copy only feeds `prune`.
            // Liveness is AB.6.3 keep-alive on the #2124 deadline: probe one
            // interval after the last BVLC received, give up one timeout after.
            let mut idle_deadline = tokio::time::Instant::now() + DIRECT_POOL_IDLE_TTL;
            let touch = |idle_deadline: &mut tokio::time::Instant| {
                *idle_deadline = tokio::time::Instant::now() + DIRECT_POOL_IDLE_TTL;
                *activity.lock().unwrap() = Instant::now() + DIRECT_POOL_IDLE_TTL;
            };
            let mut last_received = tokio::time::Instant::now();
            let mut pending_heartbeat = None;
            // Both armed for real at the top of the loop.
            let idle_sleep = tokio::time::sleep_until(idle_deadline);
            let heartbeat_sleep = tokio::time::sleep_until(last_received);
            tokio::pin!(idle_sleep, heartbeat_sleep);
            let locally_retired = loop {
                if idle_sleep.deadline() != idle_deadline {
                    idle_sleep.as_mut().reset(idle_deadline);
                }
                let heartbeat_deadline =
                    heartbeat::deadline(last_received, pending_heartbeat.is_some(), heartbeat_ms);
                if heartbeat_sleep.deadline() != heartbeat_deadline {
                    heartbeat_sleep.as_mut().reset(heartbeat_deadline);
                }
                let event = tokio::select! {
                    biased;
                    _ = async { if !*retired.borrow_and_update() { let _ = retired.changed().await; } } => break true,
                    _ = &mut idle_sleep => break true,
                    _ = &mut heartbeat_sleep => {
                        // An unanswered probe means the peer is gone: close
                        // without a Disconnect-Request it cannot answer.
                        if pending_heartbeat.is_some() {
                            break false;
                        }
                        let id = owner.egress.next_message.fetch_add(1, Ordering::Relaxed);
                        let request = bare_control(ScFunction::HeartbeatRequest, id);
                        // Retirement mid-write ends the worker the way a
                        // DirectWrite does.
                        let sent = tokio::select! {
                            biased;
                            _ = retired.changed() => break true,
                            sent = tokio::time::timeout(wait, ws.send(&request)) => sent,
                        };
                        if !matches!(sent, Ok(Ok(()))) {
                            break false;
                        }
                        pending_heartbeat = Some(id);
                        continue;
                    }
                    event = next_event(&ws, &mut recv, prefer_send) => event,
                };
                match event {
                    WorkerEvent::Send(Some(request)) => {
                        prefer_send = false;
                        if !request.can_start() {
                            continue;
                        }
                        if owner.with_current(|| request.mark_started()).is_none() {
                            break true;
                        }
                        let result = tokio::select! {
                            biased;
                            _ = retired.changed() => Err(crate::direct_response::unavailable()),
                            result = tokio::time::timeout(wait, ws.send(&request.bytes)) =>
                                result.unwrap_or_else(|_| Err(crate::direct_response::unavailable())),
                        };
                        let failed = result.is_err();
                        let _ = request.done.send(result);
                        if failed {
                            break true;
                        }
                        touch(&mut idle_deadline);
                    }
                    WorkerEvent::Send(None) => break true,
                    WorkerEvent::Received(received) => {
                        prefer_send = true;
                        let wire = match received {
                            Ok(DirectFrame::Binary(wire)) => wire,
                            #[cfg(feature = "sc-tls")]
                            Ok(DirectFrame::Control) => {
                                tokio::task::yield_now().await;
                                continue;
                            }
                            Err(_) => break false,
                        };
                        if wire.len() > usize::from(local_limits.0) {
                            continue;
                        }
                        let Ok(message) = decode_sc_message(&wire) else {
                            continue;
                        };
                        match message.function {
                            ScFunction::HeartbeatAck => {
                                // Only the ACK to the outstanding probe proves
                                // liveness; it never counts as use.
                                if heartbeat::ack_matches_outstanding(&message, pending_heartbeat) {
                                    last_received = tokio::time::Instant::now();
                                    pending_heartbeat = None;
                                }
                                continue;
                            }
                            // AB.6.3 assigns probing to the initiator, but a
                            // peer that probes anyway gets an answer. Like an
                            // ACK, it shows the peer is alive, not in use.
                            ScFunction::HeartbeatRequest => {
                                // A rejected probe is not liveness, as on the hub.
                                let reply = match validate_control(
                                    &message,
                                    &wire,
                                    ControlRecipient::HubConnector,
                                ) {
                                    Ok(()) => {
                                        last_received = tokio::time::Instant::now();
                                        pending_heartbeat = None;
                                        bare_control(ScFunction::HeartbeatAck, message.message_id)
                                            .freeze()
                                    }
                                    Err(Some(nak)) => nak,
                                    Err(None) => continue,
                                };
                                if !matches!(
                                    tokio::time::timeout(wait, ws.send(&reply)).await,
                                    Ok(Ok(()))
                                ) {
                                    break false;
                                }
                                continue;
                            }
                            // Both refresh below only once admitted (a valid
                            // Disconnect-Request ends the link anyway).
                            ScFunction::EncapsulatedNpdu | ScFunction::DisconnectRequest => {}
                            // Anything else, a BVLC-Result included, shows the
                            // peer is alive but is not use of the link, so a
                            // peer NAKing every probe cannot pin it open.
                            _ => {
                                last_received = tokio::time::Instant::now();
                                pending_heartbeat = None;
                            }
                        }
                        if message.function == ScFunction::EncapsulatedNpdu {
                            match direct_must_understand_decision(&message, &wire) {
                                DirectMuDecision::Drop => continue,
                                DirectMuDecision::Nak(nak) => {
                                    let mut bytes = bytes::BytesMut::new();
                                    encode_sc_message(&mut bytes, &nak);
                                    if !matches!(
                                        tokio::time::timeout(wait, ws.send(&bytes)).await,
                                        Ok(Ok(()))
                                    ) {
                                        break false;
                                    }
                                }
                                DirectMuDecision::Pass => {
                                    // As on the hub, only admitted traffic
                                    // counts; a dropped or NAKed NPDU doesn't.
                                    last_received = tokio::time::Instant::now();
                                    pending_heartbeat = None;
                                    touch(&mut idle_deadline);
                                    #[cfg(feature = "sc-tls")]
                                    if let (
                                        Some(intake),
                                        Some(provenance),
                                        Some(response),
                                        Some(address),
                                        Some(npdu),
                                    ) = (
                                        &intake,
                                        &provenance,
                                        &response,
                                        ws.peer_address(),
                                        direct_npdu(&message, local_limits.1),
                                    ) {
                                        owner.with_current(|| {
                                            intake.admission.admit_direct_peer(
                                                &intake.tx,
                                                &message,
                                                npdu,
                                                DirectPeer {
                                                    vmac: owner.vmac,
                                                    addr: address,
                                                    provenance: provenance.clone(),
                                                    response: Some(response.clone()),
                                                },
                                            )
                                        });
                                    }
                                }
                            }
                            continue;
                        }
                        if message.function != ScFunction::DisconnectRequest {
                            continue;
                        }
                        match validate_control(&message, &wire, ControlRecipient::HubConnector) {
                            Err(Some(nak)) => {
                                if !matches!(
                                    tokio::time::timeout(wait, ws.send(&nak)).await,
                                    Ok(Ok(()))
                                ) {
                                    break false;
                                }
                                continue;
                            }
                            Err(None) => continue,
                            Ok(()) => {}
                        }
                        owner.retire();
                        let ack = ScMessage {
                            function: ScFunction::DisconnectAck,
                            message_id: message.message_id,
                            originating_vmac: None,
                            destination_vmac: None,
                            dest_options: vec![],
                            data_options: vec![],
                            payload: bytes::Bytes::new(),
                        };
                        let mut bytes = bytes::BytesMut::new();
                        encode_sc_message(&mut bytes, &ack);
                        let _ = tokio::time::timeout(wait, ws.send(&bytes)).await;
                        break false;
                    }
                }
            };
            owner.retire();
            if locally_retired {
                let mut bytes = bytes::BytesMut::new();
                encode_sc_message(&mut bytes, &disconnect_request());
                let _ = tokio::time::timeout(wait, ws.send(&bytes)).await;
            }
        });
        (
            Self {
                member,
                idle_deadline,
            },
            task,
        )
    }
    #[cfg(all(test, feature = "sc-tls"))]
    pub(super) async fn send(&self, bytes: &[u8]) -> Result<(), DirectSendError> {
        self.member
            .egress
            .send_frame(bytes::Bytes::copy_from_slice(bytes))
            .await
    }
}
enum WorkerEvent {
    Received(Result<DirectFrame, bacnet_types::error::Error>),
    Send(Option<DirectWrite>),
}
// FIFO admission bounds ordinary/reply contention. Alternate one read frame
// against one bounded write; WebSocket controls are explicit read turns too.
async fn next_event<W: WebSocketPort>(
    ws: &DirectSocket<W>,
    recv: &mut mpsc::Receiver<DirectWrite>,
    prefer_send: bool,
) -> WorkerEvent {
    if prefer_send {
        tokio::select! { biased; request = recv.recv() => WorkerEvent::Send(request), received = ws.next_frame() => WorkerEvent::Received(received) }
    } else {
        tokio::select! { biased; received = ws.next_frame() => WorkerEvent::Received(received), request = recv.recv() => WorkerEvent::Send(request) }
    }
}
pub(crate) struct DirectPool {
    entries: HashMap<Vmac, PooledDirect>,
    order: VecDeque<Vmac>,
}
impl DirectPool {
    pub(crate) fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    #[cfg(test)]
    pub(crate) fn get(&mut self, vmac: &Vmac, now: Instant) -> Option<PooledDirect> {
        match self.entries.get(vmac) {
            Some(e) if now < *e.idle_deadline.lock().unwrap() && e.member.is_current() => {
                Some(e.clone())
            }
            _ => {
                self.remove(vmac);
                None
            }
        }
    }
    pub(super) fn prune(&mut self, now: Instant) {
        let expired: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, e)| now >= *e.idle_deadline.lock().unwrap() || !e.member.is_current())
            .map(|(v, _)| *v)
            .collect();
        for vmac in expired {
            self.remove(&vmac);
        }
    }
    pub(crate) fn insert(&mut self, vmac: Vmac, pooled: PooledDirect) {
        self.remove(&vmac);
        self.order.push_back(vmac);
        self.entries.insert(vmac, pooled);
        while self.entries.len() > DIRECT_POOL_MAX_ENTRIES {
            self.evict_oldest();
        }
    }
    pub(super) fn clear(&mut self) {
        for entry in self.entries.values() {
            entry.member.retire();
        }
        self.entries.clear();
        self.order.clear();
    }
    pub(super) fn evict_oldest(&mut self) {
        if let Some(vmac) = self.order.front().copied() {
            self.remove(&vmac);
        }
    }
    fn remove(&mut self, vmac: &Vmac) {
        if let Some(entry) = self.entries.remove(vmac) {
            entry.member.retire();
        }
        self.order.retain(|v| v != vmac);
    }
    #[cfg(test)]
    pub(crate) fn insert_test_entry<W>(
        &mut self,
        vmac: Vmac,
        _ws: Arc<W>,
        _uri: String,
        now: Instant,
    ) {
        use super::direct_membership::{DirectMembership, DirectRole};
        let owner = Arc::new(DirectMembership::default());
        let member = owner
            .reserve([1; 16], vmac, [2; 16], [0xff; 6], DirectRole::Outbound, 16)
            .unwrap()
            .commit();
        self.insert(
            vmac,
            PooledDirect {
                member,
                idle_deadline: Arc::new(std::sync::Mutex::new(now + DIRECT_POOL_IDLE_TTL)),
            },
        );
    }
}
impl Drop for DirectPool {
    fn drop(&mut self) {
        for entry in self.entries.values() {
            entry.member.retire();
        }
    }
}

#[cfg(test)]
#[path = "direct_worker_tests.rs"]
mod direct_worker_tests;

#[cfg(test)]
#[path = "direct_heartbeat_tests.rs"]
mod direct_heartbeat_tests;
