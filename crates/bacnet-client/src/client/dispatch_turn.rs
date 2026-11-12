//! The order in which the client's dispatch loop takes its work (#1978 item 8).
//!
//! The loop serves three sources: the network layer's APDUs, the network
//! controls met while receiving them, and the cleanups that requests given up
//! on send. A `select!` over them polled every source on every wake, from a
//! random start, and each idle source it polled registered the task's waker
//! again. A ReadProperty round trip wakes the loop twice: once for the
//! answer, once to find nothing left. [`TurnOrder`] takes them in a fixed
//! order instead:
//!
//! - **Controls first, with no waker.** A layer started with
//!   [`NetworkLayer::start`](bacnet_network::layer::NetworkLayer::start)
//!   hands a control to its receiver only while the APDU receiver is polled,
//!   and only this task polls it. So a `try_recv` finds every control there
//!   is: before the APDU receiver is polled, those an earlier turn left, and
//!   after, those that poll just queued. The layer's ingress sequence
//!   advances for each control it admits, so while it stands where the
//!   queue was last found empty, the loop doesn't read the queue at all.
//! - **Then APDUs, then cleanups.** A turn that finds an APDU doesn't poll
//!   the cleanup queue, except that after [`APDUS_BEFORE_CLEANUP`] APDUs in a
//!   row the next turn polls it first: a steady stream of APDUs can't hold a
//!   cleanup back for longer than that. A turn that finds no APDU polls the
//!   cleanup queue, so a turn that finds nothing has registered with both
//!   receivers, and either wakes the task.
//!
//! The device purge, the loop's fourth source before, is a task of its own
//! (`lifecycle.rs`), so its timer isn't polled on every turn either.

use std::task::{Context, Poll};

/// APDUs the loop takes in a row before a turn polls the cleanup queue first.
pub(super) const APDUS_BEFORE_CLEANUP: u32 = 16;

/// What one turn of the dispatch loop took.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Turn<A, N, C> {
    /// A network control.
    Control(N),
    /// An APDU, or `None` once the APDU receiver is closed and drained.
    Apdu(Option<A>),
    /// A cleanup, or `None` once every sender is gone.
    Cleanup(Option<C>),
}

/// How many APDUs the loop has taken in a row; see the [module docs](self).
#[derive(Debug, Default)]
pub(super) struct TurnOrder {
    apdus_in_a_row: u32,
}

impl TurnOrder {
    /// Take the next piece of work, or, with every source polled, return
    /// `Pending`.
    ///
    /// `controls` takes a queued control without waiting. `apdus` and
    /// `cleanups` poll their receivers, each at most once per turn.
    pub(super) fn poll_turn<A, N, C>(
        &mut self,
        cx: &mut Context<'_>,
        mut controls: impl FnMut() -> Option<N>,
        apdus: impl FnOnce(&mut Context<'_>) -> Poll<Option<A>>,
        cleanups: impl FnOnce(&mut Context<'_>) -> Poll<Option<C>>,
    ) -> Poll<Turn<A, N, C>> {
        if let Some(control) = controls() {
            return Poll::Ready(Turn::Control(control));
        }
        let mut cleanups = Some(cleanups);
        if self.apdus_in_a_row >= APDUS_BEFORE_CLEANUP
            && let Poll::Ready(cleanup) = self.poll_cleanups(&mut cleanups, cx)
        {
            return Poll::Ready(Turn::Cleanup(cleanup));
        }
        if let Poll::Ready(apdu) = apdus(cx) {
            self.apdus_in_a_row = self.apdus_in_a_row.saturating_add(1);
            return Poll::Ready(Turn::Apdu(apdu));
        }
        if let Poll::Ready(cleanup) = self.poll_cleanups(&mut cleanups, cx) {
            return Poll::Ready(Turn::Cleanup(cleanup));
        }
        match controls() {
            Some(control) => Poll::Ready(Turn::Control(control)),
            None => Poll::Pending,
        }
    }

    /// Poll the cleanup queue, unless this turn already has; either way the
    /// run of APDUs ends.
    fn poll_cleanups<C>(
        &mut self,
        cleanups: &mut Option<impl FnOnce(&mut Context<'_>) -> Poll<Option<C>>>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<C>> {
        self.apdus_in_a_row = 0;
        cleanups.take().map_or(Poll::Pending, |poll| poll(cx))
    }
}
