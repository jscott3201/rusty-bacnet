//! Which device a point's values come from, and when it restarted (#2603).
//!
//! The rule: only an answer to a request the hub sent (a check's read of
//! the Device object, a poll's identity row, the sender probe, the
//! restart-time read, a start's read by the wildcard instance) may set a
//! point's device once one is known, confirm it, give a point up, change
//! the restart time known, or take a sender name for the device. A frame
//! nobody asked for (a COV notification, an I-Am and the discovery row it
//! fills, a restart notification) may only be taken as a value when it
//! names the point's device or the name taken for it, be dropped, or cue
//! one paced read. The one exception: with no device known at all, a
//! notification's sender is taken, unconfirmed.
//!
//! A point's device is what the device at its address last named itself in
//! such an answer (the client keeps it per address until the discovery
//! table changes there), else the confirmed device of the address's other
//! points, else the table's row there, else a notification's sender. Until
//! an answer names the point's device the point is unconfirmed, and the
//! first answer naming another replaces it: a stale or forged row is no
//! word of the device's. An answer naming another than a confirmed device
//! gives the point up ([`Why::Replaced`]). A poll by ReadPropertyMultiple
//! asks which device answers in the same request, and a subscribed device's
//! first restart-time read (by the point's instance) names it too.
//!
//! A cue (an I-Am of another device at the address, the table naming one
//! once a start or poll is answered, a notification naming another sender)
//! is a claim: the device there is asked which it is, by one read of its
//! Device object (by the wildcard instance, or, for a device that refused
//! that, by the watched one, and then by the claimed one when the watched
//! one is missing from an unconfirmed point). The first claim is asked at
//! once, the next after 60 s, the wait doubling up to the subscription
//! lifetime ([`WatchOptions::lifetime`]) and staying there: at most four
//! asks in the first seven minutes, then one a lifetime (twelve an hour for
//! the five-minute default), however many frames come. A claim waiting
//! puts a note on the points. While one waits, a point polled without the
//! identity row skips its polls, unless the device already refuted that
//! same claim (the last one refuted is kept): a forged I-Am of one instance
//! a second costs those reads and holds no values; one naming a new
//! instance each time holds them until the next ask. A given-up point's
//! renewals stop and nothing more is sent for it; its instance announcing
//! itself at another address has the scheduler look there, sending
//! nothing, and found there alone it is reported [`Why::Moved`].
//!
//! A device whose notifications name another sender (a gateway naming
//! itself, say): once the device said which it is, one read of the
//! sender's Device object at the address that finds none there has that
//! name taken for its points for a lifetime, until a device of that name
//! announces itself there, or the points are given up or come back. A
//! device there that answers as the sender too can't be told from it: its
//! points are polled instead ([`Why::Refused`], the note names the sender).
//!
//! What a frame nobody asked for does, by the point's state (cue: one paced
//! read, a check of which device answers or the restart-time read):
//!
//! | Frame | No device | Unconfirmed | Confirmed | Name taken | Claim waits | Given up |
//! |---|---|---|---|---|---|---|
//! | Notification, the point's device | - | value | value | value | value | dropped |
//! | Notification, another sender | sender taken, unconfirmed | dropped, cue | dropped, cue | value if the name, else dropped, cue | dropped, waits | dropped |
//! | I-Am, the point's device here | restart cue | restart cue | restart cue | restart cue | restart cue | cue; back on the answer |
//! | I-Am, another device here | restart cue | cue | cue | cue; drops the name if it is the name | waits | - |
//! | I-Am, a given-up device elsewhere | - | - | - | - | - | looked for (no request, or a following handle's paced Who-Is) |
//! | Restart notification, its own | restart cue | restart cue | restart cue | restart cue | restart cue | - |
//! | Restart notification, another's | ignored | ignored | ignored | ignored | ignored | ignored |
//!
//! A table row such a frame filled, read when a start or poll is answered
//! or after a miss, is a cue too, and holds the start's values, or the
//! polls without the identity row, until the answer (unless refuted
//! before). Points are given up as moved only by the pre-existing rule:
//! after a request of the hub's went unanswered, on the table's positive
//! evidence or a following handle's own move.
//!
//! Not caught: a device that never announces itself taking the address of
//! a point polled by ReadProperty alone, and one taking the address before
//! any answer named the watched device (it is taken as the point's device).
//!
//! A device announcing itself again for its own instance, or sending a
//! restart notification (Clause 19.3), may have restarted and lost its
//! subscriptions: its Time_Of_Device_Restart, read once when its points are
//! first subscribed, is read again, at most once a lifetime. When it moved,
//! the subscribed points are read and subscribed again (not within
//! [`QUIET`] of the last time), and those polled after a failure tried
//! again; unchanged, missing, or never moving (no clock), nothing more is
//! sent, and the points are recovered at their renewal. A restart
//! notification's own time isn't taken. A restart check waits a part of
//! [`SPREAD`] set by the address and the hub, so devices answering one
//! Who-Is are asked one after another; asks of which device answers are
//! paced by their own wait, not spread.
//!
//! When announcements may have been missed, the scheduler reads the table
//! at the address, which sends nothing. Nothing here awaits: the reads are
//! jobs, which the scheduler polls beside its wake-up and its tick, none
//! of them first, and the receiver wakes it only for news.
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
use super::super::hub::Source;
use super::super::jobs::{self, Answer, Outcome, Polled, Restart, Started};
use super::super::shared::{Alias, Running};
use super::{Device, Done, Holder, Job, Mode, Op, Row};
use crate::device::{DeviceAddress, QUIET};
use crate::limits::charged;

/// The longest a restart check waits after an announcement: a part of it
/// set by the device's address and the hub.
pub(super) const SPREAD: Duration = Duration::from_secs(10);

/// The wait after the first ask of which device answers, which doubles at
/// each ask up to the subscription lifetime and stays there.
const ASK_AGAIN: Duration = Duration::from_secs(60);

/// A check a scheduler runs as a job.
pub(super) enum Check {
    /// Read the discovery table at the address. Sends nothing.
    Table,
    /// Ask the device at the address which instance it is: another,
    /// `claimed`, was said to be there while the points watch `watched`.
    Confirm { watched: u32, claimed: u32 },
    /// Whether device `claimed`'s Device object answers at the address,
    /// whose unconfirmed points' device has none there.
    Probe { claimed: u32 },
    /// Whether device `sender`'s Device object answers at the address,
    /// whose device `device` names itself so in its notifications.
    Alias { device: u32, sender: u32 },
    /// Read device `instance`'s Time_Of_Device_Restart.
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
    Probed {
        claimed: u32,
        answer: Answer,
    },
    Alias {
        device: u32,
        sender: u32,
        answer: Answer,
    },
    /// The time, and whether the device answered as Device `instance`.
    RestartTime {
        instance: u32,
        time: Restart,
        there: bool,
    },
}

/// What a scheduler knows of which device answers at its address and of
/// its restarts.
#[derive(Debug, Default)]
pub(super) struct Ids {
    /// A table read is queued or running.
    reading: bool,
    /// A confirm or a probe is queued or running.
    confirming: bool,
    /// When the device was last asked which it is, and the wait before the
    /// next ask.
    checked: Option<Instant>,
    wait: Duration,
    /// The latest claim waiting for the next ask.
    deferred: Option<u32>,
    /// The last claim the device refuted.
    refuted: Option<u32>,
    /// The sender a notification named, raising the claim being asked.
    sender: Option<u32>,
    /// A refuted claim was logged.
    logged: bool,
    /// When the points were first subscribed, or last subscribed again.
    restarted: Option<Instant>,
    /// The device's Time_Of_Device_Restart, once read.
    restart_time: Option<Restart>,
    /// A restart-time read is queued or running.
    timing: bool,
    /// When the restart time was last read for a cue.
    gated: Option<Instant>,
    /// When to read the restart time for a cue.
    restart_due: Option<Instant>,
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

/// The part of [`SPREAD`] the device at `address` waits in the hub whose
/// process identifiers start at `hub`.
fn spread(address: &DeviceAddress, hub: u32) -> Duration {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (address, hub).hash(&mut hasher);
    SPREAD.mul_f64((hasher.finish() % 1000) as f64 / 1000.0)
}

impl<T: TransportPort + 'static> Device<T> {
    fn lifetime(&self) -> Duration {
        self.inner.options.lifetime.duration()
    }

    fn spread(&self) -> Duration {
        spread(&self.address, self.inner.registry.process_base)
    }

    /// The instance the points watch, if any is known.
    fn watched(&self) -> Option<u32> {
        let mut live = self.rows.values().filter(|row| live(row));
        live.find_map(|row| row.shared.identity())
    }

    /// The device an answer named for the points, if one did.
    pub(super) fn known(&self) -> Option<u32> {
        let mut live = self.rows.values().filter(|row| live(row));
        live.find_map(|row| row.shared.confirmed())
    }

    /// Whether the table naming `found` at the address is stale: the
    /// device's last word there names another, or it refuted `found`.
    fn stale(&self, found: u32) -> bool {
        let said = self.inner.client.said(&self.address);
        said.is_some_and(|said| said != found) || self.ids.refuted == Some(found)
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
            Check::Probe { claimed } => Box::pin(async move {
                let answer = charged(account, jobs::probe(inner, address, claimed)).await;
                Done::Checked(Checked::Probed { claimed, answer })
            }),
            Check::Alias { device, sender } => Box::pin(async move {
                let answer = charged(account, jobs::probe(inner, address, sender)).await;
                Done::Checked(Checked::Alias {
                    device,
                    sender,
                    answer,
                })
            }),
            Check::RestartTime { instance } => Box::pin(async move {
                let read = jobs::restart_time(inner, address, instance);
                let (time, there) = charged(account, read).await;
                Done::Checked(Checked::RestartTime {
                    instance,
                    time,
                    there,
                })
            }),
        }
    }

    pub(super) fn checked(&mut self, checked: Checked) {
        match checked {
            // Read after announcements were missed: the device's last word
            // may predate them, so only a claim it refuted is passed over.
            Checked::Table(found) => {
                self.ids.reading = false;
                if let Some(found) = found.filter(|found| self.ids.refuted != Some(*found)) {
                    self.named(found);
                }
            }
            Checked::Confirmed {
                watched,
                claimed,
                answer,
            } => self.confirmed(watched, claimed, answer),
            Checked::Probed { claimed, answer } => self.probed(claimed, answer),
            Checked::Alias {
                device,
                sender,
                answer,
            } => self.aliased(device, sender, answer),
            Checked::RestartTime {
                instance,
                time,
                there,
            } => self.timed(instance, time, there),
        }
    }

    /// Take in what the receiver noted at the address since the last look.
    pub(super) fn heard(&mut self) {
        let notes = self.slot.take_notes();
        for (found, fresh) in notes.announced {
            self.announced(found, fresh);
        }
        for sender in notes.senders {
            self.ids.sender = Some(sender);
            self.claim(sender);
        }
        if let Some(instance) = notes.restart {
            self.restart_notice(instance);
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
        // A device of the name a gateway's notifications use may be here.
        if self.slot.alias().is_some_and(|alias| alias.sender == found) {
            self.slot.set_alias(None);
        }
        if !self.named(found) && fresh {
            self.restart_cue();
        }
    }

    /// The address is said to hold `found`: if that would change any point
    /// (one of another instance, or one given up for it), the device there
    /// is asked which it is. Whether it is.
    fn named(&mut self, found: u32) -> bool {
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
        if let Some(found) = found.filter(|found| !self.stale(*found)) {
            self.named(found);
        }
    }

    /// Ask the device at the address which it is, for a claim of `claimed`
    /// there: now, or once the wait since the last ask is over.
    fn claim(&mut self, claimed: u32) {
        self.doubt(claimed);
        let now = Instant::now();
        let spent = self.ids.checked.is_some_and(|at| now < at + self.ids.wait);
        if self.ids.confirming || spent {
            self.ids.deferred = Some(claimed);
            return;
        }
        let cap = self.lifetime();
        let first = ASK_AGAIN.min(cap);
        self.ids.wait = match self.ids.checked {
            Some(_) => (self.ids.wait * 2).clamp(first, cap),
            None => first,
        };
        self.ids.checked = Some(now);
        self.ids.confirming = true;
        let watched = self.watched().unwrap_or(claimed);
        let check = Check::Confirm { watched, claimed };
        self.ops.push_front(Op::Check(check));
    }

    /// Note on the points of other instances that `claimed` is said to be
    /// at the address, until the device there says which it is.
    fn doubt(&self, claimed: u32) {
        let at = &self.address;
        let note = format!(
            "device {claimed} is said to answer at {at}; what it sends isn't taken \
             until the device there is asked which it is"
        );
        let seq = &self.inner.registry.seq;
        for row in self.rows.values().filter(|row| live(row)) {
            if row.shared.identity().is_some_and(|own| own != claimed) {
                row.shared.doubt(seq, note.clone());
            }
        }
    }

    /// The claims are settled: the points' own notes come back.
    fn undoubt(&self) {
        if self.ids.confirming || self.ids.deferred.is_some() {
            return;
        }
        let seq = &self.inner.registry.seq;
        for row in self.rows.values() {
            row.shared.undoubt(seq);
        }
    }

    /// What the device at the address answered, asked about `claimed`
    /// while the points watch `watched`.
    fn confirmed(&mut self, watched: u32, claimed: u32, answer: Answer) {
        self.ids.confirming = false;
        match answer {
            Answer::Is(found) => {
                self.ids.deferred = None;
                self.settle(found, Some(claimed));
                return;
            }
            // It has no Device object `watched`: another device is there,
            // when an answer had named `watched` for the points; otherwise
            // the claimed one may be what they watch.
            Answer::Not => {
                let rows = || self.rows.values().filter(|row| live(row));
                if rows().any(|row| row.shared.confirmed() == Some(watched)) {
                    self.ids.sender = None;
                    self.replaced(watched, claimed);
                } else if claimed != watched {
                    self.ids.confirming = true;
                    self.ops.push_front(Op::Check(Check::Probe { claimed }));
                    return;
                }
            }
            Answer::Unclear => {
                tracing::debug!(
                    "device at {} couldn't say which it is; device {claimed} there is ignored",
                    self.address
                );
                self.ids.sender = None;
            }
            Answer::Silent => {
                self.ids.sender = None;
                if !self.backing_off() {
                    self.back_off();
                }
                self.locate_all();
            }
        }
        self.release_held();
        self.undoubt();
    }

    /// Whether device `claimed` answers at the address, whose unconfirmed
    /// points' device has no Device object there: then it is theirs.
    fn probed(&mut self, claimed: u32, answer: Answer) {
        self.ids.confirming = false;
        match answer {
            Answer::Is(found) => self.settle(found, Some(claimed)),
            _ => {
                self.ids.sender = None;
                self.release_held();
                self.undoubt();
            }
        }
    }

    /// The device at the address said it is `found`: asked about a claim
    /// of `claimed`, or in a poll's own answer (`None`), which may predate
    /// a move, brings none back and leaves a waiting claim to its ask.
    fn settle(&mut self, found: u32, claimed: Option<u32>) {
        if self.ids.refuted == Some(found) {
            self.ids.refuted = None;
        }
        if let Some(claimed) = claimed.filter(|claimed| *claimed != found) {
            self.ids.refuted = Some(claimed);
            let count = self.inner.registry.refuted.fetch_add(1, Ordering::Relaxed) + 1;
            if !std::mem::replace(&mut self.ids.logged, true) {
                tracing::info!(
                    "device {found} at {} says so itself: device {claimed} said to be there is \
                     ignored ({count} so far; later ones aren't logged)",
                    self.address
                );
            }
        }
        let sender = match claimed {
            Some(_) => self.ids.sender.take(),
            None => None,
        };
        self.check(found);
        if let Some(sender) = sender.filter(|sender| *sender != found)
            && self.known() == Some(found)
        {
            let check = Check::Alias {
                device: found,
                sender,
            };
            self.ops.push_front(Op::Check(check));
        }
        if claimed.is_some() {
            self.answered_for(Some(found));
        }
        self.release_held();
        self.undoubt();
    }

    /// What a read of device `sender`'s Device object found at the address
    /// of device `device`, whose notifications name `sender`: none there,
    /// the name is taken for `device`'s points for a lifetime; one there
    /// too, they can't be told apart, and the points are polled instead.
    fn aliased(&mut self, device: u32, sender: u32, answer: Answer) {
        if self.known() != Some(device) {
            return;
        }
        match answer {
            Answer::Not => {
                tracing::info!(
                    "device {device} at {} names itself {sender} in its notifications; \
                     they are taken as its own",
                    self.address
                );
                let until = Instant::now() + self.lifetime();
                self.slot.set_alias(Some(Alias {
                    device,
                    sender,
                    until,
                }));
            }
            Answer::Is(found) if found == sender => self.poll_instead(device, sender),
            _ => {}
        }
    }

    /// Poll device `device`'s subscribed points: its notifications name
    /// device `sender`, which answers at the address too. Their
    /// subscriptions are cancelled, and the address remembered as refusing
    /// their COV service, as a device's own refusal would be.
    fn poll_instead(&mut self, device: u32, sender: u32) {
        let at = &self.address;
        tracing::info!(
            "device {device} at {at} names device {sender} in its notifications, which \
             answers there too; its points are polled"
        );
        let note = format!(
            "its notifications name device {sender}, which answers at {at} too, so \
             they can't be told from another device's; polled instead"
        );
        let seq = &self.inner.registry.seq;
        let mut cancels = Vec::new();
        for row in self.rows.values_mut() {
            if !live(row) || row.shared.identity() != Some(device) {
                continue;
            }
            let Mode::Cov(managed, _) = std::mem::replace(&mut row.mode, Mode::Polled) else {
                continue;
            };
            row.resubscribe = None;
            row.maybe_subscribed = false;
            let watched = &row.shared.point.watched;
            self.inner
                .client
                .cov_observed(at, watched, false, Some(device));
            let state = {
                let mut entry = row.shared.lock();
                entry.snapshot.why = Some(Why::Refused(note.clone()));
                entry.snapshot.state
            };
            row.shared
                .set_state(seq, Some(Source::Poll), state, Some(note.clone()));
            if let Some(target) = row.target.clone() {
                cancels.push((Arc::clone(&row.shared), target, *managed));
            }
        }
        for (shared, target, managed) in cancels {
            let running = Running::new(&self.inner.registry);
            self.queue_cancel(&shared, Holder::Alone(running), target, Some(managed));
        }
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

    /// The device at the address named itself `found` in an answer: a point
    /// whose device no answer had named takes it, and a point of another
    /// instance an answer had named is given up for it.
    fn check(&mut self, found: u32) {
        self.inner.client.note_said(&self.address, found);
        let others: BTreeSet<u32> = self
            .rows
            .values()
            .filter(|row| live(row))
            .filter_map(|row| row.shared.named(found))
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
            self.slot.set_alias(None);
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
            self.slot.set_alias(None);
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
    /// table named another instance at the address than the point's (and
    /// the device's last word doesn't say otherwise). Then nothing of the
    /// start is the point's (its subscription, if it made one, isn't kept),
    /// and the point waits for the device there to say which it is.
    pub(super) fn started_elsewhere(&mut self, process: u32, started: &Started) -> bool {
        let stale = started.instance.is_some_and(|found| self.stale(found));
        let Some(row) = self.rows.get_mut(&process) else {
            return false;
        };
        let (learned, vouched) = match started.before {
            Some(before) => (Some(before), started.vouched),
            None => (started.instance, false),
        };
        let identity = row.shared.learn(learned, vouched);
        let (Some(watched), Some(found)) = (identity, started.instance) else {
            return false;
        };
        if watched == found || stale {
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
    /// taken: the device's own answer settles which it is; without one, the
    /// points of another instance than the table names there (unless the
    /// device's last word says otherwise, or it refuted that one) are
    /// skipped, and the device is asked. The points skipped.
    pub(super) fn poll_identity(&mut self, polled: &Polled) -> BTreeSet<u32> {
        if let Some(found) = polled.answered {
            self.settle(found, None);
            return BTreeSet::new();
        }
        // Polled without the question: its restart-time read names it.
        if self.known().is_none() && self.ids.restart_time.is_none() && !self.ids.timing {
            self.read_restart_time();
        }
        let Some(found) = polled.table.filter(|found| !self.stale(*found)) else {
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
    /// clock starts and its Time_Of_Device_Restart is read once, so its
    /// first restart can be told from it; the read names the device too.
    pub(super) fn subscribed(&mut self) {
        self.ids.restarted.get_or_insert_with(Instant::now);
        if self.ids.restart_time.is_none() && !self.ids.timing {
            self.read_restart_time();
        }
    }

    /// Read the device's Time_Of_Device_Restart.
    fn read_restart_time(&mut self) {
        let instance = self
            .watched()
            .unwrap_or(ObjectIdentifier::WILDCARD_INSTANCE);
        self.ids.timing = true;
        self.ops
            .push_back(Op::Check(Check::RestartTime { instance }));
    }

    /// Run the identity checks due by `now`: a claim whose wait is over,
    /// and a restart check.
    pub(super) fn identity_due(&mut self, now: Instant) {
        let ready = self.ids.checked.is_none_or(|at| now >= at + self.ids.wait);
        if !self.ids.confirming
            && ready
            && let Some(claimed) = self.ids.deferred.take()
        {
            self.claim(claimed);
        }
        if self.ids.restart_due.is_some_and(|at| at <= now) {
            self.ids.restart_due = None;
            self.restart_check();
        }
    }

    /// Device `instance` at the address said it restarted: a cue to read
    /// its restart time, unless it is another device than the points'.
    fn restart_notice(&mut self, instance: u32) {
        if self.watched().is_none_or(|watched| watched == instance) {
            self.restart_cue();
        }
    }

    /// The device may have restarted: read its restart time within the
    /// spread.
    fn restart_cue(&mut self) {
        if self.ids.restart_due.is_none() {
            self.ids.restart_due = Some(Instant::now() + self.spread());
        }
    }

    /// Read the device's restart time for a cue, at most once a lifetime.
    /// A device without that property is recovered at its next renewal.
    fn restart_check(&mut self) {
        let now = Instant::now();
        let read = self.ids.gated.is_some_and(|at| now < at + self.lifetime());
        let lacks = self.ids.restart_time == Some(Restart::Absent);
        if !lacks && !read && !self.ids.timing {
            self.ids.gated = Some(now);
            self.read_restart_time();
        }
    }

    /// Device `instance`'s Time_Of_Device_Restart, as read: when the device
    /// answered as that Device object, the points of that instance are
    /// confirmed; when the time moved since the last read, the device
    /// restarted. The first read only learns it.
    fn timed(&mut self, instance: u32, time: Restart, there: bool) {
        self.ids.timing = false;
        if there && instance != ObjectIdentifier::WILDCARD_INSTANCE {
            self.inner.client.note_said(&self.address, instance);
            for row in self.rows.values().filter(|row| live(row)) {
                if row.shared.identity() == Some(instance) {
                    row.shared.named(instance);
                }
            }
        }
        if time == Restart::Silent {
            return;
        }
        let before = self.ids.restart_time.replace(time.clone());
        if let (Some(Restart::At(before)), Restart::At(now)) = (before, time)
            && before != now
        {
            self.resubscribe();
        }
    }

    /// Subscribe the points again: the device restarted. A subscription
    /// held is dropped, so it stops renewing (the start's own takes its
    /// place on the device, which keeps one a subscriber and object), and
    /// may still be cancelled. Not within [`QUIET`] of the last time, or of
    /// the first subscription.
    fn resubscribe(&mut self) {
        let now = Instant::now();
        if self
            .ids
            .restarted
            .is_some_and(|at| now.saturating_duration_since(at) < QUIET)
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
            "device at {} restarted; subscribing its {} points again",
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
