//! Which device a point's values come from, and when it restarted (#2603).
//!
//! The rule: only an answer to a request the hub sent may set a point's
//! device once one is known, give a point up, change the restart time
//! known, or take a sender name for the device; and only an answer that
//! names the device (a read of its Device object by the wildcard instance,
//! or a poll's identity row) may confirm it. A read of one Device object by
//! its instance (the sender probe, the read of the watched instance for a
//! device that refuses the wildcard, the restart-time read) names nothing,
//! as a gateway may answer for any instance. A frame nobody asked for (a
//! COV notification, an I-Am and the discovery row it fills, a restart
//! notification) may only be taken as a value when it names the point's
//! device or the name taken for it, be dropped, or cue one paced read. The
//! one exception: with no device known at all, a notification's sender is
//! taken, unconfirmed. (The client acknowledges a confirmed notification to
//! the frame's source whatever the hub makes of it.)
//!
//! A point's device is what the device at its address last named itself
//! (the client keeps it per address until the discovery table changes
//! there), else the confirmed device of the address's other points, else
//! the table's row there, else a notification's sender. Until an answer
//! names it the point is unconfirmed: the device there is asked its name
//! once its points are first subscribed or polled without the identity row,
//! and the first answer naming another replaces the point's device, as a
//! stale or forged row is no word of the device's. An answer naming another
//! than a confirmed device gives the point up ([`Why::Replaced`]).
//!
//! A cue (an I-Am of another device at the address, the table naming one
//! once a start or poll is answered, a notification naming another sender)
//! is a claim: the device there is asked which it is, by one read of its
//! Device object by the wildcard instance, or, for a device that refused
//! that, by the watched instance; that one missing (or, for points no
//! answer named, not shown: another error, another device's identifier),
//! the claimed instance is read, then the one the table names, and the
//! points take the one that answers, unconfirmed (a confirmed point is
//! given up for it). The client's own failure refutes nothing. The first claim is asked
//! at once, the next after 60 s, the wait doubling up to the subscription
//! lifetime ([`WatchOptions::lifetime`]) and staying there: at most four
//! asks in the first seven minutes, then one a lifetime (twelve an hour for
//! the five-minute default), however many frames come. A claim waiting puts
//! a note on the points. While an ask runs (it goes first in the device's
//! queue), a start answered while the table named another instance, and
//! polls without the identity row, are held, for the whole chain at most:
//! up to four requests, one of which may time out. Between asks their
//! values are taken, the poll's answer being the device's. The last claim refuted is
//! kept by the address's scheduler, and isn't cued again from the table. A
//! given-up point's renewals stop and nothing more is sent for it; its
//! instance announcing itself at another address has the scheduler look
//! there, and found there alone, and named there by the device's own
//! answer, it is reported [`Why::Moved`] ([`moves`](super::moves)).
//!
//! A device whose notifications name another sender (a gateway naming
//! itself, say): once the device was asked which it is, one read of the
//! sender's Device object at the address that finds none there has that
//! name taken for its points for a lifetime, until a device of that name
//! announces itself there, or the points are given up or come back; when
//! that lifetime ends, the next such notification is dropped while the name
//! is checked again. Any other answer to that read (one there answers as
//! the sender, or as the device, or can't say) means the notifications
//! can't be told from another device's: the points are polled instead
//! ([`Why::ForeignSender`]), probed again when they start again or another
//! watch of the device starts.
//!
//! What a frame nobody asked for does, by the point's state (cue: one paced
//! read, a check of which device answers or the restart-time read; cue
//! there: one paced read by the wildcard instance at the address the
//! device is said to have moved to; when it names the point's device, the
//! device here is asked once more (not when no route here is known, nor,
//! here failing, for the scheduler's own look, which followed its own
//! unanswered request): still that device, nothing changes (two
//! claimants); timing out (no other failure), or not that device, the
//! point is given up as moved, for another device named here given up for
//! it first; cue there, no ask: a point given up already only says where
//! its device went, a hint for whoever watches it there):
//!
//! | Frame | No device | Unconfirmed | Confirmed | Name taken | Claim waits | Given up |
//! |---|---|---|---|---|---|---|
//! | Notification, the point's device | - | value | value | value | value | dropped |
//! | Notification, another sender | sender taken, unconfirmed | dropped, cue | dropped, cue | value if the name, else dropped, cue | dropped, waits | dropped |
//! | I-Am, the point's device here | - | restart cue | restart cue | restart cue | restart cue | cue; back on the answer |
//! | I-Am, another device here | - | cue | cue | cue; drops the name if it is the name | waits | - |
//! | I-Am, a given-up device elsewhere | - | - | - | - | - | looked for (no request, or a following handle's paced Who-Is); found, cue there, no ask |
//! | A handle following the device moving away (after its own reads) | cue there, asked here even while failing | same | same | same | same | cue there, no ask |
//! | I-Am, the point's device elsewhere, answering there too | - | cue there through a handle's move or the scheduler's look: two claimants, nothing | same | same | same | cue there, no ask |
//! | Restart notification, its own | - | restart cue | restart cue | restart cue | restart cue | - |
//! | Restart notification, another's | ignored | ignored | ignored | ignored | ignored | ignored |
//!
//! "Given up" is a pinned point's [`Phase::Gone`]; a following point
//! (#2795) is unreachable then, as the hub looks for its device, and where
//! a pinned point would be given up as moved it goes to the new address
//! instead ([`follow`](super::follow)). A following point's instance is
//! its own from the start: an answer naming another is another device.
//!
//! [`Phase::Gone`]: super::super::hub::Phase::Gone
//!
//! Points are given up as moved only on that read's answer (#2812), and,
//! while they are watched, on the answer here to one more ask
//! ([`moves`](super::moves)), skipped only with no route here, or for the
//! scheduler's own look while here is failing: its cue is a following
//! handle's move (once the device there named the instance to the
//! handle's own read), or the new address the scheduler's own look found
//! after one of its requests went unanswered (the handle's rule, or the
//! table's positive evidence).
//!
//! Not caught: a device taking the address before the watched device named
//! itself, or at all for a device that refuses the wildcard read (it is
//! taken as the point's device); and a device that never announces itself
//! taking the address of a point polled by ReadProperty alone. A forged
//! answer needs the request's invoke id.
//!
//! A device announcing itself again for its own instance, or sending a
//! restart notification (Clause 19.3), may have restarted and lost its
//! subscriptions: its Time_Of_Device_Restart, read once when its points are
//! first subscribed, is read again, at most once a lifetime. When it moved,
//! the subscribed points are read and subscribed again (not within
//! [`QUIET`] of the last time), and those polled after a failure tried
//! again; unchanged, missing, or never moving (no clock), nothing more is
//! sent, and the points are recovered at their renewal. A read finding no
//! such Device object learns nothing, and asks the device its name. A
//! restart notification's own time isn't taken. A restart check waits a
//! part of [`SPREAD`] set by the address and the hub, so devices answering
//! one Who-Is are asked one after another; asks of which device answers are
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

/// When asks of which device answers may go: the first at once, the next
/// [`ASK_AGAIN`] after it, the wait doubling at each ask up to a cap (the
/// subscription lifetime) and staying there. A claim's asks keep one, and
/// so do the reads at an address the device is said to have moved to
/// (#2812).
#[derive(Debug, Default)]
pub(super) struct Pace {
    /// When the last ask went.
    asked: Option<Instant>,
    /// The wait after it.
    wait: Duration,
}

impl Pace {
    /// Whether an ask may go at `now`.
    pub(super) fn ready(&self, now: Instant) -> bool {
        self.asked.is_none_or(|at| now >= at + self.wait)
    }

    /// An ask goes at `now`, which sets the wait before the next.
    pub(super) fn spend(&mut self, now: Instant, cap: Duration) {
        let first = ASK_AGAIN.min(cap);
        self.wait = match self.asked {
            Some(_) => (self.wait * 2).clamp(first, cap),
            None => first,
        };
        self.asked = Some(now);
    }
}

/// A check a scheduler runs as a job.
pub(super) enum Check {
    /// Read the discovery table at the address. Sends nothing.
    Table,
    /// Ask the device at the address which instance it is: another,
    /// `claimed`, was said to be there while the points watch `watched`.
    Confirm { watched: u32, claimed: u32 },
    /// For a claim, whether a Device object (the instance given, else the
    /// one the table names) answers at the address, whose points' device
    /// has none there.
    Probe(u32, Option<u32>),
    /// Ask the device at the address its name, by the wildcard instance.
    Name,
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
    /// For the claim, whether the instance read was the claimed one (not
    /// the table's), and the answer.
    Probed(u32, bool, Answer),
    /// The instance read, and its restart time.
    Named(Option<u32>),
    Alias {
        device: u32,
        sender: u32,
        answer: Answer,
    },
    RestartTime(u32, Restart),
}

/// What a scheduler knows of which device answers at its address and of
/// its restarts.
#[derive(Debug, Default)]
pub(super) struct Ids {
    /// A table read is queued or running.
    reading: bool,
    /// A confirm or a probe is queued or running.
    confirming: bool,
    /// A read of the device's name is queued or running.
    naming: bool,
    /// The device was asked its name unprompted (once).
    named_asked: bool,
    /// When the device may next be asked which it is.
    pace: Pace,
    /// The latest claim waiting for the next ask.
    deferred: Option<u32>,
    /// The last claim the device didn't bear out.
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
    pub(super) fn lifetime(&self) -> Duration {
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
            Check::Probe(claim, instance) => Box::pin(async move {
                let first = instance.is_some();
                let instance = match instance {
                    Some(instance) => Some(instance),
                    None => inner.client.instance_at(&address).await,
                };
                let answer = match instance {
                    Some(instance) => charged(account, jobs::probe(inner, address, instance)).await,
                    None => Answer::Unclear,
                };
                Done::Checked(Checked::Probed(claim, first, answer))
            }),
            Check::Name => Box::pin(async move {
                let found = charged(account, jobs::name(inner, address)).await;
                Done::Checked(Checked::Named(found))
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
                let time = charged(account, jobs::restart_time(inner, address, instance)).await;
                Done::Checked(Checked::RestartTime(instance, time))
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
            Checked::Probed(claim, first, answer) => self.probed(claim, first, answer),
            Checked::Named(found) => {
                self.ids.naming = false;
                if let Some(found) = found {
                    self.settle(found, None);
                }
            }
            Checked::Alias {
                device,
                sender,
                answer,
            } => self.aliased(device, sender, answer),
            Checked::RestartTime(instance, time) => self.timed(instance, time),
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
        if !self.named(found) && fresh && self.watched() == Some(found) {
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
        if self.ids.confirming || !self.ids.pace.ready(now) {
            self.ids.deferred = Some(claimed);
            return;
        }
        self.ids.pace.spend(now, self.lifetime());
        self.ids.confirming = true;
        let watched = self.watched().unwrap_or(claimed);
        let check = Check::Confirm { watched, claimed };
        self.ops.push_front(Op::Check(check));
    }

    /// Ask the device at the address its name, unless that is under way or
    /// it refused the wildcard read.
    fn name_device(&mut self) {
        if !self.ids.naming && !self.inner.client.wildcard_refused(&self.address) {
            self.ids.naming = true;
            self.ops.push_front(Op::Check(Check::Name));
        }
    }

    /// Note on the points of other instances that `claimed` is said to be
    /// at the address, until the device there says which it is.
    fn doubt(&self, claimed: u32) {
        let at = &self.address;
        let note = format!(
            "device {claimed} is said to answer at {at}; notifications naming it aren't \
             taken, and the device there will be asked which it is"
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

    /// The device didn't bear out the claim of `claim` at the address.
    fn refute(&mut self, claim: u32) {
        self.ids.refuted = Some(claim);
        let count = self.inner.registry.refuted.fetch_add(1, Ordering::Relaxed) + 1;
        if !std::mem::replace(&mut self.ids.logged, true) {
            tracing::info!(
                "device {claim} said to answer at {} isn't borne out by the device there; \
                 ignored ({count} so far; later ones aren't logged)",
                self.address
            );
        }
    }

    /// What the device at the address answered, asked about `claimed`
    /// while the points watch `watched`.
    fn confirmed(&mut self, watched: u32, claimed: u32, answer: Answer) {
        self.ids.confirming = false;
        // For points no answer named, a read of the watched instance that
        // doesn't show it (another error, another device's identifier) is
        // as good as its absence.
        let rows = || self.rows.values().filter(|row| live(row));
        let named = rows().any(|row| row.shared.confirmed() == Some(watched));
        let answer = match answer {
            Answer::Unclear if !named => Answer::Not,
            Answer::Has(found) if found != watched && !named => Answer::Not,
            answer => answer,
        };
        match answer {
            Answer::Is(found) => {
                self.ids.deferred = None;
                self.settle(found, Some(claimed));
                return;
            }
            // It has Device object `found` (read by its instance, for a
            // device that refuses the wildcard): no name, but no sign of the
            // claimed device either.
            Answer::Has(found) => {
                self.ids.deferred = None;
                if claimed != found {
                    self.refute(claimed);
                }
                self.sender_probe(found);
                self.answered_for(Some(found));
            }
            // It has no Device object `watched`: the claimed one is read
            // next, then the one the table names.
            Answer::Not => {
                self.ids.confirming = true;
                let instance = (claimed != watched).then_some(claimed);
                self.ops
                    .push_front(Op::Check(Check::Probe(claimed, instance)));
                return;
            }
            Answer::Unclear => {
                if claimed != watched {
                    self.refute(claimed);
                }
                self.ids.sender = None;
            }
            Answer::Failed => self.ids.sender = None,
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

    /// What a read of a Device object found at the address, whose points'
    /// device has none there, for the claim `claim`: the instance that
    /// answers is the points' (a confirmed one is given up for it); none
    /// does, the claimed one read `first`, the table's is read next.
    fn probed(&mut self, claim: u32, first: bool, answer: Answer) {
        self.ids.confirming = false;
        match answer {
            Answer::Has(found) | Answer::Is(found) => {
                if found != claim {
                    self.refute(claim);
                }
                self.ids.sender = None;
                let others: BTreeSet<u32> = self
                    .rows
                    .values()
                    .filter(|row| live(row))
                    .filter_map(|row| row.shared.named(found, false))
                    .collect();
                for watched in others {
                    self.replaced(watched, found);
                }
                self.answered_for(Some(found));
            }
            Answer::Not | Answer::Unclear if first => {
                self.ids.confirming = true;
                self.ops.push_front(Op::Check(Check::Probe(claim, None)));
                return;
            }
            Answer::Not | Answer::Unclear => {
                self.refute(claim);
                self.ids.sender = None;
            }
            Answer::Silent | Answer::Failed => self.ids.sender = None,
        }
        self.release_held();
        self.undoubt();
    }

    /// The device at the address named itself `found` (by the wildcard
    /// instance or in a poll's identity row): asked about a claim of
    /// `claimed`, or unprompted (`None`), which may predate a move, brings
    /// none back and leaves a waiting claim to its ask.
    pub(super) fn settle(&mut self, found: u32, claimed: Option<u32>) {
        if self.ids.refuted == Some(found) {
            self.ids.refuted = None;
        }
        if let Some(claimed) = claimed.filter(|claimed| *claimed != found) {
            self.refute(claimed);
        }
        self.check(found);
        if claimed.is_some() {
            self.sender_probe(found);
            self.answered_for(Some(found));
        }
        self.release_held();
        self.undoubt();
    }

    /// The claim being asked came from a notification naming a sender
    /// other than `found`, the points' device: read whether the sender's
    /// Device object answers at the address.
    fn sender_probe(&mut self, found: u32) {
        if let Some(sender) = self.ids.sender.take().filter(|sender| *sender != found)
            && self.watched() == Some(found)
        {
            let check = Check::Alias {
                device: found,
                sender,
            };
            self.ops.push_front(Op::Check(check));
        }
    }

    /// What a read of device `sender`'s Device object found at the address
    /// of device `device`, whose notifications name `sender`: none there,
    /// the name is taken for `device`'s points for a lifetime; any other
    /// answer, they can't be told from another device's, and the points
    /// are polled instead.
    fn aliased(&mut self, device: u32, sender: u32, answer: Answer) {
        if self.watched() != Some(device) {
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
            Answer::Silent | Answer::Failed => {}
            Answer::Is(_) | Answer::Has(_) | Answer::Unclear => self.poll_instead(device, sender),
        }
    }

    /// Poll device `device`'s subscribed points: its notifications name
    /// device `sender`, which can't be told from another device at the
    /// address. Their subscriptions are cancelled; nothing is remembered of
    /// the device's COV service, so a later start subscribes and probes
    /// again.
    fn poll_instead(&mut self, device: u32, sender: u32) {
        let at = &self.address;
        tracing::info!(
            "device {device} at {at} names device {sender} in its notifications, which \
             can't be told from another device there; its points are polled"
        );
        let note = format!(
            "its notifications name device {sender}, which can't be told from another \
             device at {at}; polled instead"
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
            let state = {
                let mut entry = row.shared.lock();
                entry.snapshot.why = Some(Why::ForeignSender { sender });
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

    /// The device at the address named itself `found`: a point whose device
    /// no answer had named takes it, confirmed, and a point of another
    /// instance an answer had named is given up for it.
    fn check(&mut self, found: u32) {
        self.inner.client.note_said(&self.address, found);
        let others: BTreeSet<u32> = self
            .rows
            .values()
            .filter(|row| live(row))
            .filter_map(|row| row.shared.named(found, true))
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

    /// The device at the address answered for `instance`: the points given
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
    /// the device's last word doesn't say otherwise), or, for a following
    /// point, while the device there had named another instance itself,
    /// and the device is being asked which it is. Then nothing of the start
    /// is the point's (its subscription, if it made one, isn't kept), and
    /// the point waits for that answer; between asks, the start stands.
    pub(super) fn started_elsewhere(&mut self, process: u32, started: &Started) -> bool {
        // A following point's instance is its own (#2795): the device's own
        // word naming another is another device there, never the point's.
        let follows = self
            .rows
            .get(&process)
            .is_some_and(|row| row.shared.point.device.instance().is_some());
        let found = match (follows, started.before, started.vouched) {
            (true, Some(before), true) => Some(before),
            _ => started.instance,
        };
        let stale = found.is_some_and(|found| self.stale(found));
        let Some(row) = self.rows.get_mut(&process) else {
            return false;
        };
        let (learned, vouched) = match started.before {
            Some(before) => (Some(before), started.vouched),
            None => (started.instance, false),
        };
        let identity = row.shared.learn(learned, vouched);
        let (Some(watched), Some(found)) = (identity, found) else {
            return false;
        };
        // The device's own word, for a following point, is never stale.
        if watched == found || (stale && !(follows && vouched)) {
            return false;
        }
        if row.removing || row.shared.is_stopped() {
            let row = self.rows.remove(&process).expect("looked up above");
            self.finish(row);
            return true;
        }
        if follows && vouched {
            // The device here named itself another instance, answering
            // the client: the followed device isn't here. Nothing of the
            // start is the point's (it asked no subscription, `jobs::start`),
            // and the point is unreachable here, replaced as the device's
            // answer says, while the hub looks for its device.
            row.mode = Mode::Held;
            row.maybe_subscribed |= matches!(started.outcome, Outcome::Asked(Ok(_)));
            self.check(found);
            return true;
        }
        self.claim(found);
        if !self.ids.confirming {
            return false;
        }
        let Some(row) = self.rows.get_mut(&process) else {
            return false;
        };
        row.mode = Mode::Held;
        row.maybe_subscribed |= matches!(started.outcome, Outcome::Asked(Ok(_)));
        true
    }

    /// What a poll said of which device answered, before its rows are
    /// taken: the device's own answer settles which it is; without one, the
    /// device is asked its name if none named it, and when the table names
    /// another instance there than some points' (unless the device's last
    /// word says otherwise, or it refuted that one), the device is asked
    /// which it is, those points' rows skipped while that ask runs. The
    /// points skipped.
    pub(super) fn poll_identity(&mut self, polled: &Polled) -> BTreeSet<u32> {
        if let Some(found) = polled.answered {
            self.settle(found, None);
            return BTreeSet::new();
        }
        if self.known().is_none() && !std::mem::replace(&mut self.ids.named_asked, true) {
            self.name_device();
        }
        let Some(found) = polled.table.filter(|found| !self.stale(*found)) else {
            return BTreeSet::new();
        };
        let others: BTreeSet<u32> = polled
            .rows
            .iter()
            .filter_map(|(process, _)| self.rows.get(process))
            .filter(|row| row.shared.identity().is_some_and(|i| i != found))
            .map(|row| row.shared.process)
            .collect();
        if others.is_empty() {
            return others;
        }
        self.claim(found);
        match self.ids.confirming {
            true => others,
            false => BTreeSet::new(),
        }
    }

    /// A point of the device was subscribed: the first time, its restart
    /// clock starts and its Time_Of_Device_Restart is read once, so its
    /// first restart can be told from it; and the device is asked its name
    /// if no answer named it.
    pub(super) fn subscribed(&mut self) {
        self.ids.restarted.get_or_insert_with(Instant::now);
        if self.ids.restart_time.is_none() && !self.ids.timing {
            self.read_restart_time();
        }
        if self.known().is_none() && !std::mem::replace(&mut self.ids.named_asked, true) {
            self.name_device();
        }
    }

    /// Read the points' device's Time_Of_Device_Restart.
    fn read_restart_time(&mut self) {
        let Some(instance) = self.watched() else {
            return;
        };
        self.ids.timing = true;
        self.ops
            .push_back(Op::Check(Check::RestartTime { instance }));
    }

    /// Run the identity checks due by `now`: a claim whose wait is over,
    /// and a restart check.
    pub(super) fn identity_due(&mut self, now: Instant) {
        let ready = self.ids.pace.ready(now);
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
    /// its restart time, when it is the points' device.
    fn restart_notice(&mut self, instance: u32) {
        if self.watched() == Some(instance) {
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

    /// Device `instance`'s Time_Of_Device_Restart, as read: when it moved
    /// since the last read, the device restarted. The first read only
    /// learns it; one finding no such Device object learns nothing, and
    /// asks the device its name; one finding no such property is kept only
    /// for an instance an answer named.
    fn timed(&mut self, instance: u32, time: Restart) {
        self.ids.timing = false;
        match time {
            Restart::Silent => {}
            Restart::NoDevice => self.name_device(),
            // Of an instance no answer named (a forged row's, say): it may
            // not be the device's, so it doesn't turn restart reads off.
            Restart::Absent if self.known() != Some(instance) => {}
            time => {
                let before = self.ids.restart_time.replace(time.clone());
                if let (Some(Restart::At(before)), Restart::At(now)) = (before, time)
                    && before != now
                {
                    self.resubscribe();
                }
            }
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
