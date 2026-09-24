//! One bounded coalescer shared by source and target callers, with private routes.
use super::*;

#[doc(hidden)]
pub struct AuditFailureContext<R> {
    pub status: Arc<AuditReporterStatus>,
    pub epoch: u64,
    pub device: ObjectIdentifier,
    pub confirmed: bool,
    pub peer: CanonicalPeer,
    pub route: R,
    pub max_apdu: u32,
}

impl<R: PartialEq> AuditFailureContext<R> {
    pub fn enabled(&self) -> bool {
        self.status.auditing_failure_epoch() == Some(self.epoch)
    }

    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.status, &other.status)
            && self.epoch == other.epoch
            && self.device == other.device
            && self.confirmed == other.confirmed
            && self.peer == other.peer
            && self.route == other.route
            && self.max_apdu == other.max_apdu
    }
}

#[doc(hidden)]
#[derive(Clone)]
pub struct AuditFailureTicket<R> {
    pub context: Arc<AuditFailureContext<R>>,
    order: u64,
}

#[doc(hidden)]
pub struct AuditFailureBatch<R> {
    pub context: Arc<AuditFailureContext<R>>,
    pub count: u64,
    pub earliest: BACnetTimeStamp,
    earliest_order: u64,
}

impl<R> AuditFailureBatch<R> {
    pub fn notification(&self) -> bacnet_types::constructed::BACnetAuditNotification {
        self.context.notification(self.count, self.earliest.clone())
    }
}
impl<R> AuditFailureContext<R> {
    pub(in crate::server) fn notification(
        &self,
        count: u64,
        timestamp: BACnetTimeStamp,
    ) -> bacnet_types::constructed::BACnetAuditNotification {
        use bacnet_types::constructed::{BACnetAuditNotification, BACnetRecipient};
        let mut value = bytes::BytesMut::new();
        bacnet_encoding::primitives::encode_app_unsigned(&mut value, count);
        BACnetAuditNotification {
            source_timestamp: None,
            target_timestamp: Some(timestamp),
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
}

struct Entry<R> {
    epoch: u64,
    eligible: bool,
    context: Option<Arc<AuditFailureContext<R>>>,
    pending: Option<AuditFailureBatch<R>>,
    _slot: Option<tokio::sync::OwnedSemaphorePermit>,
}

struct State<R> {
    owned: bool,
    current: Option<u64>,
    order: u64,
    entries: Vec<Entry<R>>,
    reserved: usize,
    historical: Option<Arc<tokio::sync::Semaphore>>,
    lost: u64,
}
impl<R> State<R> {
    fn prune(&mut self) {
        let current = self.current;
        self.entries.retain(|entry| {
            Some(entry.epoch) == current
                || entry.pending.is_some()
                || entry
                    .context
                    .as_ref()
                    .is_some_and(|context| Arc::strong_count(context) > 1)
        });
    }
    fn eligible(&self, context: &Arc<AuditFailureContext<R>>) -> bool
    where
        R: PartialEq,
    {
        self.entries.iter().any(|entry| {
            entry
                .context
                .as_ref()
                .is_some_and(|c| Arc::ptr_eq(c, context))
                && entry.eligible
                && (self.historical.is_some() || context.enabled())
        })
    }
}

#[doc(hidden)]
pub struct AuditFailureQueue<R> {
    state: Arc<Mutex<State<R>>>,
    changed: Arc<tokio::sync::Notify>,
}
impl<R> Clone for AuditFailureQueue<R> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            changed: Arc::clone(&self.changed),
        }
    }
}
impl<R> Default for AuditFailureQueue<R> {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                owned: false,
                current: None,
                order: 0,
                entries: vec![],
                reserved: 0,
                historical: None,
                lost: 0,
            })),
            changed: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

/// A rollback-owned slot. Publishing after canonical commit cannot allocate or fail.
pub(in crate::server) struct ContextReservation<R> {
    queue: AuditFailureQueue<R>,
    entry: Option<Entry<R>>,
}
impl<R> ContextReservation<R> {
    pub(in crate::server) fn publish(mut self) {
        let mut state = self.queue.state.lock().unwrap();
        let entry = self.entry.take().unwrap();
        state.reserved -= 1;
        state.current = Some(entry.epoch);
        state.entries.push(entry);
        state.prune();
        drop(state);
        self.queue.changed.notify_waiters();
    }
}
impl<R> Drop for ContextReservation<R> {
    fn drop(&mut self) {
        if self.entry.is_some() {
            self.queue.state.lock().unwrap().reserved -= 1;
        }
    }
}

impl<R> AuditFailureQueue<R> {
    pub(super) fn target(budget: Arc<tokio::sync::Semaphore>, epoch: u64, eligible: bool) -> Self {
        let queue = Self::default();
        queue.state.lock().unwrap().historical = Some(budget);
        queue
            .prepare_context(epoch, eligible)
            .expect("validated at most 64 baseline contexts")
            .publish();
        queue
    }
    pub(in crate::server) fn prepare_context(
        &self,
        epoch: u64,
        eligible: bool,
    ) -> Result<ContextReservation<R>, Error> {
        let mut state = self.state.lock().unwrap();
        state.prune();
        let denied = || Error::Protocol {
            class: bacnet_types::enums::ErrorClass::SERVICES.to_raw() as u32,
            code: bacnet_types::enums::ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
        };
        if state.entries.len() + state.reserved >= 8
            || state.entries.iter().any(|entry| entry.epoch == epoch)
        {
            return Err(denied());
        }
        let slot = Arc::clone(state.historical.as_ref().expect("target context owner"))
            .try_acquire_owned()
            .map_err(|_| denied())?;
        // Capacity is allocated during fallible preparation, never after canonical assignment.
        let reserve = state.reserved + 1;
        state.entries.try_reserve(reserve).map_err(|_| denied())?;
        state.reserved += 1;
        Ok(ContextReservation {
            queue: self.clone(),
            entry: Some(Entry {
                epoch,
                eligible,
                context: None,
                pending: None,
                _slot: Some(slot),
            }),
        })
    }
    pub(super) fn has_pending(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.owned || state.entries.iter().any(|e| e.pending.is_some())
    }
    #[cfg(test)]
    pub(in crate::server) fn context_resources(&self) -> (usize, usize, u64) {
        let state = self.state.lock().unwrap();
        (state.entries.len(), state.reserved, state.lost)
    }
    #[cfg(test)]
    pub(super) fn resources(&self) -> (bool, u64) {
        let state = self.state.lock().unwrap();
        (
            state.owned,
            state
                .entries
                .iter()
                .filter_map(|e| e.pending.as_ref())
                .fold(0u64, |n, b| n.saturating_add(b.count)),
        )
    }
}
impl<R: PartialEq> AuditFailureQueue<R> {
    /// Current-only source profiles retire old contexts; target contexts remain pinned.
    pub fn recipient_changed(&self) {
        let mut state = self.state.lock().unwrap();
        if state.historical.is_none() {
            state.entries.clear();
            state.current = None;
        }
        drop(state);
        self.changed.notify_waiters();
    }
    /// Capture original admission order before queueing or any subsequent drop callback.
    pub fn observe(&self, context: AuditFailureContext<R>) -> Option<AuditFailureTicket<R>> {
        let mut state = self.state.lock().unwrap();
        if state.historical.is_none() {
            if !context.enabled() {
                return None;
            }
            let changed = !state
                .entries
                .first()
                .and_then(|e| e.context.as_ref())
                .is_some_and(|old| old.same(&context));
            if changed {
                state.entries.clear();
                state.current = Some(context.epoch);
                state.entries.push(Entry {
                    epoch: context.epoch,
                    eligible: true,
                    context: None,
                    pending: None,
                    _slot: None,
                });
                self.changed.notify_waiters();
            }
        }
        let order = state.order.checked_add(1)?;
        let entry = state
            .entries
            .iter_mut()
            .find(|entry| entry.epoch == context.epoch)?;
        let context = Arc::clone(entry.context.get_or_insert_with(|| Arc::new(context)));
        state.order = order;
        Some(AuditFailureTicket { context, order })
    }
    pub fn record_drop(
        &self,
        owner: &NotificationTransactions,
        ticket: AuditFailureTicket<R>,
        timestamp: BACnetTimeStamp,
        count: u64,
    ) -> Option<AuditFailureWorker<R>> {
        let mut state = self.state.lock().unwrap();
        state.lost = state.lost.saturating_add(count);
        if owner.audit_permits.is_closed() || !state.eligible(&ticket.context) {
            return None;
        }
        let entry = state.entries.iter_mut().find(|e| {
            e.context
                .as_ref()
                .is_some_and(|c| Arc::ptr_eq(c, &ticket.context))
        })?;
        merge(
            &mut entry.pending,
            AuditFailureBatch {
                context: ticket.context,
                count,
                earliest: timestamp,
                earliest_order: ticket.order,
            },
        );
        if state.owned {
            return None;
        }
        state.owned = true;
        Some(AuditFailureWorker {
            core: Arc::clone(&owner.core),
            permits: Arc::clone(&owner.audit_permits),
            failures: Arc::clone(&self.state),
            changed: Arc::clone(&self.changed),
            active: true,
        })
    }
}
fn merge<R>(pending: &mut Option<AuditFailureBatch<R>>, batch: AuditFailureBatch<R>) {
    if let Some(pending) = pending {
        pending.count = pending.count.saturating_add(batch.count);
        if batch.earliest_order < pending.earliest_order {
            pending.earliest_order = batch.earliest_order;
            pending.earliest = batch.earliest;
        }
    } else {
        *pending = Some(batch);
    }
}

#[doc(hidden)]
pub struct AuditFailureWorker<R> {
    core: Arc<NotificationCore>,
    permits: Arc<tokio::sync::Semaphore>,
    failures: Arc<Mutex<State<R>>>,
    changed: Arc<tokio::sync::Notify>,
    active: bool,
}
impl<R: PartialEq> AuditFailureWorker<R> {
    fn finish_if_empty(&mut self) -> bool {
        let mut state = self.failures.lock().unwrap();
        if state.historical.is_none() {
            for entry in &mut state.entries {
                if entry.pending.as_ref().is_some_and(|b| !b.context.enabled()) {
                    entry.pending = None;
                }
            }
        }
        state.prune();
        if state.entries.iter().all(|e| e.pending.is_none()) {
            state.owned = false;
            self.active = false;
            true
        } else {
            false
        }
    }
    /// Restore only the captured eligible context; never recursively count a summary.
    pub fn restore(&mut self, batch: AuditFailureBatch<R>) {
        let mut state = self.failures.lock().unwrap();
        if !state.eligible(&batch.context) {
            return;
        }
        if let Some(entry) = state.entries.iter_mut().find(|e| {
            e.context
                .as_ref()
                .is_some_and(|c| Arc::ptr_eq(c, &batch.context))
        }) {
            merge(&mut entry.pending, batch);
        }
    }
    pub async fn next(
        &mut self,
    ) -> Option<(
        AuditFailureBatch<R>,
        tokio::sync::OwnedSemaphorePermit,
        Option<NotificationReservation>,
    )> {
        if self.finish_if_empty() {
            return None;
        }
        let permit = Arc::clone(&self.permits).acquire_owned().await.ok()?;
        let coordinator = Arc::clone(&self.core.coordinator);
        let changed_owner = Arc::clone(&self.changed);
        loop {
            let released = coordinator.released();
            let changed = changed_owner.notified();
            tokio::pin!(released, changed);
            released.as_mut().enable();
            changed.as_mut().enable();
            let ready = {
                let mut state = self.failures.lock().unwrap();
                let historical = state.historical.is_some();
                let Some(entry) = state.entries.iter_mut().find(|e| e.pending.is_some()) else {
                    state.owned = false;
                    self.active = false;
                    return None;
                };
                let batch = entry.pending.as_ref().unwrap();
                if !historical && !batch.context.enabled() {
                    entry.pending = None;
                    None
                } else {
                    let reserved = if batch.context.confirmed {
                        self.core
                            .reserve(
                                batch.context.peer.clone(),
                                ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                            )
                            .map(Some)
                    } else {
                        Ok(None)
                    };
                    match reserved {
                        Ok(reserved) => Some((entry.pending.take().unwrap(), reserved)),
                        Err(NotificationReserveError::Coordinator(ReserveError::Exhausted)) => None,
                        Err(_) => {
                            entry.pending = None;
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
            tokio::select! { _ = released => {}, _ = changed => {} }
        }
    }
}
impl<R> Drop for AuditFailureWorker<R> {
    fn drop(&mut self) {
        if self.active {
            let mut state = self.failures.lock().unwrap();
            for entry in &mut state.entries {
                entry.pending = None;
            }
            state.owned = false;
        }
    }
}

#[cfg(test)]
#[path = "audit_historical_loss_tests.rs"]
mod historical_tests;
