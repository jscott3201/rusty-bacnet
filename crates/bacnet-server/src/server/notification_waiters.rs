//! The confirmed event notifications waiting for an invoke ID, served
//! first-come (#2563).
//!
//! A confirmed event notification that finds no invoke ID it may lease
//! waits at the head of its destination's lane (#2537). Those waiting are
//! queued here in the order they began to wait, under the transaction
//! state's lock. Only the queue's head tries for an ID: it waits for a lease
//! to end, and the others for their turn. Once the head has its ID, or
//! leaves the queue for any reason (stop, its lane's task aborted, its
//! future dropped), the next is woken to try at once. No ID is ever set
//! aside for a waiter: each takes its own, in a poll of its own future, so
//! one that is cancelled or no longer polled holds none.
//!
//! The queue orders the event notifications among themselves. A
//! reservation that doesn't wait (a COV report, an Audit record, a run's
//! request) and the COV and Audit retries, which keep their own places
//! (#2661, #2671), may still take a freed ID first, as a COV waiter lets a
//! fresh change go first (#2734). But a head that a lease's end woke and
//! that still found no ID was passed over: from then on every other
//! reservation of this server leaves the last free ID to it, until it has
//! one. So the head is passed over by this server's own work at most twice,
//! once as its turn comes and once after a lease ends, and the next ID this
//! server can lease is its own. A node's client role leases from the node's
//! pool directly and is not ordered here; on a seat the server holds at
//! most [`SEAT_LEASES`] and the client keeps the rest.

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
    /// reservation leaves the last free ID to it until it takes one.
    passed_over: bool,
}

struct Waiter {
    ticket: u64,
    /// Woken when its turn comes.
    waker: Option<Waker>,
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
    /// new head.
    fn leave(&mut self, ticket: u64) -> Option<Waker> {
        let at = self
            .queue
            .iter()
            .position(|waiter| waiter.ticket == ticket)?;
        self.queue.remove(at);
        if at != 0 {
            return None;
        }
        self.passed_over = false;
        self.queue.front_mut().and_then(|head| head.waker.take())
    }

    /// The free IDs a reservation other than the head's must leave: one
    /// while the head is owed it.
    pub(super) fn held_back(&self) -> usize {
        usize::from(self.passed_over)
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
            released.await;
            woken = true;
        }
    }
}

impl NotificationCore {
    /// Lease an invoke ID for `metadata`, within the cap, or say why not.
    /// The waiting head (`head`) takes the last free ID; any other
    /// reservation leaves it while the head is owed one. On a lease the
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
        let room = state.pending.len() < self.lease_cap
            && (head.is_some() || !state.waiters.passed_over || self.spare(&state));
        let token = match room.then(|| self.coordinator.reserve(metadata)) {
            Some(Ok(token)) => token,
            Some(Err(ReserveError::Exhausted)) | None => {
                // Woken by a lease's end and still none: passed over.
                if head.is_some_and(|head| head.woken) {
                    state.waiters.passed_over = true;
                }
                return Err(NotificationReserveError::Coordinator(
                    ReserveError::Exhausted,
                ));
            }
            Some(Err(error)) => return Err(NotificationReserveError::Coordinator(error)),
        };
        state.pending.insert(token, sender);
        let next = head.and_then(|head| state.waiters.leave(head.ticket));
        drop(state);
        if let Some(next) = next {
            next.wake();
        }
        Ok(self.reservation(token, receiver))
    }

    /// Whether a reservation other than the head's may go while the head is
    /// owed an ID: at least two are free.
    #[cold]
    #[inline(never)]
    fn spare(&self, state: &NotificationState) -> bool {
        self.free_with(state.pending.len()) >= 2
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

    /// Give up `ticket`'s place, waking the next when it was the head.
    fn leave(&self, ticket: u64) {
        let next = match self.state.lock() {
            Ok(mut state) => state.waiters.leave(ticket),
            Err(_) => None,
        };
        if let Some(next) = next {
            next.wake();
        }
    }
}
