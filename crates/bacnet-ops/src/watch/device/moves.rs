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
//! nothing changes. So the device at the address is asked once more which
//! it is, as a claim is checked, whatever the address looked like before:
//! a renewal or a poll may fail for many reasons, and a routed address's
//! router reject can be forged. It isn't asked when no route to the
//! address is known, when none of its points is watched any more, or when
//! the cue came from the scheduler's own look and the address is failing
//! (in its backoff, or a point there unreachable), as the look followed a
//! request of the scheduler's own that went unanswered there. Asked, still
//! the device, the watch goes on (a warning once a lifetime for each
//! address it is also said to answer at); its request timing out (no other
//! failure counts), without that device's Device object, or answering for
//! it with another identifier, the points are given up as moved; named
//! another device, they are given up for it first ([`Why::Replaced`]),
//! then say where their own went. Any other answer, a router's reject, or
//! the client's own failure changes nothing. The answer is taken when it
//! comes, however long the cue waited for its turn.
//!
//! Points given up already (for another device at the address) aren't
//! watched there, so the address isn't asked: any device naming their
//! instance at the new address has them say it went there, at most once a
//! turn of the pace. That is a hint for an application watching it there,
//! not a confirmed device.
//!
//! The reads keep a [`Pace`] of their own, as a claim's asks do, and it is
//! the watched address's, whichever addresses are named: the first at
//! once, the next not within 60 s of it, the wait doubling up to the
//! subscription lifetime; the ask at the address rides with the read that
//! led to it. A cue meanwhile waits for its turn in one slot, the latest
//! kept; one for the instance and address being read or asked about is
//! that one. So forged cues can hold a real move's read to one a lifetime,
//! or take its slot; the scheduler's own look after each unanswered poll
//! cues the real address again. A handle that moved stays where it moved,
//! whatever the reads find; the hub's own look ([`Client::relocated`])
//! moves no handle, but a move the reads bore out, the address asked once
//! more, takes the handles following the instance with the hub's
//! following points (#2795).
//!
//! When the reads bear a move out, the pinned points are given up
//! ([`Why::Moved`]) and the following ones go to the new address
//! ([`follow`](super::follow)).
//!
//! [`Client::relocated`]: crate::Client

use std::collections::HashMap;
use std::sync::Arc;

use bacnet_transport::port::TransportPort;
use tokio::time::Instant;

use super::super::hub::Phase;
use super::super::jobs::{self, Answer};
use super::super::{Binding, Why};
use super::follow::follows_to;
use super::identity::{Pace, give_up};
use super::{Device, Done, Job, Mode, Op, Row};
use crate::device::DeviceAddress;
use crate::limits::charged;

/// The most addresses whose two claimants a scheduler remembers having
/// logged as a warning; past it, the one logged longest ago is forgotten.
const WARNED: usize = 16;

/// A move heard of: the instance, where it is said to answer now, and
/// whether the scheduler's own look found it there (else a handle moved).
#[derive(Debug, Clone)]
pub(super) struct Cue {
    instance: u32,
    to: DeviceAddress,
    looked: bool,
}

impl Cue {
    fn is(&self, instance: u32, to: &DeviceAddress) -> bool {
        self.instance == instance && self.to == *to
    }
}

/// The reads at the addresses a scheduler's device is said to have moved
/// to.
#[derive(Debug, Default)]
pub(super) struct Moves {
    /// The cue being read.
    reading: Option<Cue>,
    /// A cue the new address bore out, while the device at the old one is
    /// asked whether it is still there.
    asking: Option<Cue>,
    /// The latest cue waiting for its turn.
    waiting: Option<Cue>,
    pace: Pace,
    /// When two claimants were last logged as a warning, by instance and
    /// the other address.
    warned: HashMap<(u32, DeviceAddress), Instant>,
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
    /// The device of `instance` is said to answer at `to` now, as the
    /// scheduler's own look found (`looked`) or a handle moved: ask the one
    /// there which it is, now or in its turn, unless no point would change.
    pub(super) fn cue_move(&mut self, instance: u32, to: DeviceAddress, looked: bool) {
        let under_way = [&mut self.moves.reading, &mut self.moves.asking];
        if let Some(cue) = under_way
            .into_iter()
            .flatten()
            .find(|c| c.is(instance, &to))
        {
            cue.looked |= looked;
            return;
        }
        if !self.changes(instance, &to) {
            return;
        }
        let cue = match self.moves.waiting.take() {
            Some(waiting) if waiting.is(instance, &to) => Cue {
                looked: waiting.looked || looked,
                ..waiting
            },
            _ => Cue {
                instance,
                to,
                looked,
            },
        };
        let now = Instant::now();
        let busy = self.moves.reading.is_some() || self.moves.asking.is_some();
        if busy || !self.moves.pace.ready(now) {
            self.moves.waiting = Some(cue);
            return;
        }
        self.moves.pace.spend(now, self.lifetime());
        self.ops
            .push_front(Op::Arrived(cue.instance, cue.to.clone()));
        self.moves.reading = Some(cue);
    }

    /// Read the cue waiting, once its turn has come by `now`.
    pub(super) fn moves_due(&mut self, now: Instant) {
        if self.moves.reading.is_none()
            && self.moves.asking.is_none()
            && self.moves.pace.ready(now)
            && let Some(cue) = self.moves.waiting.take()
        {
            self.cue_move(cue.instance, cue.to, cue.looked);
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
    /// asked again unless that can't tell more, isn't that one.
    pub(super) fn arrived(&mut self, instance: u32, to: DeviceAddress, found: Option<u32>) {
        let cue = self.moves.reading.take();
        let looked = cue.is_some_and(|cue| cue.is(instance, &to) && cue.looked);
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
        let failed = looked && self.failing();
        if watched && !failed && !self.unrouted(instance) {
            self.ops.push_front(Op::Left(instance));
            self.moves.asking = Some(Cue {
                instance,
                to,
                looked,
            });
        } else {
            // No point of it watched here: given up for another device the
            // hub's ask found here, the address was asked; given up for no
            // word of the device's (unconfirmed), or here failing for the
            // hub's own request, or no route here: nothing asked.
            let asked = !watched && self.replaced_here(instance);
            self.moved(instance, to, asked);
        }
    }

    /// Whether a point of `instance` here was given up for another device
    /// the hub's ask found answering here: this address was asked, and
    /// isn't the device. A point given up as unconfirmed is no such word.
    fn replaced_here(&self, instance: u32) -> bool {
        self.rows.values().any(|row| {
            matches!(
                row.shared.lock().snapshot.why,
                Some(Why::Replaced { watched, .. }) if watched == instance
            )
        })
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

    /// Whether no route to the address is known for the points of
    /// `instance`: nothing could be sent there.
    fn unrouted(&self, instance: u32) -> bool {
        self.rows
            .values()
            .any(|row| watches(row, instance) && matches!(row.mode, Mode::Waiting))
    }

    /// The ask of whether the device of `instance` is still at the address,
    /// as a claim is checked; none when no point of it is watched by now,
    /// and then the points given up for it say where it went.
    pub(super) fn left_job(&mut self, instance: u32) -> Option<Job> {
        if !self.rows.values().any(|row| watches(row, instance)) {
            if let Some(cue) = self.moves.asking.take() {
                let asked = self.replaced_here(cue.instance);
                self.moved(cue.instance, cue.to, asked);
            }
            return None;
        }
        let inner = Arc::clone(&self.inner);
        let account = Arc::clone(&inner.account);
        let address = self.address.clone();
        Some(Box::pin(async move {
            let answer = charged(account, jobs::still_there(inner, address, instance)).await;
            Done::Left(instance, answer)
        }))
    }

    /// What the device at the address answered, asked whether the device
    /// of `instance` is still there, another address having named itself
    /// so.
    pub(super) fn left(&mut self, instance: u32, answer: Answer) {
        let Some(Cue { to, .. }) = self.moves.asking.take() else {
            return;
        };
        match answer {
            Answer::Is(found) | Answer::Has(found) if found == instance => {
                self.claimants(instance, &to)
            }
            Answer::Is(found) => {
                self.settle(found, None);
                self.moved(instance, to, true);
            }
            Answer::Has(_) | Answer::Not | Answer::Silent => self.moved(instance, to, true),
            Answer::Unclear | Answer::Failed => tracing::debug!(
                "device {instance} answers at {to}; the device at {} couldn't say whether it is \
                 still there; no move",
                self.address
            ),
        }
    }

    /// The device of `instance` answers both at the address and at `to`:
    /// the watch goes on, logged as a warning once a lifetime for each
    /// instance and other address.
    fn claimants(&mut self, instance: u32, to: &DeviceAddress) {
        let now = Instant::now();
        let lifetime = self.lifetime();
        let warned = &mut self.moves.warned;
        let key = (instance, to.clone());
        let at = &self.address;
        if warned.get(&key).is_some_and(|last| now < *last + lifetime) {
            tracing::debug!("device {instance} answers both at {at} and at {to}; no move");
            return;
        }
        warned.retain(|_, last| now < *last + lifetime);
        if warned.len() >= WARNED
            && let Some(oldest) = warned.iter().min_by_key(|(_, last)| **last)
        {
            let oldest = oldest.0.clone();
            warned.remove(&oldest);
        }
        warned.insert(key, now);
        tracing::warn!(
            "device {instance} answers both at {at} and at {to}: two claimants; its watches stay \
             at {at}"
        );
    }

    /// A following point's start found a device at the address that
    /// refuses the read that names it (#2795): nothing there can confirm
    /// it, so nothing it answered is taken or subscribed, and the point is
    /// unreachable as unconfirmed. It goes with its device if the hub's
    /// reads find it at another address.
    pub(super) fn unconfirmed(&mut self, process: u32) {
        let seq = &self.inner.registry.seq;
        let at = &self.address;
        let Some(row) = self.rows.get_mut(&process) else {
            return;
        };
        let Some(instance) = row.shared.point.device.instance() else {
            return;
        };
        if row.removing || row.shared.is_stopped() {
            let row = self.rows.remove(&process).expect("looked up above");
            self.finish(row);
            return;
        }
        let note = format!(
            "the device at {at} refuses the read that names it, so device {instance} can't be \
             confirmed there; watch it by its address"
        );
        give_up(row, Why::Unconfirmed, note, seq, instance);
    }

    /// The device of `instance` answers at `to` now, as answers there and
    /// at the address showed. Its pinned points are given up (those of
    /// another instance stay), stopping their renewals and sending nothing
    /// more for them, each noting the instance it was given up for; a point
    /// given up for that instance already, its address taken by another
    /// device, now says where it went. Its following points go to `to`
    /// ([`follow`](super::follow)), the client's handles with them when
    /// this address was `asked` (#2795). No start runs now: the scheduler
    /// runs one job at a time, and this is another's end.
    fn moved(&mut self, instance: u32, to: DeviceAddress, asked: bool) {
        tracing::info!(
            "device {instance} at {} answers at {to} now; its pinned watches stop and its \
             following ones go there",
            self.address
        );
        let seq = &self.inner.registry.seq;
        let note = format!("the device answers at {to} now; watch it there");
        let mut gone = Vec::new();
        let mut going = Vec::new();
        let mut keys = Vec::new();
        for (process, row) in &mut self.rows {
            if !goes(row, instance) {
                continue;
            }
            if follows_to(&row.shared, instance, &to) {
                going.push(*process);
                if let Binding::Follows { instance, network } = row.shared.point.device {
                    keys.push(self.inner.client.key(network, instance));
                }
            } else {
                let why = Why::Moved { to: to.clone() };
                give_up(row, why, note.clone(), seq, instance);
                gone.push(*process);
            }
        }
        self.ops.retain(|op| {
            !matches!(op, Op::Start(process) if gone.contains(process) || going.contains(process))
        });
        keys.sort_unstable();
        keys.dedup();
        #[cfg(test)]
        {
            let hook = self.inner.registry.before_move.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
        }
        self.follow_to((instance, keys), going, &to, asked);
    }
}

#[cfg(test)]
mod tests;
