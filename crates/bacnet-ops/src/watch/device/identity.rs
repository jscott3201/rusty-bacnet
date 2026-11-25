//! Which device a point's values come from (#2603).
//!
//! A point's device is the instance its address named when its watch
//! started: the discovery table's row there, else an instance read there
//! earlier by the wildcard, else one read so once the first read was
//! answered, else the first one announced there. A start, poll or verify
//! reads the table at the address once answered, and each announcement
//! there names an instance. When the address names another instance than a
//! point's, the point is given up ([`Why::Replaced`]): what the address sends
//! is not taken as the point's any more (the receiver checks the instance
//! the address names before each notification), its renewals stop and
//! nothing more is sent for it, cancels included, since the new device
//! never held its subscription. Its instance is looked for elsewhere, and
//! found at one other address the point is reported [`Why::Moved`] there;
//! announced at its own address again, it starts again. A point started at
//! the address later takes the instance there then, so it watches the new
//! device.
//!
//! The device announcing itself again (an I-Am at its address, for its own
//! instance) may have restarted and lost its subscriptions: its subscribed
//! points are read and subscribed again at once, and those polled after a
//! failure are tried again, rather than at their next renewal or retry. At
//! most once a lifetime a device ([`WatchOptions::lifetime`]): a device that
//! announces itself every few seconds, or answers every Who-Is, costs at
//! most one more read and subscription a point a lifetime, no more than its
//! renewals, and a second restart within that lifetime is subscribed again
//! by the renewal, as every restart was before. The starts queue on the
//! scheduler's one job at a time and go through the device's gate, the
//! client's rate and the hub's own, so many devices announcing at once are
//! subscribed again no faster than the limits allow.
//!
//! When announcements may have been missed (the receiver fell behind, or
//! more came between two looks than are kept), the scheduler reads the
//! discovery table at the address before its next job, which sends
//! nothing. Everything here runs on the scheduler's task without awaiting:
//! the reads of the table are the jobs' own.
//!
//! [`WatchOptions::lifetime`]: super::super::hub::WatchOptions::lifetime

use std::collections::BTreeSet;
use std::sync::atomic::AtomicU64;

use bacnet_transport::port::TransportPort;
use tokio::time::Instant;

use super::super::Why;
use super::super::jobs::Started;
use super::{Device, Mode, Op, Row};
use crate::device::DeviceAddress;

/// Whether `row` is watched and not starting: a start in flight decides by
/// its own answer.
fn live(row: &Row) -> bool {
    !row.removing && !matches!(row.mode, Mode::Gone | Mode::Starting)
}

impl<T: TransportPort + 'static> Device<T> {
    /// Take in what was announced at the address since the last look.
    pub(super) fn heard(&mut self) {
        let (instances, missed) = self.slot.take_heard();
        for found in instances {
            self.announced(found);
        }
        if missed && !self.verifying {
            self.verifying = true;
            self.ops.push_front(Op::Verify);
        }
    }

    /// A device at the address announced itself as `found`: the points
    /// given up for that instance start again, those of another are given
    /// up, those of no known instance take it, and the rest are subscribed
    /// again ([`restart`](Self::restart)).
    fn announced(&mut self, found: u32) {
        self.answered_for(Some(found));
        for row in self.rows.values() {
            if live(row) && !matches!(row.mode, Mode::Queued) {
                row.shared.learn(Some(found));
            }
        }
        self.check(found);
        self.restart();
    }

    /// The address names `found` now: give up the points of every other
    /// instance.
    pub(super) fn check(&mut self, found: u32) {
        let others: BTreeSet<u32> = self
            .rows
            .values()
            .filter(|row| live(row))
            .filter_map(|row| row.shared.identity())
            .filter(|watched| *watched != found)
            .collect();
        for watched in others {
            self.replaced(watched, found);
        }
    }

    /// Device `found` answers at the address now, not `watched`: give up
    /// `watched`'s points and look for it elsewhere.
    fn replaced(&mut self, watched: u32, found: u32) {
        let at = &self.address;
        tracing::info!(
            "device {found} answers at {at} now, not device {watched}; its watches stop"
        );
        let note = format!(
            "device {found} answers at {at} now, not device {watched}; \
             nothing from there is taken as this point's"
        );
        let seq = &self.inner.registry.seq;
        let mut gone = Vec::new();
        for (process, row) in &mut self.rows {
            if live(row) && row.shared.identity() == Some(watched) {
                let why = Why::Replaced { watched, found };
                give_up(row, why, note.clone(), seq, watched);
                gone.push(*process);
            }
        }
        self.ops
            .retain(|op| !matches!(op, Op::Start(process) if gone.contains(process)));
        if !gone.is_empty() {
            self.locate(watched);
        }
    }

    /// Subscribe the points again: the device announced itself, as one
    /// does on starting up. A subscription held is dropped, so it stops
    /// renewing (the start's own takes its place on the device, which
    /// keeps one a subscriber and object), and may still be cancelled. At
    /// most once a lifetime (see the module's notes).
    fn restart(&mut self) {
        let now = Instant::now();
        let lifetime = self.inner.options.lifetime.duration();
        if self
            .restarted
            .is_some_and(|at| now.saturating_duration_since(at) < lifetime)
        {
            return;
        }
        let mut again = Vec::new();
        for (process, row) in &mut self.rows {
            if row.removing || row.shared.is_stopped() {
                continue;
            }
            match row.mode {
                Mode::Cov(..) => row.maybe_subscribed = true,
                Mode::Polled | Mode::Waiting if row.resubscribe.is_some() => {}
                _ => continue,
            }
            row.mode = Mode::Queued;
            again.push(*process);
        }
        if again.is_empty() {
            return;
        }
        tracing::info!(
            "device at {} announced itself; subscribing its {} points again",
            self.address,
            again.len()
        );
        self.restarted = Some(now);
        self.ops.extend(again.into_iter().map(Op::Start));
    }

    /// The device of `instance` answers at `to` now: give its points up
    /// (those of another instance stay), stopping their renewals and
    /// sending nothing more for them, each noting the instance it was given
    /// up for; a point given up for that instance already, its address
    /// taken by another device, now says where it went. A point whose start
    /// is running decides by its own answer.
    pub(super) fn moved(&mut self, instance: u32, to: DeviceAddress) {
        tracing::info!(
            "device {instance} at {} answers at {to} now; its watches stop",
            self.address
        );
        let seq = &self.inner.registry.seq;
        let note = format!("the device answers at {to} now; watch it there");
        let mut gone = Vec::new();
        for (process, row) in &mut self.rows {
            let theirs = row.shared.identity().is_none_or(|known| known == instance);
            let live = match row.mode {
                Mode::Starting => false,
                Mode::Gone => row.moved == Some(instance),
                _ => true,
            };
            if theirs && live && !row.removing {
                let why = Why::Moved { to: to.clone() };
                give_up(row, why, note.clone(), seq, instance);
                gone.push(*process);
            }
        }
        self.ops
            .retain(|op| !matches!(op, Op::Start(process) if gone.contains(process)));
    }

    /// The device at the address answered, or announced itself, for
    /// `instance`: the points given up for that instance start again, and
    /// those given up for another stay.
    pub(super) fn answered_for(&mut self, instance: Option<u32>) {
        let Some(instance) = instance else {
            return;
        };
        let seq = &self.inner.registry.seq;
        let mut revived = false;
        for (process, row) in &mut self.rows {
            if row.moved == Some(instance) && matches!(row.mode, Mode::Gone) && !row.removing {
                row.moved = None;
                row.mode = Mode::Queued;
                row.shared.revive(seq);
                self.ops.push_back(Op::Start(*process));
                revived = true;
            }
        }
        if revived {
            tracing::info!(
                "device {instance} answers at {} again; its watches start again",
                self.address
            );
        }
    }

    /// What the table named at the address after announcements there were
    /// missed.
    pub(super) fn verified(&mut self, instance: Option<u32>, generation: u64) {
        self.verifying = false;
        if let Some(found) = instance {
            self.slot.confirm(generation, found);
            self.check(found);
        }
    }

    /// Whether `started`, the start of `process`, was answered by another
    /// device than the point's: its address names another instance now
    /// than the one the point's watch started with. Then nothing of the
    /// start is the point's: its first read isn't taken, a subscription it
    /// made lapses unrenewed, and the point is given up with the others of
    /// its instance.
    pub(super) fn started_elsewhere(&mut self, process: u32, started: &Started) -> bool {
        let Some(row) = self.rows.get_mut(&process) else {
            return false;
        };
        let identity = row.shared.learn(started.before.or(started.instance));
        let (Some(watched), Some(found)) = (identity, started.instance) else {
            return false;
        };
        if watched == found {
            return false;
        }
        if row.removing || row.shared.is_stopped() {
            let row = self.rows.remove(&process).expect("looked up above");
            self.finish(row);
            return true;
        }
        // No longer starting: given up below with its instance's others.
        row.mode = Mode::Polled;
        self.check(found);
        true
    }
}

/// Give `row` up `why`, for the device of `instance`: its managed
/// subscription, dropped, stops renewing, and nothing more is sent for it,
/// cancels included, so what its address may hold lapses with its
/// lifetime. It starts again once its address answers for `instance`.
pub(super) fn give_up(row: &mut Row, why: Why, note: String, seq: &AtomicU64, instance: u32) {
    row.mode = Mode::Gone;
    row.moved = Some(instance);
    row.maybe_subscribed = false;
    row.resubscribe = None;
    row.shared.gone(seq, why, note);
}
