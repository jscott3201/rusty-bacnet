//! Telling subscribers of `bacnet://session` and `bacnet://ledger` that
//! they changed (#2160): a watch channel each, bumped when this server
//! arms a session, spends a plan grant's budget or settles or reverts a
//! write, and by [`follow_ends`] when an armed session or a plan grant
//! runs out, which no request marks.
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

use super::unix_now;
use crate::context::Ctx;

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

    /// Announce the ledger if `desk`'s entries are no longer `before`: a
    /// write settled or reverted, or one left `sending` was settled. A
    /// write the device refused leaves the ledger as it was.
    pub(crate) fn ledger_settled(&self, before: &[Entry], desk: &Desk) {
        if desk.entries() != before {
            self.ledger_changed();
        }
    }
}

/// Announce the session when its armed session or its plan grant runs
/// out, until the server closes: no request arrives to mark either. The
/// plan's end, a wall-clock time, is turned into the process's clock
/// once, at the start.
pub(crate) async fn follow_ends<T: TransportPort + 'static>(ctx: &Ctx<T>) {
    let mut changed = ctx.writes.signals.session();
    let mut plan_end = ctx.policy.writes.plan.as_ref().and_then(|plan| {
        let left = plan.expires_unix.checked_sub(unix_now())?;
        (left > 0).then(|| Instant::now() + Duration::from_secs(left))
    });
    loop {
        let session_end = ctx.writes.armed_until();
        let next = match (session_end, plan_end) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
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
                if plan_end.is_some_and(|end| end <= Instant::now()) {
                    plan_end = None;
                }
                ctx.writes.signals.session_changed();
            }
        }
    }
}
