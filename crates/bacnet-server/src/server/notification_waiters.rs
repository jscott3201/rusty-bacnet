//! The confirmed event notifications waiting for an invoke ID, served
//! first-come (#2563).
//!
//! A confirmed event notification that finds no invoke ID it may lease
//! waits at the head of its destination's lane (#2537). Those waiting are
//! queued here in the order they began to wait, under the transaction
//! state's lock. Only the queue's head tries for an ID: it waits for a lease
//! to end, and the others for their turn. Once the head has its ID, or
//! leaves the queue for any reason (stop, its lane's task aborted, its
//! future dropped), the next is woken to try at once. Closing the state
//! wakes every waiter, the head too, to find it closed. No ID is ever set
//! aside for a waiter: each takes its own, in a poll of its own future, so
//! one that is cancelled or no longer polled holds none.
//!
//! The queue orders the event notifications among themselves. A
//! reservation that doesn't wait (a COV report, an Audit record, a run's
//! request) and the COV and Audit retries, which keep their own places
//! (#2661, #2671), may still take a freed ID first, as a COV waiter lets a
//! fresh change go first (#2734). But a head that a lease's end woke and
//! that still found no ID was passed over: from then on every other
//! reservation of this server leaves it an ID sure to be the server's, until
//! it has one: the last free one in a pool of the server's own, the last of
//! the server's reserve in a node's (below). So the head is passed over by
//! this server's own work at most twice, once as its turn comes and once
//! after a lease ends, and the next ID this server can lease is its own. In
//! that window, until the woken head is polled, that ID is refused to the
//! rest as when none is free: a Command or Channel run's request fails, an
//! Audit record is dropped or kept for a retry as it would be then, and a
//! COV report is owed a look. On a node that can happen with other IDs
//! free, so once such work was held back, the head's leaving the queue
//! wakes the pool's waits for an ID to look again.
//!
//! On a node's seat the client leases from the node's pool directly, not
//! through this queue, and may take a freed ID ahead of the head. It can
//! only while the server holds its reserve or more (#2617,
//! [`NOTIFICATION_RESERVE`]): the client never holds more than the pool
//! less that reserve, so a server under it always finds an ID, and the head
//! waits only while the server holds that many or more. Once the head is
//! owed, the server's other work leaves it the last of the reserve, so what
//! the server holds only falls until one of its leases ends with it under
//! its reserve, and that ID is the head's. Each lease ends within the
//! longest the server's request stays open (the APDU timeout times the
//! retries plus one, after its sends): the head is served within two of
//! those of its first wait, whatever the client does.
//!
//! [`NOTIFICATION_RESERVE`]: bacnet_endpoint_core::coordinator::NOTIFICATION_RESERVE

use super::*;
use std::collections::VecDeque;
use std::task::Context;

#[cfg(test)]
#[path = "notification_waiters_tests.rs"]
mod tests;

/// The notifications waiting for an invoke ID, oldest first.
#[derive(Default)]
pub(super) struct Waiters {
    queue: VecDeque<Waiter>,
    /// The ticket of the next to arrive.
    next: u64,
    /// The head was woken by a lease's end and found no ID: every other
    /// reservation leaves it an ID sure to be the server's until it takes
    /// one ([`NotificationCore::spare_with`]).
    passed_over: bool,
    /// Other work was refused an ID while the head was owed one. On a node
    /// that can happen with several free (#2617), so as the head leaves the
    /// pool's waits for capacity are woken to look again: no lease's end may
    /// come to wake them.
    held_work: bool,
}

struct Waiter {
    ticket: u64,
    /// Woken when its turn comes, or as the state closes.
    waker: Option<Waker>,
}

/// What a waiter's leaving the queue wakes, once the state's lock is let
/// go.
#[derive(Default)]
struct Left {
    /// The new head, for its turn.
    next: Option<Waker>,
    /// Whether work was held back for the head that left.
    held_work: bool,
}

impl Left {
    fn wake(self, coordinator: &OutboundTransactionCoordinator) {
        if let Some(next) = self.next {
            next.wake();
        }
        if self.held_work {
            coordinator.wake_released();
        }
    }
}

impl Waiters {
    /// Queue one more waiter, last.
    fn join(&mut self) -> u64 {
        let ticket = self.next;
        self.next = self.next.wrapping_add(1);
        self.queue.push_back(Waiter {
            ticket,
            waker: None,
        });
        ticket
    }

    /// Take `ticket` out of the queue; when it was the head, what wakes the
    /// new head and the work held back for the old one.
    fn leave(&mut self, ticket: u64) -> Left {
        let Some(at) = self.queue.iter().position(|waiter| waiter.ticket == ticket) else {
            return Left::default();
        };
        self.queue.remove(at);
        if at != 0 {
            return Left::default();
        }
        self.passed_over = false;
        Left {
            next: self.queue.front_mut().and_then(|head| head.waker.take()),
            held_work: std::mem::take(&mut self.held_work),
        }
    }

    /// Whether the head is owed the next ID: a reservation other than the
    /// head's must leave it one.
    pub(super) fn owed(&self) -> bool {
        self.passed_over
    }

    /// Wake every waiter, as the state closes, each to find it closed.
    pub(super) fn wake_all(&mut self) -> Vec<Waker> {
        (self.queue.iter_mut())
            .filter_map(|waiter| waiter.waker.take())
            .collect()
    }

    /// How many notifications wait.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.queue.len()
    }
}

/// The waiting head's attempt: its ticket, and whether a lease's end woke it
/// since its last.
#[derive(Clone, Copy)]
pub(super) struct Head {
    ticket: u64,
    woken: bool,
}

/// What [`NotificationCore::arrive`] did.
enum Arrival {
    Leased(NotificationReservation),
    /// Queued with this ticket.
    Queued(u64),
}

/// A place in the queue, given up as it drops.
struct Place<'a> {
    core: &'a NotificationCore,
    /// `None` once the place is gone: served, or left.
    ticket: Option<u64>,
}

impl Drop for Place<'_> {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            self.core.leave(ticket);
        }
    }
}

impl NotificationTransactions {
    /// [`Self::reserve`], waiting while no invoke ID may be leased: a
    /// confirmed event notification holds its lane until it has one
    /// (#2537). The waiters are served in the order they arrive (#2563): one
    /// that arrives while others wait queues behind them without trying, and
    /// only the head tries, each time a lease ends. `waits` is called once,
    /// as the wait starts. Any other refusal returns at once. No lock is held
    /// while it waits, and dropping it gives up its place.
    pub(in crate::server) async fn reserve_waiting(
        &self,
        peer: CanonicalPeer,
        service_choice: ConfirmedServiceChoice,
        waits: impl FnOnce(),
    ) -> Result<NotificationReservation, NotificationReserveError> {
        let metadata = || LeaseMetadata::notification(peer.clone(), service_choice);
        let core = &self.core;
        let mut place = match core.arrive(metadata())? {
            Arrival::Leased(reservation) => return Ok(reservation),
            Arrival::Queued(ticket) => Place {
                core,
                ticket: Some(ticket),
            },
        };
        waits();
        let mut woken = false;
        loop {
            poll_fn(|cx| core.poll_turn(cx, place.ticket)).await;
            // Made before the attempt, so a lease that ends after it fails
            // still ends this wait: the coordinator's wake reaches every
            // waiter already made, polled or not, and stores no permit.
            let released = core.coordinator.released();
            let ticket = place.ticket.expect("a place until served");
            match core.lease(metadata(), Some(Head { ticket, woken })) {
                Err(NotificationReserveError::Coordinator(ReserveError::Exhausted)) => {}
                result => {
                    // Served, or refused for good: the place is gone.
                    if result.is_ok() {
                        place.ticket = None;
                    }
                    return result;
                }
            }
            // A lease's end wakes it, or the state closing, which wakes the
            // head through its place as it wakes the rest: with no lease of
            // this server's own to cancel, closing ends no lease.
            let mut released = std::pin::pin!(released);
            woken = poll_fn(|cx| match released.as_mut().poll(cx) {
                Poll::Ready(()) => Poll::Ready(true),
                Poll::Pending => core.park(cx, ticket).map(|()| false),
            })
            .await;
        }
    }

    /// How many event notifications wait for an invoke ID, for tests.
    #[cfg(test)]
    pub(in crate::server) fn waiting_for_ids(&self) -> usize {
        self.core
            .state
            .lock()
            .map_or(0, |state| state.waiters.len())
    }
}

impl NotificationCore {
    /// Lease an invoke ID for `metadata`, as the pool allows, or say why
    /// not. The waiting head (`head`) takes the last free ID; any other
    /// reservation leaves it one while the head is owed it. On a lease the
    /// head leaves the queue and the next is woken.
    pub(super) fn lease(
        self: &Arc<Self>,
        metadata: LeaseMetadata,
        head: Option<Head>,
    ) -> Result<NotificationReservation, NotificationReserveError> {
        let (sender, receiver) = oneshot::channel();
        let mut state = self
            .state
            .lock()
            .map_err(|_| NotificationReserveError::StatePoisoned)?;
        if state.closed {
            return Err(NotificationReserveError::Closed);
        }
        let room = head.is_some() || !state.waiters.passed_over || self.spare();
        let token = match room.then(|| self.coordinator.reserve(metadata)) {
            Some(Ok(token)) => token,
            Some(Err(ReserveError::Exhausted)) | None => {
                // Woken by a lease's end and still none: passed over.
                if head.is_some_and(|head| head.woken) {
                    state.waiters.passed_over = true;
                }
                // Held back for the head, perhaps with IDs free.
                state.waiters.held_work |= !room;
                return Err(NotificationReserveError::Coordinator(
                    ReserveError::Exhausted,
                ));
            }
            Some(Err(error)) => return Err(NotificationReserveError::Coordinator(error)),
        };
        state.pending.insert(token, sender);
        let left = head.map(|head| state.waiters.leave(head.ticket));
        drop(state);
        if let Some(left) = left {
            left.wake(&self.coordinator);
        }
        Ok(self.reservation(token, receiver))
    }

    /// Whether a reservation other than the head's may go while the head is
    /// owed an ID: it still leaves the head one sure to be the server's
    /// ([`Self::spare_with`]). Called under the state's lock; the
    /// coordinator's is taken inside it.
    #[cold]
    #[inline(never)]
    fn spare(&self) -> bool {
        Self::spare_with(self.room()) >= 1
    }

    /// Lease at once when nobody waits, or queue: behind those waiting, or
    /// as the head when no ID may be leased.
    fn arrive(
        self: &Arc<Self>,
        metadata: LeaseMetadata,
    ) -> Result<Arrival, NotificationReserveError> {
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| NotificationReserveError::StatePoisoned)?;
            if state.closed {
                return Err(NotificationReserveError::Closed);
            }
            if !state.waiters.queue.is_empty() {
                return Ok(Arrival::Queued(state.waiters.join()));
            }
        }
        match self.lease(metadata, None) {
            Ok(reservation) => Ok(Arrival::Leased(reservation)),
            Err(NotificationReserveError::Coordinator(ReserveError::Exhausted)) => {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| NotificationReserveError::StatePoisoned)?;
                Ok(Arrival::Queued(state.waiters.join()))
            }
            Err(error) => Err(error),
        }
    }

    /// Ready once `ticket` heads the queue, or once the state is closed or
    /// poisoned, for the attempt to say so.
    fn poll_turn(&self, cx: &mut Context<'_>, ticket: Option<u64>) -> Poll<()> {
        let Ok(mut state) = self.state.lock() else {
            return Poll::Ready(());
        };
        let closed = state.closed;
        let queue = &mut state.waiters.queue;
        match queue
            .iter()
            .position(|waiter| Some(waiter.ticket) == ticket)
        {
            Some(at) if at > 0 && !closed => {
                let waker = &mut queue[at].waker;
                if !waker.as_ref().is_some_and(|w| w.will_wake(cx.waker())) {
                    *waker = Some(cx.waker().clone());
                }
                Poll::Pending
            }
            _ => Poll::Ready(()),
        }
    }

    /// Ready once the state is closed or poisoned; otherwise `ticket`'s place
    /// keeps `cx`'s waker, for closing to wake.
    fn park(&self, cx: &mut Context<'_>, ticket: u64) -> Poll<()> {
        let Ok(mut state) = self.state.lock() else {
            return Poll::Ready(());
        };
        if state.closed {
            return Poll::Ready(());
        }
        let queue = &mut state.waiters.queue;
        if let Some(waiter) = queue.iter_mut().find(|waiter| waiter.ticket == ticket) {
            waiter.waker = Some(cx.waker().clone());
        }
        Poll::Pending
    }

    /// Give up `ticket`'s place, waking the next when it was the head, and
    /// the work held back for it.
    fn leave(&self, ticket: u64) {
        let left = match self.state.lock() {
            Ok(mut state) => state.waiters.leave(ticket),
            Err(_) => Left::default(),
        };
        left.wake(&self.coordinator);
    }
}
