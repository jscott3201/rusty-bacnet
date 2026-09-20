use std::collections::HashMap;
use std::fmt;
use std::future::{poll_fn, Future};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

use bacnet_encoding::apdu::Apdu;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_endpoint_core::coordinator::{
    Admission, AdmissionKind, AdmissionOutcome, CanonicalPeer, LeaseMetadata, LeaseOwner,
    LeaseToken, OutboundTransactionCoordinator, ReserveError,
};
use bacnet_objects::audit::AuditReporterStatus;
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier};
use tokio::sync::oneshot;
use tokio::task::{JoinError, JoinSet};
use tokio::time::Duration;

use super::event_recipient_route::ConfirmedRecipientRoute;
use super::CovAckResult;

#[cfg(test)]
#[path = "notification_worker_owner_tests.rs"]
mod notification_worker_owner_tests;

#[derive(Debug)]
#[doc(hidden)]
pub enum NotificationReserveError {
    Closed,
    Coordinator(ReserveError),
    StatePoisoned,
}

impl fmt::Display for NotificationReserveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("notification transaction adapter is closed"),
            Self::Coordinator(error) => error.fmt(formatter),
            Self::StatePoisoned => {
                formatter.write_str("notification transaction state is poisoned")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub enum NotificationWorkerResult {
    Ack,
    Error,
    Exhausted,
    Closed,
}

struct NotificationState {
    closed: bool,
    pending: HashMap<LeaseToken, oneshot::Sender<CovAckResult>>,
}

#[doc(hidden)]
pub struct NotificationTransactions {
    core: Arc<NotificationCore>,
    workers: Mutex<NotificationWorkers>,
    audit_permits: Arc<tokio::sync::Semaphore>,
    audit_failures: Arc<Mutex<AuditFailures>>,
}

#[derive(Default)]
struct NotificationWorkers {
    closed: bool,
    tasks: JoinSet<()>,
    waiter: Option<Waker>,
}

// Operations retain transaction state, never the owner of their JoinSet.
struct NotificationCore {
    coordinator: Arc<OutboundTransactionCoordinator>,
    state: Mutex<NotificationState>,
    // RB-21 producers run only in BACnetServer, which creates a private pool
    // via new(): every capacity release passes through this adapter. The
    // shared endpoint has no audit producer; extending it requires a
    // coordinator-wide release signal, not this adapter-local notification.
    released: tokio::sync::Notify,
}

pub(super) type NotificationReservation = (NotificationOperation, oneshot::Receiver<CovAckResult>);

/// One server-wide coalesced batch, never an ordinary-record queue.
pub(super) struct AuditFailureBatch {
    pub(super) count: u64,
    pub(super) earliest: BACnetTimeStamp,
    pub(super) device: ObjectIdentifier,
    pub(super) status: Arc<AuditReporterStatus>,
    pub(super) epoch: u64,
    pub(super) confirmed: bool,
    pub(super) route: Arc<ConfirmedRecipientRoute>,
    pub(super) max_apdu: u32,
}

impl AuditFailureBatch {
    pub(super) fn notification(&self) -> bacnet_types::constructed::BACnetAuditNotification {
        use bacnet_types::constructed::{BACnetAuditNotification, BACnetRecipient};
        let mut value = bytes::BytesMut::new();
        bacnet_encoding::primitives::encode_app_unsigned(&mut value, self.count);
        BACnetAuditNotification {
            source_timestamp: None,
            target_timestamp: Some(self.earliest.clone()),
            source_device: BACnetRecipient::Device(self.device),
            source_object: None,
            operation: bacnet_types::enums::AuditOperation::AUDITING_FAILURE,
            source_comment: None,
            target_comment: None,
            invoke_id: None,
            source_user_id: None,
            source_user_role: None,
            target_device: BACnetRecipient::Device(self.device),
            target_object: None,
            target_property: None,
            target_priority: None,
            target_value: None,
            current_value: Some(value.to_vec()),
            result: None,
        }
    }

    fn enabled(&self) -> bool {
        self.status.auditing_failure_epoch() == Some(self.epoch)
    }
}

#[derive(Default)]
struct AuditFailures {
    owned: bool,
    pending: Option<AuditFailureBatch>,
}

/// Retains core/permit/state handles, never the JoinSet owner. Drop also covers
/// rejected spawn, cancellation and panic; only this owner may drain batches.
pub(super) struct AuditFailureWorker {
    core: Arc<NotificationCore>,
    permits: Arc<tokio::sync::Semaphore>,
    failures: Arc<Mutex<AuditFailures>>,
    active: bool,
}

impl AuditFailureWorker {
    fn finish_if_empty(&mut self) -> bool {
        let mut state = self.failures.lock().unwrap();
        if state.pending.as_ref().is_some_and(|batch| !batch.enabled()) {
            state.pending = None;
        }
        if state.pending.is_none() {
            state.owned = false;
            self.active = false;
            true
        } else {
            false
        }
    }

    pub(super) async fn next(
        &mut self,
    ) -> Option<(
        AuditFailureBatch,
        tokio::sync::OwnedSemaphorePermit,
        Option<NotificationReservation>,
    )> {
        if self.finish_if_empty() {
            return None;
        }
        let permit = Arc::clone(&self.permits).acquire_owned().await.ok()?;
        let core = Arc::clone(&self.core);
        loop {
            // Register before checking capacity: releases between the failed
            // reservation and await cannot be lost. No polling or timed retries.
            let released = core.released.notified();
            tokio::pin!(released);
            released.as_mut().enable();
            let ready = {
                let mut state = self.failures.lock().unwrap();
                let batch = state.pending.as_ref()?;
                if !batch.enabled() {
                    state.pending = None;
                    None
                } else {
                    let reserved = if batch.confirmed {
                        self.core
                            .reserve(
                                batch.route.canonical_peer.clone(),
                                ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                            )
                            .map(Some)
                    } else {
                        Ok(None)
                    };
                    match reserved {
                        Ok(reserved) => Some((state.pending.take().unwrap(), reserved)),
                        Err(NotificationReserveError::Coordinator(ReserveError::Exhausted)) => None,
                        Err(_) => {
                            let batch = state.pending.take().unwrap();
                            batch
                                .status
                                .complete_delivery(batch.status.begin_delivery(), false);
                            None
                        }
                    }
                }
            };
            if let Some((batch, reserved)) = ready {
                return Some((batch, permit, reserved));
            }
            if self.finish_if_empty() {
                return None;
            }
            released.await;
        }
    }
}

impl Drop for AuditFailureWorker {
    fn drop(&mut self) {
        if self.active {
            let mut state = self.failures.lock().unwrap();
            state.pending = None;
            state.owned = false;
        }
    }
}

impl NotificationTransactions {
    pub(super) fn record_audit_drop(
        &self,
        mut batch: AuditFailureBatch,
    ) -> Option<AuditFailureWorker> {
        if !batch.enabled() || self.audit_permits.is_closed() {
            return None;
        }
        let mut state = self.audit_failures.lock().unwrap();
        let changed =
            if let Some(mut previous) = state.pending.take().filter(AuditFailureBatch::enabled) {
                previous.count = previous.count.saturating_add(batch.count);
                batch = previous;
                false
            } else {
                true
            };
        // Keep the earliest record's delivery context, including its instance.
        // Aggregation must not transfer an older pending summary's completion
        // authority to a replacement Reporter at the same object identifier.
        state.pending = Some(batch);
        if state.owned {
            if changed {
                self.core.released.notify_waiters();
            }
            return None;
        }
        state.owned = true;
        Some(AuditFailureWorker {
            core: Arc::clone(&self.core),
            permits: Arc::clone(&self.audit_permits),
            failures: Arc::clone(&self.audit_failures),
            active: true,
        })
    }

    pub(super) fn try_admit_audit(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        Arc::clone(&self.audit_permits).try_acquire_owned().ok()
    }

    #[doc(hidden)]
    pub fn new() -> Arc<Self> {
        Self::with_coordinator(Arc::new(OutboundTransactionCoordinator::new()))
    }

    #[doc(hidden)]
    pub fn with_coordinator(coordinator: Arc<OutboundTransactionCoordinator>) -> Arc<Self> {
        Arc::new(Self {
            core: Arc::new(NotificationCore {
                coordinator,
                state: Mutex::new(NotificationState {
                    closed: false,
                    pending: HashMap::new(),
                }),
                released: tokio::sync::Notify::new(),
            }),
            workers: Mutex::new(NotificationWorkers::default()),
            audit_permits: Arc::new(tokio::sync::Semaphore::new(64)),
            audit_failures: Arc::new(Mutex::new(AuditFailures::default())),
        })
    }

    #[doc(hidden)]
    pub fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        let mut workers = self.workers.lock().unwrap();
        if workers.closed {
            // A rejected future may own an operation and resource guards.
            drop(workers);
            drop(task);
            return;
        }
        workers.tasks.spawn(task);
        let waiter = workers.waiter.take();
        drop(workers);
        if let Some(waiter) = waiter {
            waiter.wake();
        }
    }

    #[doc(hidden)]
    pub fn close(&self) {
        self.audit_permits.close();
        let mut workers = self.workers.lock().unwrap();
        // Serialize worker registration with transaction sealing. Reservation
        // uses only the core; no future can bypass closed worker admission.
        self.core.close();
        workers.closed = true;
        workers.tasks.abort_all();
        let waiter = workers.waiter.take();
        drop(workers);
        if let Some(waiter) = waiter {
            waiter.wake();
        }
    }

    /// Dispatch is the sole consumer until joined by stop. An empty open set
    /// waits for producer admission, including when ingress is idle. Cancelling
    /// this future retains every outstanding join in the owner.
    #[doc(hidden)]
    pub async fn join_next(&self) -> Option<Result<(), JoinError>> {
        poll_fn(|cx| {
            let mut workers = self.workers.lock().unwrap();
            match workers.tasks.poll_join_next(cx) {
                Poll::Ready(None) if !workers.closed => {
                    workers.waiter = Some(cx.waker().clone());
                    Poll::Pending
                }
                result => result,
            }
        })
        .await
    }

    #[doc(hidden)]
    pub fn observe(result: Option<Result<(), JoinError>>) {
        if let Some(Err(error)) = result {
            if !error.is_cancelled() {
                tracing::warn!(%error, "Confirmed notification worker failed");
            }
        }
    }

    #[doc(hidden)]
    pub fn reserve(
        &self,
        peer: CanonicalPeer,
        service_choice: ConfirmedServiceChoice,
    ) -> Result<(NotificationOperation, oneshot::Receiver<CovAckResult>), NotificationReserveError>
    {
        self.core.reserve(peer, service_choice)
    }

    #[doc(hidden)]
    pub fn admit_terminal(
        &self,
        immediate_source: &[u8],
        routed_source: Option<&NpduAddress>,
        apdu: &Apdu,
    ) -> bool {
        self.core
            .admit_terminal(immediate_source, routed_source, apdu)
    }

    /// Completes one already-admitted terminal (session dispatch only).
    ///
    /// The shared-coordinator `admit` owns exact-once claim; this releases
    /// the exact lease without re-admitting.
    #[doc(hidden)]
    pub fn complete_pre_admitted(&self, admission: Admission, apdu: &Apdu) -> bool {
        self.core.complete_pre_admitted(admission, apdu)
    }

    #[cfg(test)]
    pub(super) fn workers_empty(&self) -> bool {
        self.workers.lock().unwrap().tasks.is_empty()
    }

    #[cfg(test)]
    pub(super) fn audit_resources(&self) -> (bool, u64, usize) {
        let state = self.audit_failures.lock().unwrap();
        (
            state.owned,
            state.pending.as_ref().map_or(0, |batch| batch.count),
            self.audit_permits.available_permits(),
        )
    }

    #[cfg(test)]
    pub(super) fn active_count(&self) -> usize {
        self.core.coordinator.active_count().unwrap_or(usize::MAX)
    }

    #[cfg(test)]
    pub(super) fn is_closed(&self) -> bool {
        self.core
            .state
            .lock()
            .map(|state| state.closed)
            .unwrap_or(true)
    }

    #[cfg(test)]
    pub(super) fn release_token_for_test(&self, token: LeaseToken) {
        self.core.release(token);
    }
}

impl NotificationCore {
    pub(super) fn reserve(
        self: &Arc<Self>,
        peer: CanonicalPeer,
        service_choice: ConfirmedServiceChoice,
    ) -> Result<(NotificationOperation, oneshot::Receiver<CovAckResult>), NotificationReserveError>
    {
        let token = self
            .coordinator
            .reserve(LeaseMetadata::server_notification(peer, service_choice))
            .map_err(NotificationReserveError::Coordinator)?;
        let (sender, receiver) = oneshot::channel();

        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => {
                let _ = self.coordinator.cancel(token);
                self.released.notify_waiters();
                return Err(NotificationReserveError::StatePoisoned);
            }
        };
        if state.closed {
            drop(state);
            let _ = self.coordinator.cancel(token);
            self.released.notify_waiters();
            return Err(NotificationReserveError::Closed);
        }
        state.pending.insert(token, sender);
        drop(state);

        Ok((
            NotificationOperation {
                transactions: Arc::clone(self),
                token,
                active: true,
            },
            receiver,
        ))
    }

    pub(super) fn admit_terminal(
        &self,
        immediate_source: &[u8],
        routed_source: Option<&NpduAddress>,
        apdu: &Apdu,
    ) -> bool {
        let peer = canonical_inbound_peer(immediate_source, routed_source);
        let admission = match self.coordinator.admit(&peer, apdu) {
            Ok(AdmissionOutcome::Admitted(admission))
                if admission.kind() == AdmissionKind::Terminal =>
            {
                admission
            }
            Ok(_) | Err(_) => return false,
        };

        self.complete_pre_admitted(admission, apdu)
    }

    pub(super) fn complete_pre_admitted(&self, admission: Admission, apdu: &Apdu) -> bool {
        if admission.kind() != AdmissionKind::Terminal
            || admission.metadata().owner() != LeaseOwner::ServerNotification
        {
            return false;
        }
        let token = admission.token();
        let result = match apdu {
            Apdu::SimpleAck(pdu)
                if pdu.invoke_id == token.invoke_id()
                    && pdu.service_choice == admission.metadata().service_choice() =>
            {
                CovAckResult::Ack
            }
            Apdu::Error(pdu) if pdu.invoke_id == token.invoke_id() => CovAckResult::Error,
            Apdu::Reject(pdu) if pdu.invoke_id == token.invoke_id() => CovAckResult::Error,
            Apdu::Abort(pdu) if pdu.invoke_id == token.invoke_id() => CovAckResult::Error,
            _ => return false,
        };
        let sender = match self.state.lock() {
            Ok(mut state) => state.pending.remove(&token),
            Err(_) => None,
        };
        let Some(sender) = sender else {
            return false;
        };

        let _ = self.coordinator.complete(token);
        self.released.notify_waiters();
        let _ = sender.send(result);
        true
    }

    pub(super) fn close(&self) {
        let pending = match self.state.lock() {
            Ok(mut state) => {
                if state.closed {
                    return;
                }
                state.closed = true;
                state.pending.drain().collect::<Vec<_>>()
            }
            Err(_) => return,
        };

        for (token, sender) in pending {
            drop(sender);
            let _ = self.coordinator.cancel(token);
        }
        self.released.notify_waiters();
    }

    fn rearm(
        &self,
        token: LeaseToken,
    ) -> Result<oneshot::Receiver<CovAckResult>, NotificationReserveError> {
        let (sender, receiver) = oneshot::channel();
        let mut state = self
            .state
            .lock()
            .map_err(|_| NotificationReserveError::StatePoisoned)?;
        if state.closed {
            return Err(NotificationReserveError::Closed);
        }
        let Some(pending) = state.pending.get_mut(&token) else {
            return Err(NotificationReserveError::Closed);
        };
        *pending = sender;
        Ok(receiver)
    }

    fn release(&self, token: LeaseToken) {
        if let Ok(mut state) = self.state.lock() {
            state.pending.remove(&token);
        }
        let _ = self.coordinator.release(token);
        self.released.notify_waiters();
    }

    fn cancel(&self, token: LeaseToken) {
        if let Ok(mut state) = self.state.lock() {
            state.pending.remove(&token);
        }
        let _ = self.coordinator.cancel(token);
        self.released.notify_waiters();
    }
}

impl Drop for NotificationTransactions {
    fn drop(&mut self) {
        self.close();
    }
}

#[doc(hidden)]
pub struct NotificationOperation {
    transactions: Arc<NotificationCore>,
    token: LeaseToken,
    active: bool,
}

impl NotificationOperation {
    #[doc(hidden)]
    pub fn invoke_id(&self) -> u8 {
        self.token.invoke_id()
    }

    fn rearm(&self) -> Result<oneshot::Receiver<CovAckResult>, NotificationReserveError> {
        self.transactions.rearm(self.token)
    }

    fn terminal_completed(&mut self) {
        self.active = false;
    }

    fn release(&mut self) {
        if self.active {
            self.transactions.release(self.token);
            self.active = false;
        }
    }

    fn cancel(&mut self) {
        if self.active {
            self.transactions.cancel(self.token);
            self.active = false;
        }
    }

    #[cfg(test)]
    pub(super) fn token(&self) -> LeaseToken {
        self.token
    }
}

impl Drop for NotificationOperation {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[doc(hidden)]
pub async fn run_notification_worker<F, Fut, E>(
    mut operation: NotificationOperation,
    mut receiver: oneshot::Receiver<CovAckResult>,
    timeout: Duration,
    max_retries: u8,
    mut send: F,
) -> NotificationWorkerResult
where
    F: FnMut(u8) -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    for attempt in 0..=max_retries {
        let send_failed = send(attempt).await.is_err();
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(CovAckResult::Ack)) => {
                operation.terminal_completed();
                return NotificationWorkerResult::Ack;
            }
            Ok(Ok(CovAckResult::Error)) => {
                operation.terminal_completed();
                return NotificationWorkerResult::Error;
            }
            Ok(Err(_)) | Err(_) if attempt < max_retries => match operation.rearm() {
                Ok(next_receiver) => receiver = next_receiver,
                Err(_) => {
                    operation.cancel();
                    return NotificationWorkerResult::Closed;
                }
            },
            Ok(Err(_)) => {
                operation.cancel();
                return NotificationWorkerResult::Closed;
            }
            Err(_) => {
                if send_failed {
                    operation.cancel();
                } else {
                    operation.release();
                }
                return NotificationWorkerResult::Exhausted;
            }
        }
    }

    operation.release();
    NotificationWorkerResult::Exhausted
}

#[doc(hidden)]
pub fn canonical_direct_peer(mac: &[u8]) -> CanonicalPeer {
    CanonicalPeer::direct(mac)
}

#[doc(hidden)]
pub fn canonical_routed_peer(network: u16, address: &[u8]) -> CanonicalPeer {
    CanonicalPeer::routed(network, address)
}

fn canonical_inbound_peer(
    immediate_source: &[u8],
    routed_source: Option<&NpduAddress>,
) -> CanonicalPeer {
    match routed_source {
        Some(source) if !source.mac_address.is_empty() => {
            canonical_routed_peer(source.network, &source.mac_address)
        }
        _ => canonical_direct_peer(immediate_source),
    }
}
