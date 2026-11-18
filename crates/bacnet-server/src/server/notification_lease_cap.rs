//! The cap on the invoke IDs a server on a node's seat holds (#2617).
use super::*;

/// The invoke IDs a server on a node's seat may hold at once (#2617): its
/// confirmed notifications (COV, event and Audit together, the node's
/// source Audit records among them) and the reads its runs make. A
/// quarter of the node's 256, so its client keeps the rest. Past it each
/// waits or gives up as it does when the node has no ID free: an event
/// notification waits for one, a COV notification is skipped with a
/// warning, an Audit record is counted lost.
pub(in crate::server) const SEAT_LEASES: usize = 64;

impl NotificationCore {
    /// [`Self::reserve`] on a seat: the count and the lease are taken under
    /// one guard, so racing reservations never pass the cap. Refused as
    /// the node's pool refuses when it is out of IDs, and with nothing
    /// leased, so a waiter's wake comes only from a lease that ends.
    pub(super) fn reserve_within_cap(
        self: &Arc<Self>,
        metadata: LeaseMetadata,
    ) -> Result<NotificationReservation, NotificationReserveError> {
        let (sender, receiver) = oneshot::channel();
        let mut state = self
            .state
            .lock()
            .map_err(|_| NotificationReserveError::StatePoisoned)?;
        if state.closed {
            return Err(NotificationReserveError::Closed);
        }
        if state.pending.len() >= self.lease_cap {
            return Err(NotificationReserveError::Coordinator(
                ReserveError::Exhausted,
            ));
        }
        let token = self
            .coordinator
            .reserve(metadata)
            .map_err(NotificationReserveError::Coordinator)?;
        state.pending.insert(token, sender);
        drop(state);
        Ok(self.reservation(token, receiver))
    }

    /// The operation that holds `token`, and the receiver of its answer.
    pub(super) fn reservation(
        self: &Arc<Self>,
        token: LeaseToken,
        receiver: oneshot::Receiver<CovAckResult>,
    ) -> NotificationReservation {
        (
            NotificationOperation {
                transactions: Arc::clone(self),
                token,
                active: true,
            },
            receiver,
        )
    }
}
