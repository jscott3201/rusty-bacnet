//! The invariants checked continuously while a scenario runs.

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::MasterState;

use super::master::PendingTx;
use super::{NodeKind, Sim};
use crate::line::{Driver, TxId};
use crate::report::{Collision, CollisionClass, Episode, ViolationKind, WireFrame};
use crate::trace::{Millis, TraceKind, Who};

/// States in which a master acts on the token it holds. A master in
/// PASS_TOKEN has handed it on and is only watching for the successor to use
/// it, so it is not counted.
pub(crate) fn holds_token(state: MasterState) -> bool {
    matches!(
        state,
        MasterState::UseToken
            | MasterState::DoneWithToken
            | MasterState::WaitForReply
            | MasterState::PollForMaster
    )
}

impl Sim {
    /// Every frame decodes unless a fault touched it, and, until the run is
    /// perturbed, every master that heard a clean frame whole counted it.
    /// A master without extended frames counts no extended frame as
    /// received: it can't take one, as a node without COBS support can't.
    pub(super) fn check_frame(&mut self, frame: &WireFrame, listeners: &[(usize, u32)]) {
        let by_master = matches!(frame.driver, Driver::Station(station)
            if matches!(self.nodes[self.index_of(station)].kind, NodeKind::Master(_)));
        if by_master && frame.is_clean() && frame.header.is_none() {
            self.violation(
                ViolationKind::Undecodable,
                format!(
                    "{:?} sent {} octets that do not decode",
                    frame.driver,
                    frame.octets.len()
                ),
            );
        }
        if self.perturbed || !frame.is_clean() || frame.header.is_none() {
            return;
        }
        let mut missed = Vec::new();
        for &(node, before) in listeners {
            let station = self.nodes[node].station;
            let listened_throughout = self.nodes[node]
                .listening_since
                .is_some_and(|since| since <= frame.start);
            let NodeKind::Master(master) = &self.nodes[node].kind else {
                continue;
            };
            let Some(core) = master.core.as_ref() else {
                continue;
            };
            let after = core.counters().frames_received;
            let takes = core.config().extended_frames
                || !frame.frame_type().is_some_and(FrameType::is_cobs);
            if listened_throughout && after != before + u32::from(takes) {
                missed.push(format!("station {station} counted {}", after - before));
            }
        }
        if !missed.is_empty() {
            self.violation(
                ViolationKind::ListenerMissedFrame,
                format!("clean frame {frame}: {}", missed.join(", ")),
            );
        }
    }

    /// Two drivers' octets overlap: explain it or fail.
    pub(super) fn on_overlap(&mut self, tx: TxId, driver: Driver, other: Driver) {
        let already = self
            .meta
            .get(&tx)
            .is_some_and(|meta| meta.collided_with.contains(&other));
        if already {
            return;
        }
        if let Some(meta) = self.meta.get_mut(&tx) {
            meta.collided_with.push(other);
        }
        let other_tx = match other {
            Driver::Station(station) => self.nodes[self.index_of(station)].driving,
            Driver::Noise(_) => None,
        };
        if let Some(meta) = other_tx.and_then(|other_tx| self.meta.get_mut(&other_tx)) {
            meta.collided_with.push(driver);
        }
        let class = self.classify(tx, driver, other_tx, other);
        let collision = Collision {
            at: self.now,
            driver,
            other,
            class,
        };
        self.report.collisions.push(collision);
        self.note(
            Who::Sim,
            TraceKind::Note(format!("collision {driver:?} over {other:?}: {class:?}")),
        );
        self.perturb("a collision");
        let tolerated = match class {
            CollisionClass::Noise | CollisionClass::GenerationRace => true,
            CollisionClass::SlowAdapter => self.config.tolerance.slow_adapter_collisions,
            CollisionClass::Scripted => self.config.tolerance.scripted_collisions,
            CollisionClass::Unexplained => false,
        };
        if !tolerated {
            self.violation(
                ViolationKind::Collision,
                format!("{driver:?} started an octet over {other:?} ({class:?})"),
            );
        }
    }

    fn classify(
        &self,
        tx: TxId,
        driver: Driver,
        other_tx: Option<TxId>,
        other: Driver,
    ) -> CollisionClass {
        if matches!(driver, Driver::Noise(_)) || matches!(other, Driver::Noise(_)) {
            return CollisionClass::Noise;
        }
        let master_of = |driver: Driver| match driver {
            Driver::Station(station) => match &self.nodes[self.index_of(station)].kind {
                NodeKind::Master(master) => Some(master),
                NodeKind::Scripted(_) => None,
            },
            Driver::Noise(_) => None,
        };
        let (Some(first), Some(second)) = (master_of(driver), master_of(other)) else {
            return CollisionClass::Scripted;
        };
        // A slow adapter explains the overlap only if its core handed the
        // frame over before the other frame's first character arrived: until
        // then the core could not have heard it, and only the adapter's delay
        // put the two together. This is the reference point the turnaround
        // check uses. A core that dispatched after hearing the other frame is
        // at fault whatever its adapter.
        let dispatched = |tx: Option<TxId>| tx.and_then(|tx| self.meta.get(&tx)?.dispatched);
        let first_heard = |tx: Option<TxId>| {
            tx.and_then(|tx| self.line.get(tx))
                .map(|t| t.start + self.timing.octet())
        };
        let slow_first = |slow: &super::master::MasterNode, own: Option<TxId>, theirs| {
            slow.spec.adapter.is_slow()
                && matches!((dispatched(own), first_heard(theirs)), (Some(at), Some(heard)) if at < heard)
        };
        if first.generating && second.generating {
            CollisionClass::GenerationRace
        } else if slow_first(first, Some(tx), other_tx) || slow_first(second, other_tx, Some(tx)) {
            CollisionClass::SlowAdapter
        } else {
            CollisionClass::Unexplained
        }
    }

    /// A master starts a frame no sooner than its Tturnaround after the last
    /// character on the line, its own included, and its core hands the frame
    /// over no sooner than that either. Every character reaches the core
    /// before it confirms a start, so neither has an excuse (#1625).
    pub(super) fn check_turnaround(&mut self, node: usize, pending: &PendingTx) {
        let now = self.now;
        let station = self.nodes[node].station;
        let NodeKind::Master(master) = &self.nodes[node].kind else {
            return;
        };
        let turnaround = master.spec.config.t_turnaround;
        if let Some(before) = pending.activity_at_dispatch {
            if pending.dispatched - before < turnaround {
                self.violation(
                    ViolationKind::Turnaround,
                    format!(
                        "station {station}'s core handed a frame over {} us after line activity at {}, Tturnaround {} us",
                        (pending.dispatched - before).as_micros(),
                        Millis(before),
                        turnaround.as_micros()
                    ),
                );
                return;
            }
        }
        let Some(last) = self.line.last_activity() else {
            return;
        };
        if now - last >= turnaround {
            return;
        }
        let detail = format!(
            "station {station} started {} us after line activity at {}, Tturnaround {} us",
            (now - last).as_micros(),
            Millis(last),
            turnaround.as_micros()
        );
        self.violation(ViolationKind::Turnaround, detail);
    }

    /// A punctual master answers a Token or Poll For Master addressed to it
    /// within Tusage_delay, when nothing else has happened on the line since.
    /// A master answering a request finishes that answer first (Clause
    /// 9.5.6.9), so its answer is not held to it.
    pub(super) fn check_usage_delay(&mut self, node: usize, pending: &PendingTx) {
        let now = self.now;
        let station = self.nodes[node].station;
        let NodeKind::Master(master) = &self.nodes[node].kind else {
            return;
        };
        if master.spec.adapter.is_slow() || pending.while_answering {
            return;
        }
        let limit = master.spec.config.t_usage_delay;
        let Some((header, end)) = self.last_clean_frame() else {
            return;
        };
        if header.destination != station
            || !matches!(
                header.frame_type,
                FrameType::TOKEN | FrameType::POLL_FOR_MASTER
            )
        {
            return;
        }
        let answering_poll = header.frame_type == FrameType::POLL_FOR_MASTER;
        let sends_reply = super::decode_whole(&pending.octets)
            .is_some_and(|(header, _)| header.frame_type == FrameType::REPLY_TO_POLL_FOR_MASTER);
        if answering_poll && !sends_reply {
            return;
        }
        if now - end > limit {
            self.violation(
                ViolationKind::UsageDelay,
                format!(
                    "station {station} answered a {:?} after {} us, Tusage_delay {} us",
                    header.frame_type,
                    (now - end).as_micros(),
                    limit.as_micros()
                ),
            );
        }
    }

    /// At most one master believes it holds the token. Masters that each
    /// generated their own token may race for up to the generation-race
    /// tolerance; after a fault, any duplicate may last up to the
    /// duplicate-token tolerance.
    pub(super) fn check_holders(&mut self) {
        let mut holders = Vec::new();
        let mut all_generating = true;
        for node in &self.nodes {
            let NodeKind::Master(master) = &node.kind else {
                continue;
            };
            if master
                .core
                .as_ref()
                .is_some_and(|core| holds_token(core.state()))
            {
                holders.push(node.station);
                all_generating &= master.generating;
            }
        }
        let now = self.now;
        if holders.len() < 2 {
            if let Some((start, race)) = self.duplicate_since.take() {
                let episode = Episode {
                    start,
                    length: now - start,
                };
                if race {
                    self.report.generation_races.push(episode);
                } else {
                    self.report.duplicate_token_episodes.push(episode);
                }
            }
            return;
        }
        let (start, race) = *self.duplicate_since.get_or_insert((now, all_generating));
        let (limit, what) = if race && all_generating {
            (self.config.tolerance.generation_race, "generated tokens")
        } else if self.perturbed {
            (
                self.config.tolerance.duplicate_token,
                "the token after a fault",
            )
        } else {
            (
                bacnet_mstp_core::Duration::ZERO,
                "the token with no fault injected",
            )
        };
        if !(race && all_generating) {
            self.duplicate_since = Some((start, false));
        }
        if now - start > limit || (!self.perturbed && !(race && all_generating)) {
            self.violation(
                ViolationKind::DuplicateToken,
                format!(
                    "stations {holders:?} have all held {what} since {}",
                    Millis(start)
                ),
            );
            self.duplicate_since = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use bacnet_mstp_codec::{Frame, FrameType};
    use bacnet_mstp_core::{Duration, Instant};

    use crate::line::Driver;
    use crate::report::{CollisionClass, ViolationKind};
    use crate::sim::{Sim, SimConfig};
    use crate::spec::{AdapterSpec, MasterSpec};

    const BAUD: u32 = 38_400;

    fn token(from: u8, to: u8) -> Vec<u8> {
        let mut wire = [0u8; 8];
        Frame::new(FrameType::TOKEN, to, from, &[])
            .encode_into(&mut wire)
            .expect("a Token encodes");
        wire.to_vec()
    }

    /// Station 0's frame starts at `t0`; station 3, whose adapter is late,
    /// starts over its second octet with a frame its core handed over
    /// `dispatched` after `t0`. Both frames go onto the line directly, not
    /// through the cores: a correct core never hands a frame over after
    /// hearing a character, because the character restarts its turnaround or
    /// cancels the frame, and such a hand-over would also fail the
    /// dispatch-time turnaround check, which this bypasses.
    fn overlap(dispatched: Duration) -> (CollisionClass, Vec<ViolationKind>) {
        let mut config = SimConfig::new(BAUD);
        config.panic_on_violation = false;
        config.tolerance.slow_adapter_collisions = true;
        let mut sim = Sim::new(config);
        let late = AdapterSpec {
            deadline_lateness: Duration::from_millis(1),
            ..AdapterSpec::default()
        };
        sim.add_master(MasterSpec::new(0, 3, BAUD))
            .add_master(MasterSpec::new(3, 3, BAUD).adapter(late));
        let octet = sim.timing.octet();
        let t0 = Instant::from_micros(1_000_000);
        sim.now = t0;
        let first =
            sim.begin_transmission(Driver::Station(0), token(0, 3), None, Some((t0, false)));
        sim.nodes[0].driving = Some(first);
        sim.octet_start(first, 0);
        sim.now = t0 + octet;
        sim.octet_start(first, 1);
        sim.now = t0 + octet + Duration::from_micros(100);
        let second = sim.begin_transmission(
            Driver::Station(3),
            token(3, 0),
            None,
            Some((t0 + dispatched, false)),
        );
        sim.nodes[1].driving = Some(second);
        sim.octet_start(second, 0);
        let kinds = sim.report.violations.iter().map(|v| v.kind).collect();
        (sim.report.collisions[0].class, kinds)
    }

    /// The reference point is the arrival of the other frame's first
    /// character, as for the turnaround check: before it the late core could
    /// not have heard the frame, so the collision is the adapter's and the
    /// tolerance covers it; from it on the core had heard the frame, and the
    /// collision is unexplained even with the tolerance on.
    #[test]
    fn a_slow_adapter_collision_needs_a_hand_over_before_the_first_character() {
        let octet = crate::wire::WireTiming::new(BAUD).octet();
        for before in [Duration::from_micros(50), octet - Duration::from_micros(1)] {
            assert_eq!(overlap(before), (CollisionClass::SlowAdapter, Vec::new()));
        }
        for after in [octet, octet + Duration::from_micros(50)] {
            assert_eq!(
                overlap(after),
                (CollisionClass::Unexplained, vec![ViolationKind::Collision])
            );
        }
    }
}
