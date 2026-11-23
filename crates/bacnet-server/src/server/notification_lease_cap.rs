//! The cap on the invoke IDs a server on a node's seat holds (#2617).
use super::*;

/// The invoke IDs a server on a node's seat may hold at once (#2617): its
/// confirmed notifications (COV, event and Audit together, the node's
/// source Audit records among them) and the reads its runs make. A
/// quarter of the node's 256, so its client keeps the rest. Past it each
/// waits or gives up as it does when the node has no ID free: an event
/// notification waits for one, a COV notification's subscription is owed
/// a look once one comes back (#2661), an Audit record is counted lost.
pub(in crate::server) const SEAT_LEASES: usize = 64;

/// The invoke IDs of a device's pool.
const INVOKE_IDS: usize = 256;

impl NotificationTransactions {
    /// How many invoke IDs a reservation could take now: those free in the
    /// pool, within the seat's cap on a seat, less the one an event
    /// notification waiting for an ID is owed (#2563), and none once
    /// closed. Racing work may take them first.
    pub(in crate::server) fn free_invoke_ids(&self) -> usize {
        self.core.free()
    }

    /// Completes once a lease of the pool ends after this is made, whether
    /// or not it was polled by then: the coordinator wakes every waiter
    /// already made.
    pub(in crate::server) fn released(&self) -> tokio::sync::futures::Notified<'_> {
        self.core.coordinator.released()
    }
}

impl NotificationCore {
    /// See [`NotificationTransactions::free_invoke_ids`].
    fn free(&self) -> usize {
        let (held, held_back) = match self.state.lock() {
            Ok(state) if !state.closed => (state.pending.len(), state.waiters.held_back()),
            _ => return 0,
        };
        self.free_with(held).saturating_sub(held_back)
    }

    /// The IDs free in the pool, within the cap less the `held` this server
    /// leases. Read under the transaction state's guard, the count and a
    /// lease are one step ([`Self::lease`]), so racing reservations never
    /// pass the cap.
    pub(super) fn free_with(&self, held: usize) -> usize {
        let pool = self
            .coordinator
            .active_count()
            .map_or(0, |active| INVOKE_IDS.saturating_sub(active));
        self.lease_cap.saturating_sub(held).min(pool)
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
