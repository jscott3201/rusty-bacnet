//! What an I-Am the server hears does to its bindings (#2825), and the
//! checks of a bound device's address that one it did not ask for cues.
//!
//! Any station on the link can send an I-Am naming any device, so an I-Am
//! alone settles nothing. The server binds a device only from an I-Am that
//! answers a Who-Is of its own for that device ([`super::binding_probes`]).
//! An I-Am from a device's bound address refreshes its binding. One naming
//! a bound device from another address, unasked, is a cue for a check: the
//! server sends a Who-Is for the device's instance to the bound address and
//! one to the claimant, and waits. The binding moves to the claimant only
//! when the bound address stays silent through the wait (a station there
//! that is now another device draws nothing from a Who-Is for this one) and
//! the claimant answers. Both answering keeps the binding; so does a third
//! address claiming the device during the wait: two claimants decide
//! nothing. Sends to the device keep going to the bound address meanwhile.
//! A configured binding is never changed by traffic.
//!
//! A device the server has asked nothing about binds nothing from an I-Am,
//! and one whose binding has lapsed is asked again where it was last seen:
//! the old address answering takes the binding back at once, and another
//! takes it only once the probe's wait passes with the old one silent
//! ([`DeviceBindingTable::after_probe`]).
//!
//! Checks are paced: one per device per [`CHECK_HOLD_OFF`], at most
//! [`MAX_CHECKS`] held at once, none while DeviceCommunicationControl
//! restricts initiation. A flood of forged I-Ams therefore costs at most two
//! directed Who-Is per bound device per hold-off, and at most
//! `2 * MAX_CHECKS` in any hold-off. A check decides when its wait is over:
//! readers see a move from then on ([`DeviceBindingTable::current`]), and
//! the next I-Am writes it into the table
//! ([`DeviceBindingTable::settle_checks`]). Nothing waits on a check, so no
//! task outlives one.

use super::super::binding_probes::{MAX_PROBE_WAIT, targeted_who_is};
use super::*;
use tokio::time::Instant as TokioInstant;

/// The least time between two checks of one device.
pub(in crate::server) const CHECK_HOLD_OFF: Duration = Duration::from_secs(60);

/// The most devices with a check out or held off at once.
pub(in crate::server) const MAX_CHECKS: usize = 64;

/// One device's check.
#[derive(Debug)]
pub(super) struct Check {
    /// Tells this check from a later one of the same device.
    id: u64,
    /// When it began; its hold-off counts from here.
    began: Instant,
    /// How long its answers count once its Who-Is went out.
    wait: Duration,
    /// When its answers stop counting: set once its Who-Is went out, so a
    /// send held up (an MS/TP station waiting for the token) costs the
    /// bound device none of its wait.
    deadline: Option<Instant>,
    /// Whether both its Who-Is went out: one that asked nothing moves
    /// nothing.
    asked: bool,
    /// The bound address it asks.
    bound: DeviceBindingTarget,
    /// The address that claimed the device.
    claimant: DeviceBindingTarget,
    /// The bound address answered as the device.
    bound_answered: bool,
    /// When the claimant last answered as the device.
    claimant_answered: Option<Instant>,
    /// A third address claimed the device meanwhile.
    contested: bool,
    /// Decided and written into the table; kept only for the hold-off.
    settled: bool,
}

impl Check {
    /// Whether answers heard at `now` count.
    fn open_at(&self, now: Instant) -> bool {
        !self.settled && self.deadline.is_none_or(|deadline| now < deadline)
    }

    /// Whether its wait is over at `now`.
    fn over_at(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }

    /// Where the device is bound once this check is over at `now`, if it
    /// moved it: to the claimant, when the bound address stayed silent and
    /// the claimant alone answered, its answer the new observation.
    fn moved_at(&self, now: Instant) -> Option<(&DeviceBindingTarget, Instant)> {
        if self.settled
            || !self.asked
            || !self.over_at(now)
            || self.bound_answered
            || self.contested
        {
            return None;
        }
        self.claimant_answered.map(|at| (&self.claimant, at))
    }

    /// Whether it still holds back another check of its device.
    fn holds_at(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now < deadline)
            || now.saturating_duration_since(self.began) < CHECK_HOLD_OFF
    }
}

/// A check whose two Who-Is the caller sends ([`ask`]), then reports with
/// [`DeviceBindingTable::check_asked`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::server) struct CheckStart {
    device: ObjectIdentifier,
    id: u64,
    bound: DeviceBindingTarget,
    claimant: DeviceBindingTarget,
}

/// The checks out or held off.
#[derive(Debug, Default)]
pub(super) struct BindingChecks {
    checks: HashMap<ObjectIdentifier, Check>,
    next_id: u64,
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
    /// counts once the question's wait is over.
    Claimed,
    /// It names a bound device from another address, unasked: the server
    /// checks both addresses ([`ask`]), the binding unchanged meanwhile.
    Cue(CheckStart),
    /// It decides nothing: the server asked nothing about the device, a
    /// check of it is held off or the checks are full, or the server may
    /// not ask.
    Ignored,
}

impl DeviceBindingTable {
    /// Hear Device `device`'s I-Am `from` a station at `now` (#2825).
    /// `checking` is how long a check it cues waits for its answers, or
    /// `None` when the server may not send a check's Who-Is:
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
        self.settle_checks(now);
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
            self.record_observation(device, bound, now);
            self.probes.answer(&device);
            if let Some(check) = self.checks.checks.get_mut(&device)
                && check.open_at(now)
            {
                check.bound_answered = true;
            }
            return Heard::Refreshed;
        }
        if asked {
            self.probes.claim(&device, target, now);
            return Heard::Claimed;
        }
        if let Some(check) = self.checks.checks.get_mut(&device)
            && check.open_at(now)
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
                .begin(device, (bound, target), now, wait)
                .map_or(Heard::Ignored, Heard::Cue),
            _ => Heard::Ignored,
        }
    }

    /// Record whether `start`'s two Who-Is went out, at `now`: its wait
    /// counts from here. A check whose Who-Is did not all go out ends
    /// undecided: one that asked nothing never moves a binding.
    pub(in crate::server) fn check_asked(&mut self, start: &CheckStart, sent: bool, now: Instant) {
        if let Some(check) = self
            .checks
            .checks
            .get_mut(&start.device)
            .filter(|check| check.id == start.id)
        {
            check.asked = sent;
            check.settled |= !sent;
            if sent {
                check.deadline = Some(now + check.wait.min(MAX_PROBE_WAIT));
            }
        }
    }

    /// Write every check whose wait is over at `now` into the table: a move
    /// it decided rebinds its device, if the device is still bound where
    /// the check found it.
    pub(super) fn settle_checks(&mut self, now: Instant) {
        let mut moves = Vec::new();
        for (device, check) in &mut self.checks.checks {
            if let Some((claimant, at)) = check.moved_at(now) {
                moves.push((*device, check.bound.clone(), claimant.clone(), at));
            }
            check.settled |= check.over_at(now);
        }
        for (device, bound, claimant, at) in moves {
            if let Some(BindingEntry::Observed {
                target,
                observed_at,
            }) = self.entries.get_mut(&device)
                && *target == bound
            {
                debug!(%device, "A check moved the device's binding: its old address was silent");
                *target = claimant;
                *observed_at = at;
            }
        }
    }

    /// `device`'s binding target, and when it was last observed (`None`
    /// for a configured one), with a decided check's move applied: what
    /// every reader of the table goes by.
    pub(super) fn current(
        &self,
        device: &ObjectIdentifier,
        now: Instant,
    ) -> Option<(&DeviceBindingTarget, Option<Instant>)> {
        match self.entries.get(device)? {
            BindingEntry::Configured(target) => Some((target, None)),
            BindingEntry::Observed {
                target,
                observed_at,
            } => Some(
                self.checks
                    .checks
                    .get(device)
                    .filter(|check| check.bound == *target)
                    .and_then(|check| check.moved_at(now))
                    .map_or((target, Some(*observed_at)), |(moved, at)| {
                        (moved, Some(at))
                    }),
            ),
        }
    }
}

impl BindingChecks {
    /// Begin a check of `device`, bound at the first of `addresses` and
    /// claimed from the second, its answers counting for `wait`, unless
    /// one is held off or the checks are full.
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
        // Every check whose wait is over was settled before this.
        self.checks.retain(|_, check| check.holds_at(now));
        if self.checks.len() >= MAX_CHECKS {
            return None;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let start = CheckStart {
            device,
            id,
            bound: bound.clone(),
            claimant: claimant.clone(),
        };
        self.checks.insert(
            device,
            Check {
                id,
                began: now,
                wait,
                deadline: None,
                asked: false,
                bound,
                claimant,
                bound_answered: false,
                claimant_answered: None,
                contested: false,
                settled: false,
            },
        );
        Some(start)
    }
}

/// Send `start`'s two Who-Is, each limited to its device's instance: to
/// the bound address and to the claimant, straight to a station on this
/// link or through its router to one on another network. Whether both went
/// out; nothing is sent while DeviceCommunicationControl restricts
/// initiation.
pub(in crate::server) async fn ask<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    comm_state: &CommState,
    start: &CheckStart,
) -> bool {
    let apdu = targeted_who_is(start.device);
    let local_network = network.local_network_number().get();
    let priority = NetworkPriority::Normal;
    let mut sent = true;
    for target in [&start.bound, &start.claimant] {
        if comm_state.initiation_restricted() {
            return false;
        }
        sent &= match target {
            DeviceBindingTarget::Local { peer_mac } => {
                network.send_apdu(&apdu, peer_mac, false, priority).await
            }
            // A binding through this network's own number is the local
            // station it names (#1358).
            DeviceBindingTarget::Routed {
                network: dnet,
                final_mac,
                ..
            } if Some(*dnet) == local_network => {
                if network.transport().is_group_destination(final_mac) {
                    return false;
                }
                network.send_apdu(&apdu, final_mac, false, priority).await
            }
            DeviceBindingTarget::Routed {
                network: dnet,
                final_mac,
                router_mac,
                ..
            } => {
                network
                    .send_apdu_routed(&apdu, *dnet, final_mac, router_mac, false, priority)
                    .await
            }
        }
        .is_ok();
    }
    sent
}

#[cfg(test)]
#[path = "binding_checks_tests.rs"]
mod tests;
