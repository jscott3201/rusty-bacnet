//! A gate that one holder passes at a time, taken only by a waiter as it
//! is polled (#2759).
//!
//! Tokio's fair `Mutex` hands its lock to the first queued waiter when the
//! holder lets go, whether or not anything polls that waiter again. A
//! request parked in a `select!` arm, a `join!` or a `buffer_unordered`
//! stream while its caller awaits something else then owns the gate, and
//! everyone behind it waits for a future that makes no progress: a loop
//! body that sends through the router its stream's requests wait for never
//! returns.
//!
//! Here a waiter takes the gate only in its own poll. A waiter that finds
//! it held sleeps; when the holder lets go, every sleeping waiter is woken,
//! oldest first. A waiter polled while the gate is free takes it if no
//! older waiter is still waiting. Otherwise it steps aside once: it wakes
//! itself and returns pending, so an older waiter woken before it is polled
//! first and goes first. On its next poll, if the gate is still free and
//! nothing has let it go since, it takes it whatever waits ahead of it. So
//! a waiter left unpolled holds nothing and holds the others up by one turn
//! of their executor each time the gate comes free, while waiters polled
//! when woken go one at a time in the order they first waited, a newcomer
//! included (it steps aside for the woken). That order is strict when they
//! are polled in the order they are woken: in one task, or on a
//! current-thread runtime. On a multi-thread runtime one slow to be polled
//! can be passed. A holder holds the gate until its guard drops, whether
//! its own caller polls it or not: only the wait is taken on poll.
//!
//! A release wakes every waiter, so draining n waiters that all wait at
//! once costs about n²/2 wakes and polls. The state's lock is a leaf, held
//! for a few steps and never across an await; wakers are woken, and those
//! replaced or removed dropped, once it is let go.

use bacnet_types::sync::MutexExt;
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

/// A one-holder gate guarding no data: holding it across awaits is its
/// purpose. See the module documentation.
#[derive(Default)]
pub(crate) struct PollGate {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    held: bool,
    /// How many times the gate has been let go: a waiter that steps aside
    /// takes the gate at its next poll only within the same turn.
    turn: u64,
    /// The next waiter's number; numbers only grow.
    next: u64,
    /// Every waiter still waiting, by number: the oldest first.
    waiters: BTreeMap<u64, Waiter>,
}

#[derive(Default)]
struct Waiter {
    /// Its waker while it sleeps; taken when it is woken.
    waker: Option<Waker>,
    /// The turn it last stepped aside in.
    stepped_aside: Option<u64>,
}

impl PollGate {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.locked()
    }

    /// Wait for the gate, borrowing it.
    pub(crate) fn lock(&self) -> Take<&Self> {
        Take {
            gate: Some(self),
            number: None,
        }
    }

    /// Wait for the gate, holding a handle on it.
    pub(crate) fn lock_owned(self: Arc<Self>) -> Take<Arc<Self>> {
        Take {
            gate: Some(self),
            number: None,
        }
    }

    /// The gate if it is free now, ahead of any waiter woken for it. Never
    /// waits.
    pub(crate) fn try_lock(&self) -> Option<PollGateGuard<&Self>> {
        let mut state = self.state();
        if state.held {
            return None;
        }
        state.held = true;
        Some(PollGateGuard { gate: self })
    }

    /// Waiter `number` (none before its first wait) polled with `cx`.
    fn poll_take(&self, cx: &Context<'_>, number: &mut Option<u64>) -> Poll<()> {
        let mut guard = self.state();
        let state = &mut *guard;
        if !state.held {
            let oldest = state.waiters.keys().next().copied();
            let takes = match (*number, oldest) {
                (_, None) => true,
                (Some(mine), Some(oldest)) => {
                    mine == oldest
                        || state.waiters.get(&mine).and_then(|w| w.stepped_aside)
                            == Some(state.turn)
                }
                (None, Some(_)) => false,
            };
            if takes {
                state.held = true;
                let gone = number.take().and_then(|mine| state.waiters.remove(&mine));
                drop(guard);
                drop(gone);
                return Poll::Ready(());
            }
            let turn = state.turn;
            let waiter = waiter(state, number);
            waiter.stepped_aside = Some(turn);
            let dropped = waiter.waker.take();
            drop(guard);
            drop(dropped);
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        let waiter = waiter(state, number);
        let dropped = match &waiter.waker {
            Some(waker) if waker.will_wake(cx.waker()) => None,
            _ => waiter.waker.replace(cx.waker().clone()),
        };
        drop(guard);
        drop(dropped);
        Poll::Pending
    }

    /// The holder lets go: every sleeping waiter is woken, oldest first.
    fn release(&self) {
        let woken: Vec<Waker> = {
            let mut state = self.state();
            state.held = false;
            state.turn = state.turn.wrapping_add(1);
            state
                .waiters
                .values_mut()
                .filter_map(|waiter| waiter.waker.take())
                .collect()
        };
        woken.into_iter().for_each(Waker::wake);
    }

    /// Waiter `number` leaves without the gate. Everyone asleep is woken
    /// at each release, so it passes nothing on.
    fn leave(&self, number: u64) {
        let gone = self.state().waiters.remove(&number);
        drop(gone);
    }
}

/// Waiter `number`'s entry, made at its first wait.
fn waiter<'s>(state: &'s mut State, number: &mut Option<u64>) -> &'s mut Waiter {
    let mine = *number.get_or_insert_with(|| {
        let mine = state.next;
        state.next += 1;
        mine
    });
    state.waiters.entry(mine).or_default()
}

impl fmt::Debug for PollGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state();
        f.debug_struct("PollGate")
            .field("held", &state.held)
            .field("waiters", &state.waiters.len())
            .finish()
    }
}

/// A wait for a [`PollGate`]: [`PollGate::lock`] or
/// [`PollGate::lock_owned`]. Dropping it leaves the wait.
pub(crate) struct Take<G: Deref<Target = PollGate>> {
    /// The gate, until the wait ends with it.
    gate: Option<G>,
    number: Option<u64>,
}

impl<G: Deref<Target = PollGate> + Unpin> Future for Take<G> {
    type Output = PollGateGuard<G>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<PollGateGuard<G>> {
        let this = self.get_mut();
        let gate = this
            .gate
            .as_ref()
            .expect("a gate wait polled after it ended");
        match gate.poll_take(cx, &mut this.number) {
            Poll::Ready(()) => Poll::Ready(PollGateGuard {
                gate: this.gate.take().expect("taken once"),
            }),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<G: Deref<Target = PollGate>> Drop for Take<G> {
    fn drop(&mut self) {
        if let (Some(gate), Some(number)) = (&self.gate, self.number) {
            gate.leave(number);
        }
    }
}

/// The gate, held until this is dropped.
pub(crate) struct PollGateGuard<G: Deref<Target = PollGate>> {
    gate: G,
}

impl<G: Deref<Target = PollGate>> Drop for PollGateGuard<G> {
    fn drop(&mut self) {
        self.gate.release();
    }
}

#[cfg(test)]
#[path = "poll_gate_tests.rs"]
mod tests;
