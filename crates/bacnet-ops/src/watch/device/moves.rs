//! Where a watched device is said to have gone (#2812). A move the hub
//! didn't see answered decides nothing: a handle following the device's
//! instance that moved away from the address (on the discovery table's
//! rows, which I-Ams nobody asked for fill, or by its own rule after a
//! timeout), or a new address the scheduler's own look found after one of
//! its requests went unanswered, is only a cue. The device at the new
//! address is asked which it is, by one read of its Device object by the
//! wildcard instance through its gate and the client's rate, and the points
//! are given up as moved there ([`Why::Moved`]) only when that answer names
//! their instance. Another instance, no answer, or a device that refuses
//! the wildcard read (then never asked so again) changes nothing, and the
//! cue is spent: a later one is read in its turn. The reads keep a
//! [`Pace`] of their own, as a claim's asks do: the first at once, the
//! next not within 60 s of it, the wait doubling up to the subscription
//! lifetime. A cue meanwhile waits for its turn, the latest kept; one for
//! the instance and address being read is that read. A handle that moved
//! stays where it moved, whatever the read finds.

use std::sync::Arc;

use bacnet_transport::port::TransportPort;
use tokio::time::Instant;

use super::super::{Why, jobs};
use super::identity::{Pace, give_up};
use super::{Device, Done, Job, Mode, Op, Row};
use crate::device::DeviceAddress;
use crate::limits::charged;

/// The reads at the addresses a scheduler's device is said to have moved
/// to: an instance, and the address.
#[derive(Debug, Default)]
pub(super) struct Moves {
    /// The cue being read.
    reading: Option<(u32, DeviceAddress)>,
    /// The latest cue waiting for its turn.
    waiting: Option<(u32, DeviceAddress)>,
    pace: Pace,
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

impl<T: TransportPort + 'static> Device<T> {
    /// The device of `instance` is said to answer at `to` now: ask the one
    /// there which it is, now or in its turn, unless no point would change.
    pub(super) fn cue_move(&mut self, instance: u32, to: DeviceAddress) {
        let moved = Some(Why::Moved { to: to.clone() });
        let changes = self
            .rows
            .values()
            .any(|row| goes(row, instance) && row.shared.lock().snapshot.why != moved);
        let cue = (instance, to);
        if !changes || self.moves.reading.as_ref() == Some(&cue) {
            return;
        }
        let now = Instant::now();
        if self.moves.reading.is_some() || !self.moves.pace.ready(now) {
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
            && self.moves.pace.ready(now)
            && let Some((instance, to)) = self.moves.waiting.take()
        {
            self.cue_move(instance, to);
        }
    }

    /// The read of which device answers at `to`, for a move of `instance`
    /// there, charged to the hub as any request.
    pub(super) fn arrival_job(&self, instance: u32, to: DeviceAddress) -> Job {
        let inner = Arc::clone(&self.inner);
        let account = Arc::clone(&inner.account);
        Box::pin(async move {
            let found = charged(account, jobs::name(inner, to.clone())).await;
            Done::Arrived(instance, to, found)
        })
    }

    /// The device at `to`, read for a move of `instance` there, named
    /// itself `found` (`None`: no answer, or it refuses the read): the
    /// points go when it is that instance.
    pub(super) fn arrived(&mut self, instance: u32, to: DeviceAddress, found: Option<u32>) {
        self.moves.reading = None;
        match found {
            Some(found) if found == instance => self.moved(instance, to),
            Some(found) => tracing::debug!(
                "device {instance} said to answer at {to}: device {found} answers there; no move"
            ),
            None => tracing::debug!(
                "device {instance} said to answer at {to}: no answer naming a device there; no move"
            ),
        }
    }

    /// The device of `instance` answers at `to` now, as an answer there
    /// named it: give its points up (those of another instance stay),
    /// stopping their renewals and sending nothing more for them, each
    /// noting the instance it was given up for; a point given up for that
    /// instance already, its address taken by another device, now says
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
