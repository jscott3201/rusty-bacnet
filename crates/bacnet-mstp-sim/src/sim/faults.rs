//! Fault injection and node power.

use bacnet_mstp_codec::FrameType;

use super::{NodeKind, Sim};
use crate::SimRng;
use crate::line::Driver;
use crate::spec::{Corruption, Fault};
use crate::trace::{TraceKind, Who};

/// Separates each node's timer-phase draw from the run's other streams.
const TIMER_PHASE_STREAM: u64 = 0x3C6E_F372_FE94_F82B;

/// A fault waiting for the transmission it applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Armed {
    Corrupt {
        station: u8,
        frame_type: Option<FrameType>,
        offset: usize,
        corruption: Corruption,
    },
    CutOff {
        station: u8,
        after_octets: usize,
    },
}

impl Sim {
    pub(super) fn apply_fault(&mut self, fault: Fault) {
        self.note(Who::Sim, TraceKind::Fault(format!("{fault:?}")));
        match fault {
            Fault::Noise { octets } => self.noise(octets),
            Fault::CorruptNext {
                station,
                frame_type,
                offset,
                corruption,
            } => {
                self.armed.retain(
                    |armed| !matches!(armed, Armed::Corrupt { station: target, .. } if *target == station),
                );
                self.armed.push(Armed::Corrupt {
                    station,
                    frame_type,
                    offset,
                    corruption,
                });
            }
            Fault::PowerOff { station } => {
                let node = self.index_of(station);
                self.power_off(node);
            }
            Fault::PowerOn { station } => {
                let node = self.index_of(station);
                self.perturb("a node powered on");
                self.power_on(node);
            }
            Fault::PowerOffMidFrame {
                station,
                after_octets,
            } => {
                self.armed.retain(
                    |armed| !matches!(armed, Armed::CutOff { station: target, .. } if *target == station),
                );
                self.armed.push(Armed::CutOff {
                    station,
                    after_octets,
                });
            }
            Fault::Disarm => self.armed.clear(),
            Fault::SetAdapter { station, adapter } => {
                self.timing.validate_post_drive(adapter.post_drive);
                self.timing
                    .validate_inter_octet_gap(adapter.inter_octet_gap);
                if let Some(host) = adapter.host {
                    host.validate();
                }
                let node = self.index_of(station);
                if let NodeKind::Master(master) = &mut self.nodes[node].kind {
                    assert!(
                        master.core.is_none() || adapter.host.is_some() == master.host.is_some(),
                        "change host/octet mode while the node is powered off"
                    );
                    master.spec.adapter = adapter;
                    self.arm_deadline(node);
                }
            }
        }
    }

    /// Power a node on: a master boots a fresh core, a scripted station
    /// starts listening.
    pub(super) fn power_on(&mut self, node: usize) {
        let now = self.now;
        if self.nodes[node].listening_since.is_some() || self.nodes[node].driving.is_some() {
            return;
        }
        let station = self.nodes[node].station;
        match &mut self.nodes[node].kind {
            NodeKind::Master(master) => {
                if master.core.is_some() {
                    return;
                }
                // From the seed, the station and the boot count alone, so
                // no other draw shifts it and each boot lands elsewhere.
                let phase = SimRng::new(
                    self.config.seed
                        ^ TIMER_PHASE_STREAM
                        ^ (u64::from(station) << 48)
                        ^ (master.incarnation + 1),
                )
                .next_u64();
                master.boot(now, phase);
            }
            NodeKind::Scripted(scripted) => scripted.decoder.reset(),
        }
        self.nodes[node].listening_since = Some(now);
        self.arm_deadline(node);
        self.note(
            Who::Station(station),
            TraceKind::Note("powered on (INITIALIZE)".into()),
        );
    }

    /// Power a node off: it stops driving at once and a master's core is
    /// dropped.
    pub(super) fn power_off(&mut self, node: usize) {
        let now = self.now;
        let station = self.nodes[node].station;
        self.perturb("a node powered off");
        self.cancel_local_echo(node);
        self.receiver_power_off(node);
        if let Some(tx) = self.nodes[node].driving.take() {
            if let Some(transmission) = self.line.truncate(tx, now) {
                self.record_frame(tx, transmission);
            }
            self.release_driver(tx, crate::DriverReleaseCause::PowerLoss);
        }
        self.nodes[node].listening_since = None;
        if let NodeKind::Master(master) = &mut self.nodes[node].kind {
            master.shut_down();
        }
        self.note(Who::Station(station), TraceKind::Note("powered off".into()));
    }

    /// The corruption armed for this frame from `station`, if any.
    pub(super) fn take_corruption(
        &mut self,
        station: u8,
        octets: &[u8],
    ) -> Option<(usize, Corruption)> {
        let frame_type = super::decode_whole(octets).map(|(header, _)| header.frame_type);
        let position = self.armed.iter().position(|armed| {
            matches!(armed, Armed::Corrupt { station: target, frame_type: wanted, offset, .. }
                if *target == station
                    && *offset < octets.len()
                    && wanted.is_none_or(|wanted| Some(wanted) == frame_type))
        })?;
        match self.armed.remove(position) {
            Armed::Corrupt {
                offset, corruption, ..
            } => Some((offset, corruption)),
            Armed::CutOff { .. } => None,
        }
    }

    /// The power failure armed for this transmission's driver, if any.
    pub(super) fn take_cut_off(&mut self, driver: Driver) -> Option<usize> {
        let Driver::Station(station) = driver else {
            return None;
        };
        let position = self.armed.iter().position(
            |armed| matches!(armed, Armed::CutOff { station: target, .. } if *target == station),
        )?;
        match self.armed.remove(position) {
            Armed::CutOff { after_octets, .. } => Some(after_octets),
            Armed::Corrupt { .. } => None,
        }
    }
}
