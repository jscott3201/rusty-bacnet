//! One owner for manual and automatic outgoing BVLC exchanges. All critical
//! sections are synchronous; the pending guard spans I/O without holding a lock.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bytes::BytesMut;
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::bvlc_response::{decode_bvlc_result_code, BvlcResponseKind, PendingBvlcResponse};
use super::client_snapshot::{increment, BvlcClientSnapshot, ForeignRegistrationOutcome};
use super::BipSocket;
use crate::bbmd::{self, BbmdState};
use crate::bvll::{decode_bip_mac, encode_bvll, BvllMessage};
use bacnet_types::enums::{BvlcFunction, BvlcResultCode};
use bacnet_types::error::Error;

#[derive(Default)]
struct State {
    pending: Option<PendingBvlcResponse>,
    snapshot: BvlcClientSnapshot,
    next_attempt: Option<Instant>,
}

#[derive(Default)]
pub(super) struct ManagementClient {
    state: Mutex<State>,
}

impl ManagementClient {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(super) fn snapshot(&self) -> BvlcClientSnapshot {
        let state = self.lock();
        let mut snapshot = state.snapshot;
        snapshot.foreign_registration.next_attempt_in = state
            .next_attempt
            .map(|when| when.saturating_duration_since(Instant::now()));
        snapshot
    }

    pub(super) fn start(&self) {
        let mut state = self.lock();
        state.pending = None;
        state.next_attempt = None;
        state.snapshot.reset_status();
        state.snapshot.running = true;
    }

    pub(super) fn stop(&self) {
        let mut state = self.lock();
        state.snapshot.running = false;
        state.pending = None;
        state.next_attempt = None;
        state.snapshot.reset_status();
    }

    pub(super) fn schedule(&self, next: Instant) {
        let mut state = self.lock();
        if state.snapshot.running {
            state.next_attempt = Some(next);
        }
    }

    pub(super) fn begin(
        self: &Arc<Self>,
        target: ([u8; 4], u16),
        function: BvlcFunction,
        expected: BvlcResponseKind,
        ttl: Option<u16>,
    ) -> Result<(RequestGuard, oneshot::Receiver<BvllMessage>), Error> {
        let mut state = self.lock();
        if !state.snapshot.running {
            return Err(Error::Encoding("B/IP management client is stopped".into()));
        }
        if state.pending.is_some() {
            increment(&mut state.snapshot.counters(function).busy);
            return Err(Error::Encoding(
                "BVLC management request already in flight".into(),
            ));
        }
        // Retaining this allocation in the guard prevents identity reuse even
        // after a response releases the slot and a newer request acquires it.
        let owner = Arc::new(());
        let (tx, rx) = oneshot::channel();
        state.pending = Some(PendingBvlcResponse {
            owner: owner.clone(),
            target,
            function,
            expected,
            ttl,
            deadline: None,
            early_response: None,
            tx,
        });
        if let Some(ttl) = ttl {
            let foreign = &mut state.snapshot.foreign_registration;
            increment(&mut foreign.attempts);
            foreign.last_bbmd = Some(SocketAddrV4::new(Ipv4Addr::from(target.0), target.1));
            foreign.last_ttl = Some(ttl);
            foreign.last_result = None;
            foreign.last_outcome = Some(ForeignRegistrationOutcome::Pending);
        }
        Ok((
            RequestGuard {
                client: self.clone(),
                owner,
                function,
            },
            rx,
        ))
    }

    /// Deliver only a peer- and operation-matched response. Malformed Results
    /// cannot steal a request; typed malformed ACKs preserve helper decode errors.
    pub(super) fn complete(&self, msg: &BvllMessage, sender: ([u8; 4], u16)) -> bool {
        let validity = match msg.function {
            BvlcFunction::BVLC_RESULT => decode_bvlc_result_code(msg).map(|_| ()),
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK => {
                BbmdState::decode_bdt(&msg.payload).map(|_| ())
            }
            BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK => {
                bbmd::decode_fdt(&msg.payload).map(|_| ())
            }
            _ => return false,
        };
        let mut state = self.lock();
        if !state.snapshot.running {
            return false;
        }
        if validity.is_err() {
            increment(&mut state.snapshot.malformed_responses);
            if let Some(function) = state
                .pending
                .as_ref()
                .filter(|pending| {
                    pending.target == sender && pending.expected.accepts(msg.function)
                })
                .map(|pending| pending.function)
            {
                increment(&mut state.snapshot.counters(function).malformed_responses);
            }
            if msg.function == BvlcFunction::BVLC_RESULT {
                return false;
            }
        }
        let matched = state.pending.as_ref().is_some_and(|pending| {
            pending.matches(sender, msg)
                && pending
                    .deadline
                    .is_none_or(|deadline| Instant::now() < deadline)
        });
        if !matched {
            if validity.is_ok() {
                increment(&mut state.snapshot.unmatched_responses);
                if let Ok(code) = decode_bvlc_result_code(msg) {
                    state.snapshot.last_unmatched_result = Some(code);
                }
            }
            return false;
        }
        let pending = state.pending.as_mut().expect("matched pending request");
        if pending.deadline.is_none() {
            // A fast receive worker can beat the sender's post-send accounting.
            // Keep one candidate, but count/deliver it only after local success.
            // A failed send or cancellation drops it with the owned slot.
            if pending.early_response.is_none() {
                pending.early_response = Some((msg.clone(), validity.is_ok()));
            }
            return true;
        }
        Self::deliver(&mut state, msg, validity.is_ok());
        true
    }

    fn deliver(state: &mut State, msg: &BvllMessage, valid_payload: bool) {
        let pending = state.pending.take().expect("matched pending request");
        if valid_payload {
            let counters = state.snapshot.counters(pending.function);
            if msg.function == BvlcFunction::BVLC_RESULT {
                let code = decode_bvlc_result_code(msg).expect("validated result");
                increment(&mut counters.results);
                counters.last_result = Some(code);
                if pending.ttl.is_some() {
                    let foreign = &mut state.snapshot.foreign_registration;
                    foreign.last_result = Some(code);
                    foreign.last_outcome = Some(if code == BvlcResultCode::SUCCESSFUL_COMPLETION {
                        increment(&mut foreign.accepted);
                        ForeignRegistrationOutcome::Accepted
                    } else {
                        increment(&mut foreign.refused);
                        ForeignRegistrationOutcome::Refused
                    });
                }
            } else {
                increment(&mut counters.acknowledgements);
            }
        }
        let _ = pending.tx.send(msg.clone());
    }

    pub(super) async fn request(
        self: &Arc<Self>,
        socket: &BipSocket,
        target: &[u8],
        function: BvlcFunction,
        expected: BvlcResponseKind,
        payload: &[u8],
        timeout: Duration,
    ) -> Result<BvllMessage, Error> {
        let (ip, port) = decode_bip_mac(target)?;
        let ttl = if function == BvlcFunction::REGISTER_FOREIGN_DEVICE {
            Some(u16::from_be_bytes(payload.try_into().map_err(|_| {
                Error::Encoding("Register-FD requires a two-byte TTL".into())
            })?))
        } else {
            None
        };
        let (guard, rx) = self.begin((ip, port), function, expected, ttl)?;
        let mut buf = BytesMut::with_capacity(4 + payload.len());
        if let Err(error) = encode_bvll(&mut buf, function, payload) {
            guard.fail(ForeignRegistrationOutcome::LocalError);
            return Err(error);
        }
        if let Err(error) = socket
            .send_to(&buf, SocketAddrV4::new(Ipv4Addr::from(ip), port))
            .await
        {
            guard.fail(ForeignRegistrationOutcome::LocalError);
            return Err(Error::Transport(error));
        }
        let deadline = guard.sent(timeout);
        match tokio::time::timeout_at(deadline, rx).await {
            Ok(Ok(msg)) => Ok(msg),
            Ok(Err(_)) => Err(Error::Encoding("BVLC response channel dropped".into())),
            Err(_) => {
                guard.fail(ForeignRegistrationOutcome::TimedOut);
                Err(Error::Timeout(timeout))
            }
        }
    }
}

pub(super) struct RequestGuard {
    client: Arc<ManagementClient>,
    owner: Arc<()>,
    function: BvlcFunction,
}

impl RequestGuard {
    /// Called exactly once after UDP send succeeds. Publish that observation
    /// and any early matched response together under the same short lock.
    pub(super) fn sent(&self, timeout: Duration) -> Instant {
        let deadline = Instant::now() + timeout;
        let mut state = self.client.lock();
        increment(&mut state.snapshot.counters(self.function).sent);
        let early = state
            .pending
            .as_mut()
            .filter(|p| Arc::ptr_eq(&p.owner, &self.owner))
            .and_then(|pending| {
                pending.deadline = Some(deadline);
                pending.early_response.take()
            });
        if let Some((msg, valid_payload)) = early {
            ManagementClient::deliver(&mut state, &msg, valid_payload);
        }
        deadline
    }

    fn fail(&self, outcome: ForeignRegistrationOutcome) {
        let mut state = self.client.lock();
        if !state
            .pending
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(&p.owner, &self.owner))
        {
            return;
        }
        let pending = state.pending.take().expect("owned pending request");
        let counters = state.snapshot.counters(self.function);
        match outcome {
            ForeignRegistrationOutcome::TimedOut => increment(&mut counters.timeouts),
            ForeignRegistrationOutcome::LocalError => increment(&mut counters.local_errors),
            _ => {}
        }
        if pending.ttl.is_some() {
            state.snapshot.foreign_registration.last_outcome = Some(outcome);
        }
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.fail(ForeignRegistrationOutcome::Cancelled);
    }
}
