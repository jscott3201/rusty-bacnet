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
        use bacnet_types::constructed::{BACnetAuditNotification, BACnetRecipient};
        let mut value = bytes::BytesMut::new();
        bacnet_encoding::primitives::encode_app_unsigned(&mut value, self.count);
        BACnetAuditNotification {
            source_timestamp: None,
            target_timestamp: Some(self.earliest.clone()),
            source_device: BACnetRecipient::Device(self.context.device),
            source_object: None,
            operation: bacnet_types::enums::AuditOperation::AUDITING_FAILURE,
            source_comment: None,
            target_comment: None,
            invoke_id: None,
            source_user_id: None,
            source_user_role: None,
            target_device: BACnetRecipient::Device(self.context.device),
            target_object: None,
            target_property: None,
            target_priority: None,
            target_value: None,
            current_value: Some(value.to_vec()),
            result: None,
        }
    }
}

struct State<R> {
    owned: bool,
    current: Option<Arc<AuditFailureContext<R>>>,
    order: u64,
    pending: Option<AuditFailureBatch<R>>,
}

#[doc(hidden)]
pub struct AuditFailureQueue<R> {
    state: Arc<Mutex<State<R>>>,
    changed: Arc<tokio::sync::Notify>,
}

impl<R> Default for AuditFailureQueue<R> {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                owned: false,
                current: None,
                order: 0,
                pending: None,
            })),
            changed: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

impl<R> AuditFailureQueue<R> {
    #[cfg(test)]
    pub(super) fn resources(&self) -> (bool, u64) {
        let state = self.state.lock().unwrap();
        (
            state.owned,
            state.pending.as_ref().map_or(0, |batch| batch.count),
        )
    }
}

impl<R: PartialEq> AuditFailureQueue<R> {
    /// Capture admission order, not completion or wire-timestamp order.
    /// A new context supersedes the single pending batch, without transferring it.
    pub fn observe(&self, context: AuditFailureContext<R>) -> Option<AuditFailureTicket<R>> {
        if !context.enabled() {
            return None;
        }
        let mut state = self.state.lock().unwrap();
        state.order = state.order.checked_add(1)?;
        let changed = !state.current.as_ref().is_some_and(|old| old.same(&context));
        if changed {
            state.pending = None;
            state.current = Some(Arc::new(context));
        }
        let ticket = AuditFailureTicket {
            context: Arc::clone(state.current.as_ref().unwrap()),
            order: state.order,
        };
        drop(state);
        if changed {
            self.changed.notify_waiters();
        }
        Some(ticket)
    }

    pub fn record_drop(
        &self,
        owner: &NotificationTransactions,
        ticket: AuditFailureTicket<R>,
        timestamp: BACnetTimeStamp,
        count: u64,
    ) -> Option<AuditFailureWorker<R>> {
        if !ticket.context.enabled() || owner.audit_permits.is_closed() {
            return None;
        }
        let mut state = self.state.lock().unwrap();
        if !state
            .current
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &ticket.context))
        {
            return None;
        }
        if let Some(batch) = &mut state.pending {
            batch.count = batch.count.saturating_add(count);
            if ticket.order < batch.earliest_order {
                batch.earliest_order = ticket.order;
                batch.earliest = timestamp;
            }
        } else {
            state.pending = Some(AuditFailureBatch {
                context: ticket.context,
                count,
                earliest: timestamp,
                earliest_order: ticket.order,
            });
        }
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
        if state
            .pending
            .as_ref()
            .is_some_and(|batch| !batch.context.enabled())
        {
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
            // Register both wake reasons before inspecting state/resources.
            let released = coordinator.released();
            let changed = changed_owner.notified();
            tokio::pin!(released, changed);
            released.as_mut().enable();
            changed.as_mut().enable();
            let ready = {
                let mut state = self.failures.lock().unwrap();
                let Some(batch) = state.pending.as_ref() else {
                    state.owned = false;
                    self.active = false;
                    return None;
                };
                if !batch.context.enabled() {
                    state.pending = None;
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
                        Ok(reserved) => Some((state.pending.take().unwrap(), reserved)),
                        Err(NotificationReserveError::Coordinator(ReserveError::Exhausted)) => None,
                        Err(_) => {
                            state.pending = None;
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
            state.pending = None;
            state.owned = false;
        }
    }
}
