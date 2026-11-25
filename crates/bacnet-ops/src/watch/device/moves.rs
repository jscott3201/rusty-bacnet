//! Where a watched device is said to have gone (#2812). A move the hub
//! didn't see answered decides nothing: a handle following the device's
//! instance that moved away from the address (on the discovery table's
//! rows, which I-Ams nobody asked for fill, or by its own rule after a
//! timeout), or a new address the scheduler's own look found after one of
//! its requests went unanswered, is only a cue. The device at the new
//! address is asked which it is, by one read of its Device object by the
//! wildcard instance through its gate and the client's rate. Another
//! instance, no answer, or a device that refuses the wildcard read (then
//! never asked so again) changes nothing, and the cue is spent: a later one
//! is read in its turn.
//!
//! An answer naming the points' device there is a move only once the
//! device is gone from the address: two addresses answering for it are two
//! claimants (a duplicate, or a responder on an impostor's own host), and
//! nothing changes. So, unless the address is failing already (in its
//! backoff, or a point there unreachable) or none of its points is watched
//! any more, the device at the address is asked once more which it is, as
//! a claim is checked: still the device, the watch goes on (logged once a
//! lifetime); silent, without that device's Device object, or answering
//! for it with another identifier, the points are given up as moved; named
//! another device, they are given up for it first ([`Why::Replaced`]),
//! then say where their own went. An answer that says neither, or the
//! client's own failure, changes nothing. The answer is taken when it
//! comes, however long the cue waited for its turn.
//!
//! The reads keep a [`Pace`] of their own, as a claim's asks do, and it is
//! the watched address's, whichever addresses are named: the first at
//! once, the next not within 60 s of it, the wait doubling up to the
//! subscription lifetime; the ask at the address rides with the read that
//! led to it. A cue meanwhile waits for its turn in one slot, the latest
//! kept; one for the instance and address being read is that read. So
//! forged cues can hold a real move's read to one a lifetime, or take its
//! slot; the scheduler's own look after each unanswered poll cues the real
//! address again. A handle that moved stays where it moved, whatever the
//! reads find, and the hub's own look ([`Client::relocated`]) still moves
//! a following handle before anything is read there (for the hub that
//! follows devices, #2795).
//!
//! [`Client::relocated`]: crate::Client

use std::sync::Arc;

use bacnet_transport::port::TransportPort;
use tokio::time::Instant;

use super::super::Why;
use super::super::hub::Phase;
use super::super::jobs::{self, Answer};
use super::identity::{Pace, give_up};
use super::{Device, Done, Job, Mode, Op, Row};
use crate::device::DeviceAddress;
use crate::limits::charged;

/// The reads at the addresses a scheduler's device is said to have moved
/// to, each an instance and an address.
#[derive(Debug, Default)]
pub(super) struct Moves {
    /// The cue being read.
    reading: Option<(u32, DeviceAddress)>,
    /// A cue the new address bore out, while the device at the old one is
    /// asked whether it is still there.
    asking: Option<(u32, DeviceAddress)>,
    /// The latest cue waiting for its turn.
    waiting: Option<(u32, DeviceAddress)>,
    pace: Pace,
    /// When two claimants were last logged as a warning.
    warned: Option<Instant>,
}

/// Whether `row` goes with its device, of `instance`, when that has moved:
/// a point of it (or of no device known) that is watched, or one given up
/// for it. A point whose start is running decides by its own answer.
fn goes(row: &Row, instance: u32) -> bool {
    let theirs = row.shared.identity().is_none_or(|known| known == instance);
    let live = match row.mode {
        Mode::Starting => false,
        Mode::Gone => row.moved == Some(instance),
        _ => true,
    };
    theirs && live && !row.removing
}

/// Whether `row` is a watched point of the device of `instance` (or of no
/// device known), not given up.
fn watches(row: &Row, instance: u32) -> bool {
    let theirs = row.shared.identity().is_none_or(|known| known == instance);
    theirs && !row.removing && !matches!(row.mode, Mode::Gone)
}

impl<T: TransportPort + 'static> Device<T> {
    /// The device of `instance` is said to answer at `to` now: ask the one
    /// there which it is, now or in its turn, unless no point would change.
    pub(super) fn cue_move(&mut self, instance: u32, to: DeviceAddress) {
        let cue = (instance, to);
        let known = [&self.moves.reading, &self.moves.asking].contains(&&Some(cue.clone()));
        if known || !self.changes(cue.0, &cue.1) {
            return;
        }
        let now = Instant::now();
        let busy = self.moves.reading.is_some() || self.moves.asking.is_some();
        if busy || !self.moves.pace.ready(now) {
            self.moves.waiting = Some(cue);
            return;
        }
        self.moves.pace.spend(now, self.lifetime());
        self.ops.push_front(Op::Arrived(cue.0, cue.1.clone()));
        self.moves.reading = Some(cue);
    }

    /// Read the cue waiting, once its turn has come by `now`.
    pub(super) fn moves_due(&mut self, now: Instant) {
        if self.moves.reading.is_none()
            && self.moves.asking.is_none()
            && self.moves.pace.ready(now)
            && let Some((instance, to)) = self.moves.waiting.take()
        {
            self.cue_move(instance, to);
        }
    }

    /// Whether giving the points up as moved to `to` for `instance` would
    /// change any.
    fn changes(&self, instance: u32, to: &DeviceAddress) -> bool {
        let moved = Some(Why::Moved { to: to.clone() });
        self.rows
            .values()
            .any(|row| goes(row, instance) && row.shared.lock().snapshot.why != moved)
    }

    /// The read of which device answers at `to`, for a move of `instance`
    /// there, charged to the hub as any request; none when no point would
    /// change by now (its watches dropped meanwhile, say).
    pub(super) fn arrival_job(&mut self, instance: u32, to: DeviceAddress) -> Option<Job> {
        if !self.changes(instance, &to) {
            self.moves.reading = None;
            return None;
        }
        let inner = Arc::clone(&self.inner);
        let account = Arc::clone(&inner.account);
        Some(Box::pin(async move {
            let found = charged(account, jobs::name(inner, to.clone())).await;
            Done::Arrived(instance, to, found)
        }))
    }

    /// The device at `to`, read for a move of `instance` there, named
    /// itself `found` (`None`: no answer, or it refuses the read): when it
    /// is that instance, the move stands once the device at the address,
    /// asked again unless it is failing, isn't that one.
    pub(super) fn arrived(&mut self, instance: u32, to: DeviceAddress, found: Option<u32>) {
        self.moves.reading = None;
        match found {
            Some(found) if found == instance => {}
            Some(found) => {
                tracing::debug!(
                    "device {instance} said to answer at {to}: device {found} answers there; no move"
                );
                return;
            }
            None => {
                tracing::debug!(
                    "device {instance} said to answer at {to}: no answer naming a device there; no move"
                );
                return;
            }
        }
        let watched = self.rows.values().any(|row| watches(row, instance));
        if watched && !self.failing() {
            self.ops.push_front(Op::Left(instance));
            self.moves.asking = Some((instance, to));
        } else {
            self.moved(instance, to);
        }
    }

    /// Whether the device at the address is failing: in its backoff, or a
    /// point there unreachable.
    fn failing(&self) -> bool {
        self.backing_off()
            || self
                .rows
                .values()
                .any(|row| row.shared.lock().snapshot.state == Phase::Unreachable)
    }

    /// The ask of whether the device of `instance` is still at the address,
    /// as a claim is checked; none when no point of it is watched by now,
    /// and then the points given up for it say where it went.
    pub(super) fn left_job(&mut self, instance: u32) -> Option<Job> {
        if !self.rows.values().any(|row| watches(row, instance)) {
            if let Some((instance, to)) = self.moves.asking.take() {
                self.moved(instance, to);
            }
            return None;
        }
        let inner = Arc::clone(&self.inner);
        let account = Arc::clone(&inner.account);
        let address = self.address.clone();
        Some(Box::pin(async move {
            let answer = charged(account, jobs::confirm(inner, address, instance)).await;
            Done::Left(instance, answer)
        }))
    }

    /// What the device at the address answered, asked whether the device
    /// of `instance` is still there, another address having named itself
    /// so.
    pub(super) fn left(&mut self, instance: u32, answer: Answer) {
        let Some((_, to)) = self.moves.asking.take() else {
            return;
        };
        match answer {
            Answer::Is(found) | Answer::Has(found) if found == instance => {
                self.claimants(instance, &to)
            }
            Answer::Is(found) => {
                self.settle(found, None);
                self.moved(instance, to);
            }
            Answer::Has(_) | Answer::Not | Answer::Silent => self.moved(instance, to),
            Answer::Unclear | Answer::Failed => tracing::debug!(
                "device {instance} answers at {to}; the device at {} couldn't say whether it is \
                 still there; no move",
                self.address
            ),
        }
    }

    /// The device of `instance` answers both at the address and at `to`:
    /// the watch goes on, logged as a warning once a lifetime.
    fn claimants(&mut self, instance: u32, to: &DeviceAddress) {
        let now = Instant::now();
        let lifetime = self.lifetime();
        let warn = self.moves.warned.is_none_or(|at| now >= at + lifetime);
        let at = &self.address;
        if warn {
            self.moves.warned = Some(now);
            tracing::warn!(
                "device {instance} answers both at {at} and at {to}: two claimants; its watches \
                 stay at {at}"
            );
        } else {
            tracing::debug!("device {instance} answers both at {at} and at {to}; no move");
        }
    }

    /// The device of `instance` answers at `to` now, as answers there and
    /// at the address showed: give its points up (those of another instance
    /// stay), stopping their renewals and sending nothing more for them,
    /// each noting the instance it was given up for; a point given up for
    /// that instance already, its address taken by another device, now says
    /// where it went.
    fn moved(&mut self, instance: u32, to: DeviceAddress) {
        tracing::info!(
            "device {instance} at {} answers at {to} now; its watches stop",
            self.address
        );
        let seq = &self.inner.registry.seq;
        let note = format!("the device answers at {to} now; watch it there");
        let mut gone = Vec::new();
        for (process, row) in &mut self.rows {
            if goes(row, instance) {
                let why = Why::Moved { to: to.clone() };
                give_up(row, why, note.clone(), seq, instance);
                gone.push(*process);
            }
        }
        self.ops
            .retain(|op| !matches!(op, Op::Start(process) if gone.contains(process)));
    }
}
