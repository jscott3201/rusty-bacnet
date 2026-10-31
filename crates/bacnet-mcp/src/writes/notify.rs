//! Telling subscribers of `bacnet://session` and `bacnet://ledger` that
//! they changed (#2160): a watch channel each, bumped when this server
//! arms a session, spends a plan grant's budget, records a write as
//! `sending`, or settles or reverts one, and by [`follow_ends`] when an
//! armed session or a plan grant runs out, which no request marks.
//!
//! Listen streams and legacy subscriptions wait on the channels like a
//! point's change signal ([`crate::follow::Follow`]), so bumps made while
//! a notification is going out are announced once. The ledger is the
//! workspace's, shared with the TUI: only this server's own changes are
//! announced.

use std::time::Duration;

use bacnet_ops::safety::desk::Desk;
use bacnet_ops::safety::ledger::Entry;
use bacnet_transport::port::TransportPort;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::context::Ctx;
use crate::resources::uri::Uri;

/// One resource's change signal, as a follower waits on it.
pub(crate) type Signal = watch::Receiver<u64>;

/// The two resources' change signals.
pub(crate) struct Signals {
    session: watch::Sender<u64>,
    ledger: watch::Sender<u64>,
}

impl Default for Signals {
    fn default() -> Self {
        Self {
            session: watch::Sender::new(0),
            ledger: watch::Sender::new(0),
        }
    }
}

impl Signals {
    /// `bacnet://session` changed.
    pub(crate) fn session_changed(&self) {
        self.session.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// `bacnet://ledger` changed.
    pub(crate) fn ledger_changed(&self) {
        self.ledger.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Wakes on the next change to the session.
    pub(crate) fn session(&self) -> watch::Receiver<u64> {
        self.session.subscribe()
    }

    /// Wakes on the next change to the ledger.
    pub(crate) fn ledger(&self) -> watch::Receiver<u64> {
        self.ledger.subscribe()
    }

    /// The signal of `uri`, when it is the session or the ledger.
    pub(crate) fn of(&self, uri: Uri) -> Option<Signal> {
        match uri {
            Uri::Session => Some(self.session()),
            Uri::Ledger => Some(self.ledger()),
            _ => None,
        }
    }

    /// Announce the ledger if `desk`'s entries are no longer `before` (a
    /// write settled or reverted, or one left `sending` was settled), or
    /// if the write's `sending` rows were `announced`: a write the device
    /// refused takes its row out again, which is a change too.
    pub(crate) fn ledger_settled(&self, before: &[Entry], desk: &Desk, announced: bool) {
        if announced || desk.entries() != before {
            self.ledger_changed();
        }
    }
}

/// The longest [`follow_ends`] sleeps before it looks at the wall clock
/// again, so a clock step or a suspend can't keep it from a plan's end for
/// longer than this.
const LOOK_AGAIN: Duration = Duration::from_secs(60);

/// Announce the session when its armed session or its plan grant runs
/// out, until the server closes: no request arrives to mark either. The
/// plan's end is a wall-clock time, checked on the clock
/// [`Writes::plan`](super::Writes::plan) uses each time this wakes, at
/// least every [`LOOK_AGAIN`]. A plan already over when the server starts
/// is no change to announce.
pub(crate) async fn follow_ends<T: TransportPort + 'static>(ctx: &Ctx<T>) {
    let mut changed = ctx.writes.signals.session();
    let plan_end = ctx
        .policy
        .writes
        .plan
        .as_ref()
        .map(|plan| plan.expires_unix);
    let mut plan_over = plan_end.is_none_or(|end| ctx.writes.wall_now() >= end);
    loop {
        let plan_left = match plan_end {
            Some(end) if !plan_over => {
                let left = end.saturating_sub(ctx.writes.wall_now());
                if left == 0 {
                    plan_over = true;
                    ctx.writes.signals.session_changed();
                    continue;
                }
                Some(Duration::from_secs(left).min(LOOK_AGAIN))
            }
            _ => None,
        };
        let session_end = ctx.writes.armed_until();
        let next = match (session_end, plan_left) {
            (Some(end), Some(left)) => Some(end.min(Instant::now() + left)),
            (end, left) => end.or_else(|| left.map(|left| Instant::now() + left)),
        };
        tokio::select! {
            () = ctx.closing.cancelled() => return,
            seen = changed.changed() => {
                // A new session, or this task's own announcement: look
                // again at what ends next.
                if seen.is_err() {
                    return;
                }
            }
            () = tokio::time::sleep_until(next.unwrap_or_else(Instant::now)), if next.is_some() => {
                // The armed session's end, or time to look at the plan's
                // (at the top of the loop).
                if session_end.is_some_and(|end| end <= Instant::now()) {
                    ctx.writes.signals.session_changed();
                }
            }
        }
    }
}
