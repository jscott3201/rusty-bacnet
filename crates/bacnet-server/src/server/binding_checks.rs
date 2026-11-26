//! What an I-Am the server hears does to its bindings (#2825), and the
//! checks of a bound device's address that one it did not ask for cues.
//!
//! Any station on the link can send an I-Am naming any device, so an I-Am
//! alone settles nothing. The server binds a device only from an I-Am that
//! answers a Who-Is of its own for that device ([`super::binding_probes`]).
//! An I-Am from a device's bound address refreshes its binding. One naming
//! a bound device from another address, unasked, is a cue for a check
//! (decision, maintainer, 2026-11-26), run by a task of its own
//! ([`run`]): the server sends the claimant a Who-Is for the device's
//! instance, and asks the bound address which device it is, with a
//! confirmed ReadProperty of the Device object's Object_Identifier by the
//! wildcard instance, or by the bound instance where the wildcard read is
//! refused. The check fails when that read times out after its retries,
//! names another instance, or finds no Device object of the bound instance
//! there, and it is a strike only when the claimant answered the Who-Is, no
//! third address claimed the device meanwhile, and the bound address
//! answered none of the server's requests meanwhile ([`Checked`]). A
//! strike schedules a second check a hold-off later, and the binding moves
//! to the claimant only when that one strikes too. Anything else ends the
//! checks with the binding where it was: the bound address naming the
//! device, or answering otherwise (a Reject, an Abort, another Error), a
//! check whose requests did not all go out, the binding changing or
//! lapsing meanwhile, or the server stopping. Sends to the device keep
//! going to the bound address throughout. A configured binding is never
//! changed by traffic.
//!
//! A device the server has asked nothing about binds nothing from an I-Am,
//! and one whose binding has lapsed is asked again where it was last seen:
//! the old address answering takes the binding back at once, and another
//! takes it only once the probe's wait passes with the old one silent
//! ([`DeviceBindingTable::after_probe`]).
//!
//! Checks are paced: one per device per [`CHECK_HOLD_OFF`] from a cue, the
//! second of a pair a hold-off after the first's strike, at most
//! [`MAX_CHECKS`] devices held at once (a cue finding them all held starts
//! nothing, and none is given up for it), none while
//! DeviceCommunicationControl restricts initiation, and at most
//! [`MAX_CHECK_READS`] reads out at once, so the checks never hold more
//! than that many invoke IDs.

use super::super::binding_probes::MAX_PROBE_WAIT;
use super::*;
use tokio::sync::Semaphore;
use tokio::time::Instant as TokioInstant;

#[path = "binding_check_run.rs"]
mod run;
pub(in crate::server) use run::{CheckHandles, start_check};

/// The least time between two checks of one device begun by cues, and
/// between a check's strike and the second check.
pub(in crate::server) const CHECK_HOLD_OFF: Duration = Duration::from_secs(60);

/// The most devices with a check out, between its two checks, or held off,
/// at once.
pub(in crate::server) const MAX_CHECKS: usize = 64;

/// The most reads the checks have out at once.
pub(in crate::server) const MAX_CHECK_READS: usize = 16;

/// How long a check out or between its two checks holds its place with no
/// word from its task, which only a panic leaves unsaid.
const RUN_LIMIT: Duration = OBSERVED_BINDING_TTL;

/// Where one device's checks stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// A check is cued or out: its claimant's answers, and other
    /// addresses' claims, count from when it begins
    /// ([`DeviceBindingTable::check_begins`]).
    Running,
    /// The first check struck; the second is due a hold-off later.
    Waiting,
    /// Over; kept only for the hold-off.
    Ended,
}

/// One device's checks.
#[derive(Debug)]
pub(super) struct Check {
    /// Tells these checks from later ones of the same device.
    id: u64,
    /// When the latest check began; its hold-off counts from here.
    began: Instant,
    phase: Phase,
    /// When the first check struck, while the second is due or out.
    struck: Option<Instant>,
    /// The bound address it asks.
    bound: DeviceBindingTarget,
    /// The address that claimed the device.
    claimant: DeviceBindingTarget,
    /// When the claimant last answered as the device during the check out.
    claimant_answered: Option<Instant>,
    /// A third address claimed the device during the check out.
    contested: bool,
}

impl Check {
    /// Whether it still holds back another check of its device at `now`.
    fn holds_at(&self, now: Instant) -> bool {
        let age = now.saturating_duration_since(self.began);
        match self.phase {
            Phase::Running | Phase::Waiting => age < RUN_LIMIT,
            Phase::Ended => age < CHECK_HOLD_OFF,
        }
    }
}

/// A check its task runs ([`start_check`]), reporting to
/// [`DeviceBindingTable::check_begins`] and
/// [`DeviceBindingTable::check_ended`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::server) struct CheckStart {
    pub(in crate::server) device: ObjectIdentifier,
    id: u64,
    pub(in crate::server) claimant: DeviceBindingTarget,
    /// How long the claimant's answer counts for, at least, from its
    /// Who-Is.
    pub(in crate::server) wait: Duration,
    pub(in crate::server) reads: ReadSlots,
}

/// The slots the checks' reads take ([`MAX_CHECK_READS`]).
#[derive(Debug, Clone)]
pub(in crate::server) struct ReadSlots(pub(in crate::server) Arc<Semaphore>);

impl PartialEq for ReadSlots {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ReadSlots {}

/// The checks out, between their two checks or held off.
#[derive(Debug)]
pub(super) struct BindingChecks {
    checks: HashMap<ObjectIdentifier, Check>,
    next_id: u64,
    reads: ReadSlots,
}

impl Default for BindingChecks {
    fn default() -> Self {
        Self {
            checks: HashMap::new(),
            next_id: 0,
            reads: ReadSlots(Arc::new(Semaphore::new(MAX_CHECK_READS))),
        }
    }
}

/// What the bound address said of itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::server) enum Identity {
    /// It named itself this instance.
    Is(u32),
    /// It has no Device object of the bound instance: it refused the
    /// wildcard read, then the read by the instance with an unknown object.
    NotThere,
    /// It answered, but with something that says neither: a Reject, an
    /// Abort or another Error, or no answer to the second read after it
    /// refused the first.
    Unclear,
    /// Every attempt went out and none drew an answer.
    Silent,
    /// Not every request went out: DeviceCommunicationControl, a send the
    /// link failed, no free invoke ID, the server stopping, or the binding
    /// changing meanwhile.
    NotAsked,
}

impl Identity {
    /// Whether this answer says the bound address no longer holds
    /// `instance`.
    fn fails(self, instance: u32) -> bool {
        match self {
            Self::Is(named) => named != instance,
            Self::NotThere | Self::Silent => true,
            Self::Unclear | Self::NotAsked => false,
        }
    }
}

/// What one check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::server) struct Checked {
    /// What the bound address said of itself.
    pub(in crate::server) identity: Identity,
    /// Whether the bound address answered any other request of the
    /// server's since the first check began (#2825): an answer to the
    /// server's own request, never a frame it sent unasked.
    pub(in crate::server) heard: bool,
}

impl Checked {
    /// A check whose requests did not all go out.
    pub(in crate::server) const NOT_ASKED: Self = Self {
        identity: Identity::NotAsked,
        heard: false,
    };
}

/// What a check's task does once a check has ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::server) enum Step {
    /// Nothing more: the binding stays where it is.
    Done,
    /// It struck: check again after [`CHECK_HOLD_OFF`].
    Again,
    /// Both checks struck: the binding moved from the first to the second.
    Moved(DeviceBindingTarget, DeviceBindingTarget),
}

/// Where an I-Am came from: its link source, the SNET and SADR a router
/// added, and this network's own number as the receiving link knows it.
#[derive(Debug, Clone, Copy)]
pub(in crate::server) struct IAmFrom<'a> {
    pub(in crate::server) mac: &'a [u8],
    pub(in crate::server) routed: Option<&'a NpduAddress>,
    pub(in crate::server) local_network: Option<u16>,
}

/// Whether `a` and `b` name one station: equal, or one MAC on this network,
/// bound through this network's own number on one side and heard straight
/// on the other (#1358, #1404).
fn same_station(a: &DeviceBindingTarget, b: &DeviceBindingTarget, local: Option<u16>) -> bool {
    let here = |target: &DeviceBindingTarget| match target {
        DeviceBindingTarget::Local { peer_mac } => Some(peer_mac.clone()),
        DeviceBindingTarget::Routed {
            network, final_mac, ..
        } if Some(*network) == local => Some(final_mac.clone()),
        DeviceBindingTarget::Routed { .. } => None,
    };
    a == b || here(a).is_some_and(|mac| here(b) == Some(mac))
}

/// What an I-Am the server heard did (#2825).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::server) enum Heard {
    /// It names no Device, or no address this link can reach.
    RejectedInvalid,
    /// It answered a Who-Is of the server's, but the table is full.
    RejectedCapacity,
    /// A configured binding names the device; traffic never changes one.
    ConfiguredPreserved,
    /// It answered the server's own Who-Is for a device with no binding,
    /// and binds the device there.
    Inserted,
    /// It came from the device's bound address and refreshes its binding.
    Refreshed,
    /// It names the device from another address while a question of the
    /// server's about the device is out: that address's answer, which
    /// counts once the question is decided.
    Claimed,
    /// It names a bound device from another address, unasked: the server
    /// checks the bound address ([`start_check`]), the binding unchanged
    /// meanwhile.
    Cue(CheckStart),
    /// It decides nothing: the server asked nothing about the device, a
    /// check of it is out, due or held off, the checks are full, or the
    /// server may not ask.
    Ignored,
}

impl DeviceBindingTable {
    /// Hear Device `device`'s I-Am `from` a station at `now` (#2825).
    /// `checking` is how long a check it cues counts the claimant's answer
    /// for, or `None` when the server may not ask:
    /// DeviceCommunicationControl restricts initiation.
    pub(in crate::server) fn hear_i_am_at(
        &mut self,
        device: ObjectIdentifier,
        from: IAmFrom<'_>,
        now: Instant,
        is_group: impl Fn(&[u8]) -> bool,
        checking: Option<Duration>,
    ) -> Heard {
        let Some(target) = observed_target(device, from.mac, from.routed, &is_group) else {
            return Heard::RejectedInvalid;
        };
        let asked = self.probes.asking_at(&device, TokioInstant::from_std(now));
        let (bound, stale) = match self.entries.get(&device) {
            Some(BindingEntry::Configured(_)) => return Heard::ConfiguredPreserved,
            Some(BindingEntry::Observed {
                target: bound,
                observed_at,
            }) => (
                bound.clone(),
                now.saturating_duration_since(*observed_at) >= OBSERVED_BINDING_TTL,
            ),
            // First contact is decided when the wait ends
            // (`after_probe`): one station answered, it is bound; more,
            // none is.
            None if asked => {
                self.probes.claim(&device, target, now);
                return Heard::Claimed;
            }
            // A late answer still binds while the probe holds the device
            // back, if no other station answered the question.
            None if self
                .probes
                .asked_lately_at(&device, TokioInstant::from_std(now)) =>
            {
                if !self.probes.sole_answer(&device, &target) {
                    self.probes.claim(&device, target, now);
                    return Heard::Claimed;
                }
                return match self.record_observation(device, target, now) {
                    ObservationOutcome::Inserted => {
                        self.probes.answer(&device);
                        Heard::Inserted
                    }
                    _ => Heard::RejectedCapacity,
                };
            }
            None => return Heard::Ignored,
        };
        if same_station(&bound, &target, from.local_network) {
            // A frame it sent unasked: no word for a check, which reads
            // the address instead.
            self.record_observation(device, bound, now);
            self.probes.answer(&device);
            return Heard::Refreshed;
        }
        if asked {
            self.probes.claim(&device, target, now);
            return Heard::Claimed;
        }
        if let Some(check) = self.checks.checks.get_mut(&device)
            && check.phase == Phase::Running
        {
            if same_station(&check.claimant, &target, from.local_network) {
                check.claimant_answered = Some(now);
            } else {
                check.contested = true;
            }
            return Heard::Claimed;
        }
        match checking {
            Some(wait) if !stale => self
                .checks
                .begin(device, (bound, target), now, wait.min(MAX_PROBE_WAIT))
                .map_or(Heard::Ignored, Heard::Cue),
            _ => Heard::Ignored,
        }
    }

    /// `start`'s next check begins at `now`, its claimant's Who-Is about
    /// to go: `device`'s binding as the check reads it, or `None` when the
    /// check is no longer current, or the binding moved, lapsed or went
    /// since it was cued, which ends it undecided. The claimant's answers
    /// count from here.
    pub(in crate::server) fn check_begins(
        &mut self,
        start: &CheckStart,
        now: Instant,
        is_group: impl Fn(&[u8]) -> bool,
    ) -> Option<DeviceResolution> {
        let check = self
            .checks
            .checks
            .get_mut(&start.device)
            .filter(|check| check.id == start.id)?;
        let still_bound = matches!(
            self.entries.get(&start.device),
            Some(BindingEntry::Observed { target, observed_at })
                if *target == check.bound
                    && now.saturating_duration_since(*observed_at) < OBSERVED_BINDING_TTL
        );
        if !still_bound || !matches!(check.phase, Phase::Running | Phase::Waiting) {
            return None;
        }
        if check.phase == Phase::Waiting {
            check.began = now;
        }
        check.phase = Phase::Running;
        check.claimant_answered = None;
        check.contested = false;
        Some(self.resolve_at(&start.device, now, is_group))
    }

    /// `start`'s check ended at `now`, finding `checked`: decide it.
    pub(in crate::server) fn check_ended(
        &mut self,
        start: &CheckStart,
        checked: Checked,
        now: Instant,
    ) -> Step {
        let Some(check) = self
            .checks
            .checks
            .get_mut(&start.device)
            .filter(|check| check.id == start.id && check.phase != Phase::Ended)
        else {
            return Step::Done;
        };
        let strike = checked.identity.fails(start.device.instance_number())
            && check.claimant_answered.is_some()
            && !check.contested
            && !checked.heard;
        check.phase = Phase::Ended;
        let first = check.struck.take();
        if !strike {
            // The bound address named the device in an answer to the
            // server's own read: as fresh as an answer to its Who-Is.
            if checked.identity == Identity::Is(start.device.instance_number())
                && let Some(BindingEntry::Observed {
                    target,
                    observed_at,
                }) = self.entries.get_mut(&start.device)
                && *target == check.bound
            {
                *observed_at = now;
            }
            return Step::Done;
        }
        let Some(first) = first else {
            check.struck = Some(now);
            check.phase = Phase::Waiting;
            debug!(device = %start.device, "A check found the device's bound address failing; checking again in a minute");
            return Step::Again;
        };
        let (bound, claimant, at) = (
            check.bound.clone(),
            check.claimant.clone(),
            check.claimant_answered.unwrap_or(now),
        );
        if now.saturating_duration_since(first) < CHECK_HOLD_OFF {
            return Step::Done;
        }
        match self.entries.get_mut(&start.device) {
            Some(BindingEntry::Observed {
                target,
                observed_at,
            }) if *target == bound => {
                *target = claimant.clone();
                *observed_at = at;
                Step::Moved(bound, claimant)
            }
            _ => Step::Done,
        }
    }
}

impl BindingChecks {
    /// Begin a check of `device`, bound at the first of `addresses` and
    /// claimed from the second, the claimant's answer counting for `wait`,
    /// unless one is out, due or held off, or the checks are full.
    fn begin(
        &mut self,
        device: ObjectIdentifier,
        (bound, claimant): (DeviceBindingTarget, DeviceBindingTarget),
        now: Instant,
        wait: Duration,
    ) -> Option<CheckStart> {
        if self
            .checks
            .get(&device)
            .is_some_and(|check| check.holds_at(now))
        {
            return None;
        }
        // Only places that hold nothing back go: a check between its two
        // checks is never given up for a new cue.
        self.checks.retain(|_, check| check.holds_at(now));
        if self.checks.len() >= MAX_CHECKS {
            return None;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let start = CheckStart {
            device,
            id,
            claimant: claimant.clone(),
            wait,
            reads: self.reads.clone(),
        };
        self.checks.insert(
            device,
            Check {
                id,
                began: now,
                phase: Phase::Running,
                struck: None,
                bound,
                claimant,
                claimant_answered: None,
                contested: false,
            },
        );
        Some(start)
    }
}

#[cfg(test)]
#[path = "binding_checks_tests.rs"]
mod tests;
