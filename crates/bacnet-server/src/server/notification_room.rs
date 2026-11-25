//! The invoke IDs the server may lease (#2617): those the pool lets its
//! confirmed notifications and its runs' requests take. A standalone
//! server's pool is its own. On a node's seat the pool is the node's, shared
//! with its client: the server always finds its reserve there
//! ([`NOTIFICATION_RESERVE`](bacnet_endpoint_core::coordinator::NOTIFICATION_RESERVE)),
//! and beyond it borrows what the client's reserve doesn't keep.
use super::*;

impl NotificationTransactions {
    /// How many invoke IDs a reservation could take now: those the pool
    /// lets the server take, less what an event notification waiting for an
    /// ID is owed (#2563), and none once closed. Racing work may take them
    /// first.
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
        // lock-poison: fail-closed (poisoned acts closed: no room)
        let owed = match self.state.lock() {
            Ok(state) if !state.closed => state.waiters.owed(),
            _ => return 0,
        };
        let room = self.room();
        if owed {
            Self::spare_with(room)
        } else {
            room.available
        }
    }

    /// What the pool lets the server lease now; nothing when it is poisoned.
    pub(super) fn room(&self) -> Room {
        self.coordinator
            .room(LeaseOwner::Notification)
            .unwrap_or(Room {
                available: 0,
                reserve_left: Some(0),
            })
    }

    /// The IDs a reservation other than the waiting head's may take while
    /// the head is owed one (#2563): all but one of those sure to stay the
    /// server's. On a pool that keeps the server a reserve, the node's
    /// client may take any other free ID first, so only what is left of the
    /// reserve is sure (#2617); on one that keeps none, nothing but the
    /// server's own work competes.
    pub(super) fn spare_with(room: Room) -> usize {
        room.reserve_left
            .map_or(room.available, |left| left.min(room.available))
            .saturating_sub(1)
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
