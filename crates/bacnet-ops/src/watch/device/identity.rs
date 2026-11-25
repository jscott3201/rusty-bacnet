//! Which device a point's values come from, and when it restarted (#2603).
//!
//! A point's device is the instance its address named when its watch
//! started: the discovery table's row there, else an instance read there
//! earlier by the wildcard, else one read so once the first read was
//! answered, else the first one announced there. Every COV notification
//! names the device that sent it, and the receiver drops one naming another
//! than its point's. A poll by ReadPropertyMultiple asks the device which it
//! is in the same request. Those are the device's own word: a poll naming
//! another instance gives the points of the old one up ([`Why::Replaced`])
//! at once. Anything else saying another device is at the address (an
//! announcement there, the table once a start or poll there is answered, a
//! notification naming another sender) is a claim: before a point is given
//! up for it, or one given up comes back, the device there is asked which
//! it is, by one read of its Device object (by the wildcard instance, or,
//! for a device that refused that, by the watched one). It answers as the
//! watched device: the claim is ignored, and a sender its notifications
//! named is taken as its own from then on (a gateway naming itself, say).
//! It answers as another: the points are given up. It doesn't answer: the
//! device is unreachable, as for any unanswered request. One such read at
//! most a lifetime ([`WatchOptions::lifetime`]) an address, revivals
//! included, so a stray or forged announcement, or two devices fighting
//! over an address, cost one read a lifetime: a later claim waits for it.
//! A given-up point's renewals stop and nothing more is sent for it, cancels
//! included; its values are dropped, as they may be the other device's. Its
//! instance announcing itself at another address has its scheduler look
//! there, sending nothing, and found there alone the point is reported
//! [`Why::Moved`]. A point started at the address later watches the device
//! there then. Not caught: a device that never announces itself taking the
//! address of a point polled by ReadProperty alone.
//!
//! A device announcing itself again for its own instance may have restarted
//! and lost its subscriptions. Its Time_Of_Device_Restart is read then (one
//! request, at most once a lifetime): when it moved since the last read,
//! the subscribed points are read and subscribed again, and those polled
//! after a failure tried again; unchanged, nothing is sent. The first read
//! only learns it, and a device without the property is taken as maybe
//! restarted on the announcement alone: subscribed again at most once a
//! lifetime counted from its first subscription, so a Who-Is answered once
//! a lifetime costs such a device one more read and subscription a point a
//! lifetime, and a device with the property one read. A restart
//! notification the device sends (Clause 19.3) is taken at its word. Every
//! check and re-subscription waits a part of [`SPREAD`] set by the address,
//! so devices answering one Who-Is don't ask at once, and goes through the
//! device's gate and the client's rate.
//!
//! When announcements may have been missed, the scheduler reads the table
//! at the address, which sends nothing. Nothing here awaits: the reads are
//! jobs, which the scheduler polls beside its wake-up and its tick.
//!
//! [`WatchOptions::lifetime`]: super::super::hub::WatchOptions::lifetime

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bacnet_transport::port::TransportPort;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::time::Instant;

use super::super::Why;
use super::super::jobs::{self, Answer, Outcome, Polled, Restart, Started};
use super::{Device, Done, Job, Mode, Op, Row};
use crate::device::{DeviceAddress, QUIET};
use crate::limits::charged;

/// The longest a check or re-subscription waits after an announcement: a
/// part of it set by the device's address, so devices answering one Who-Is
/// are asked one after another, not together.
pub(super) const SPREAD: Duration = Duration::from_secs(10);

/// A check a scheduler runs as a job.
pub(super) enum Check {
    /// Read the discovery table at the address. Sends nothing.
    Table,
    /// Ask the device at the address which instance it is: another,
    /// `claimed`, was said to be there while the points watch `watched`.
    Confirm { watched: u32, claimed: u32 },
    /// Read device `instance`'s Time_Of_Device_Restart, for an
    /// announcement.
    RestartTime { instance: u32 },
}

/// What a [`Check`] found.
pub(super) enum Checked {
    Table(Option<u32>),
    Confirmed {
        watched: u32,
        claimed: u32,
        answer: Answer,
    },
    RestartTime(Restart),
}

/// What a scheduler knows of which device answers at its address and of
/// its restarts.
#[derive(Debug, Default)]
pub(super) struct Ids {
    /// The address's sender name was loaded from the client's memory.
    loaded: bool,
    /// A table read is queued or running.
    reading: bool,
    /// A confirm is queued or running.
    confirming: bool,
    /// When the device was last asked which it is: once a lifetime at most.
    checked: Option<Instant>,
    /// A claim waiting for the next confirm.
    deferred: Option<u32>,
    /// Instances claimed at the address since the device last said which
    /// it is.
    claims: BTreeSet<u32>,
    /// Claims the device refuted: the table still naming one is no claim.
    refuted: BTreeSet<u32>,
    /// Senders notifications named since the last confirm.
    senders: BTreeSet<u32>,
    /// A refuted claim was logged.
    logged: bool,
    /// When the points were first subscribed, or last subscribed again.
    restarted: Option<Instant>,
    /// The device's Time_Of_Device_Restart, once read for an announcement.
    restart_time: Option<Restart>,
    /// A restart-time read is queued or running.
    timing: bool,
    /// When the restart time was last read for an announcement.
    gated: Option<Instant>,
    /// When to check for a restart, and whether it is certain (the
    /// device said so).
    restart_due: Option<(Instant, bool)>,
}

/// Whether `row` is watched and not starting: a start in flight decides by
/// its own answer.
fn live(row: &Row) -> bool {
    !row.removing && !matches!(row.mode, Mode::Gone | Mode::Starting)
}

/// Whether `row` was given up for the device of `instance`.
fn given_up_for(row: &Row, instance: u32) -> bool {
    !row.removing && matches!(row.mode, Mode::Gone) && row.moved == Some(instance)
}

/// The part of [`SPREAD`] the device at `address` waits.
fn spread(address: &DeviceAddress) -> Duration {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    address.hash(&mut hasher);
    SPREAD.mul_f64((hasher.finish() % 1000) as f64 / 1000.0)
}

impl<T: TransportPort + 'static> Device<T> {
    fn budget(&self) -> Duration {
        self.inner.options.lifetime.duration()
    }

    /// The instance the points watch, if any is known.
    fn watched(&self) -> Option<u32> {
        let mut live = self.rows.values().filter(|row| live(row));
        live.find_map(|row| row.shared.identity())
    }

    /// The job for `check`, charged to the hub as any request.
    pub(super) fn check_job(&self, check: Check) -> Job {
        let inner = Arc::clone(&self.inner);
        let account = Arc::clone(&inner.account);
        let address = self.address.clone();
        match check {
            Check::Table => Box::pin(async move {
                Done::Checked(Checked::Table(jobs::verify(inner, address).await))
            }),
            Check::Confirm { watched, claimed } => Box::pin(async move {
                let answer = charged(account, jobs::confirm(inner, address, watched)).await;
                Done::Checked(Checked::Confirmed {
                    watched,
                    claimed,
                    answer,
                })
            }),
            Check::RestartTime { instance } => Box::pin(async move {
                let time = charged(account, jobs::restart_time(inner, address, instance)).await;
                Done::Checked(Checked::RestartTime(time))
            }),
        }
    }

    pub(super) fn checked(&mut self, checked: Checked) {
        match checked {
            Checked::Table(found) => {
                self.ids.reading = false;
                if let Some(found) = found {
                    self.named(found, false);
                }
            }
            Checked::Confirmed {
                watched,
                claimed,
                answer,
            } => self.confirmed(watched, claimed, answer),
            Checked::RestartTime(time) => self.timed(time),
        }
    }

    /// Take in what the receiver noted at the address since the last look.
    pub(super) fn heard(&mut self) {
        if !std::mem::replace(&mut self.ids.loaded, true) {
            let sender = self.inner.client.initiator(&self.address);
            self.slot.set_alias(sender);
        }
        let notes = self.slot.take_notes();
        for (found, fresh) in notes.announced {
            self.announced(found, fresh);
        }
        for sender in notes.senders {
            self.ids.senders.insert(sender);
            self.claim(sender);
        }
        if let Some(time) = notes.restart {
            self.restart_notice(time);
        }
        for instance in notes.elsewhere {
            if self.rows.values().any(|row| given_up_for(row, instance)) {
                self.locate(instance);
            } else {
                let registry = &self.inner.registry;
                registry.unindex_replaced(Some(instance), &self.address);
            }
        }
        if notes.missed && !self.ids.reading {
            self.ids.reading = true;
            self.ops.push_front(Op::Check(Check::Table));
        }
    }

    /// A device at the address announced itself as `found`, by an I-Am
    /// newer than any seen there when `fresh`.
    fn announced(&mut self, found: u32, fresh: bool) {
        self.ids.refuted.remove(&found);
        for row in self.rows.values() {
            if live(row) && !matches!(row.mode, Mode::Queued) {
                row.shared.learn(Some(found));
            }
        }
        if !self.named(found, true) && fresh && self.ids.restart_due.is_none() {
            let due = Instant::now() + spread(&self.address);
            self.ids.restart_due = Some((due, false));
        }
    }

    /// The address is said to hold `found` (by an announcement when
    /// `heard`, else by the table): if that would change any point (one of
    /// another instance, or one given up for it), the device there is asked
    /// which it is. Whether it is.
    fn named(&mut self, found: u32, heard: bool) -> bool {
        if !heard && self.ids.refuted.contains(&found) {
            return false;
        }
        let rows = || self.rows.values();
        let other = |row: &Row| live(row) && row.shared.identity().is_some_and(|i| i != found);
        let changes = rows().any(other) || rows().any(|row| given_up_for(row, found));
        if changes {
            self.claim(found);
        }
        changes
    }

    /// A start was answered while the table named `found` at the address.
    pub(super) fn answered_at(&mut self, found: Option<u32>) {
        if let Some(found) = found {
            self.named(found, false);
        }
    }

    /// Ask the device at the address which it is, for a claim of `claimed`
    /// there: now, or once the last ask is a lifetime old.
    fn claim(&mut self, claimed: u32) {
        self.ids.claims.insert(claimed);
        let now = Instant::now();
        let budget = self.budget();
        let spent = self.ids.checked.is_some_and(|at| now < at + budget);
        if self.ids.confirming || spent {
            self.ids.deferred = Some(claimed);
            return;
        }
        let watched = self.watched().unwrap_or(claimed);
        self.ids.checked = Some(now);
        self.ids.confirming = true;
        let check = Check::Confirm { watched, claimed };
        self.ops.push_front(Op::Check(check));
    }

    /// What the device at the address answered, asked about `claimed`
    /// while the points watch `watched`.
    fn confirmed(&mut self, watched: u32, claimed: u32, answer: Answer) {
        self.ids.confirming = false;
        match answer {
            Answer::Is(found) => self.settle(found, true),
            // It has no Device object `watched`: another device is there.
            Answer::Not => {
                self.ids.claims.clear();
                self.replaced(watched, claimed);
            }
            Answer::Unclear => {
                tracing::debug!(
                    "device at {} couldn't say which it is; device {claimed} there is ignored",
                    self.address
                );
                self.release_held();
            }
            Answer::Silent => {
                if !self.backing_off() {
                    self.back_off();
                }
                self.locate_all();
                self.release_held();
            }
        }
    }

    /// The device at the address said it is `found`: asked (`asked`), when
    /// the points given up for it start again, or in a poll's own answer,
    /// which may predate a move and brings none back. Every other instance
    /// claimed there since the last answer is refuted.
    fn settle(&mut self, found: u32, asked: bool) {
        self.ids.deferred = None;
        for doubt in std::mem::take(&mut self.ids.claims) {
            if doubt != found && self.ids.refuted.insert(doubt) {
                let count = self.inner.registry.refuted.fetch_add(1, Ordering::Relaxed) + 1;
                if !std::mem::replace(&mut self.ids.logged, true) {
                    tracing::info!(
                        "device {found} at {} says so itself: device {doubt} said to be there is \
                         ignored ({count} so far; later ones aren't logged)",
                        self.address
                    );
                }
            }
        }
        let senders = std::mem::take(&mut self.ids.senders);
        let here = self.watched() == Some(found);
        if let Some(sender) = senders.into_iter().find(|sender| here && *sender != found) {
            self.inner
                .client
                .accept_initiator(&self.address, found, sender);
            self.slot.set_alias(Some(sender));
        }
        self.check(found);
        if asked {
            self.answered_for(Some(found));
        }
        self.release_held();
    }

    /// Start again the points held for a claim the device settled.
    fn release_held(&mut self) {
        for (process, row) in &mut self.rows {
            if matches!(row.mode, Mode::Held) && !row.removing {
                row.mode = Mode::Queued;
                self.ops.push_back(Op::Start(*process));
            }
        }
    }

    /// The address names `found` now, by the device's own word: give up
    /// the points of every other instance.
    fn check(&mut self, found: u32) {
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
    /// `watched`'s points and look for it elsewhere, now and whenever it
    /// announces itself at another address.
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
            self.inner.registry.index_replaced(watched, &self.address);
            self.locate(watched);
        }
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

    /// The device at the address said it is `instance`: the points given
    /// up for that instance start again, and those given up for another
    /// stay.
    pub(super) fn answered_for(&mut self, instance: Option<u32>) {
        let Some(instance) = instance else {
            return;
        };
        let seq = &self.inner.registry.seq;
        let mut revived = false;
        for (process, row) in &mut self.rows {
            if given_up_for(row, instance) {
                row.moved = None;
                row.mode = Mode::Queued;
                row.shared.revive(seq);
                self.ops.push_back(Op::Start(*process));
                revived = true;
            }
        }
        if revived {
            self.inner
                .registry
                .unindex_replaced(Some(instance), &self.address);
            tracing::info!(
                "device {instance} answers at {} again; its watches start again",
                self.address
            );
        }
    }

    /// Whether `started`, the start of `process`, was answered while the
    /// table named another instance at the address than the one its
    /// point's watch started with (and no read refuted that). Then nothing
    /// of the start is the point's (its subscription, if it made one, isn't
    /// kept), and the point waits for the device there to say which it is.
    pub(super) fn started_elsewhere(&mut self, process: u32, started: &Started) -> bool {
        let refuted = &self.ids.refuted;
        let Some(row) = self.rows.get_mut(&process) else {
            return false;
        };
        let identity = row.shared.learn(started.before.or(started.instance));
        let (Some(watched), Some(found)) = (identity, started.instance) else {
            return false;
        };
        if watched == found || refuted.contains(&found) {
            return false;
        }
        if row.removing || row.shared.is_stopped() {
            let row = self.rows.remove(&process).expect("looked up above");
            self.finish(row);
            return true;
        }
        row.mode = Mode::Held;
        row.maybe_subscribed |= matches!(started.outcome, Outcome::Asked(Ok(_)));
        self.claim(found);
        true
    }

    /// What a poll said of which device answered, before its rows are
    /// taken: the points of another instance than the device's own answer
    /// are given up; without one, those of another than the table names
    /// there (unless refuted) are skipped, and the device is asked. The
    /// points skipped.
    pub(super) fn poll_identity(&mut self, polled: &Polled) -> BTreeSet<u32> {
        if let Some(found) = polled.answered {
            self.settle(found, false);
            return BTreeSet::new();
        }
        let Some(found) = polled
            .table
            .filter(|found| !self.ids.refuted.contains(found))
        else {
            return BTreeSet::new();
        };
        let skipped: BTreeSet<u32> = polled
            .rows
            .iter()
            .filter_map(|(process, _)| self.rows.get(process))
            .filter(|row| row.shared.identity().is_some_and(|i| i != found))
            .map(|row| row.shared.process)
            .collect();
        if !skipped.is_empty() {
            self.claim(found);
        }
        skipped
    }

    /// A point of the device was subscribed: the first time, its restart
    /// clock starts.
    pub(super) fn subscribed(&mut self) {
        self.ids.restarted.get_or_insert_with(Instant::now);
    }

    /// Run the identity checks due by `now`: a claim that waited for the
    /// last confirm to be a lifetime old, and a restart check.
    pub(super) fn identity_due(&mut self, now: Instant) {
        let budget = self.budget();
        let ready = self.ids.checked.is_none_or(|at| now >= at + budget);
        if !self.ids.confirming
            && ready
            && let Some(claimed) = self.ids.deferred.take()
        {
            self.claim(claimed);
        }
        if let Some((at, sure)) = self.ids.restart_due
            && at <= now
        {
            self.ids.restart_due = None;
            match sure {
                true => self.resubscribe(QUIET),
                false => self.restart_check(),
            }
        }
    }

    /// The device said it restarted, with its Time_Of_Device_Restart when
    /// it gave one: subscribe again, unless that time is known already.
    fn restart_notice(&mut self, time: Option<Vec<u8>>) {
        if let Some(time) = time {
            if matches!(&self.ids.restart_time, Some(Restart::At(known)) if *known == time) {
                return;
            }
            self.ids.restart_time = Some(Restart::At(time));
        }
        let due = Instant::now() + spread(&self.address);
        self.ids.restart_due = Some((due, true));
    }

    /// The device announced itself again: ask when it restarted (at most
    /// once a lifetime), or, without that property, subscribe again (at
    /// most once a lifetime).
    fn restart_check(&mut self) {
        let now = Instant::now();
        let budget = self.budget();
        if self.ids.restart_time == Some(Restart::Absent) {
            self.resubscribe(budget);
            return;
        }
        let read = self.ids.gated.is_some_and(|at| now < at + budget);
        if read || self.ids.timing {
            return;
        }
        let instance = self
            .watched()
            .unwrap_or(ObjectIdentifier::WILDCARD_INSTANCE);
        self.ids.gated = Some(now);
        self.ids.timing = true;
        self.ops
            .push_back(Op::Check(Check::RestartTime { instance }));
    }

    /// The device's Time_Of_Device_Restart, read for an announcement. The
    /// first read only learns it, and the device is taken as maybe
    /// restarted, as without the property.
    fn timed(&mut self, time: Restart) {
        self.ids.timing = false;
        if time == Restart::Silent {
            return;
        }
        match (self.ids.restart_time.replace(time.clone()), time) {
            (Some(Restart::At(before)), Restart::At(now)) if before == now => {}
            (Some(Restart::At(_)), Restart::At(_)) => self.resubscribe(QUIET),
            _ => self.resubscribe(self.budget()),
        }
    }

    /// Subscribe the points again: the device restarted, or may have. A
    /// subscription held is dropped, so it stops renewing (the start's own
    /// takes its place on the device, which keeps one a subscriber and
    /// object), and may still be cancelled. Not within `gap` of the last
    /// time, or of the first subscription.
    fn resubscribe(&mut self, gap: Duration) {
        let now = Instant::now();
        if self
            .ids
            .restarted
            .is_some_and(|at| now.saturating_duration_since(at) < gap)
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
            "device at {} may have restarted; subscribing its {} points again",
            self.address,
            again.len()
        );
        self.ids.restarted = Some(now);
        self.ops.extend(again.into_iter().map(Op::Start));
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
