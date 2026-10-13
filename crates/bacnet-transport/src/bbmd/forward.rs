//! The hosted BBMD's forwarding: the core's decisions (Annex J.4.5) under
//! the host's clock, fan-out budget and counters.
//!
//! Each method purges lapsed registrations where the BBMD always has (a
//! decision never sends to one either way), and copies its destinations out
//! of the tables, so the caller can release the lock before it sends
//! anything.

use std::net::SocketAddrV4;
use std::time::Instant;

use bacnet_bip_core::bbmd::{
    Context, DistributeRefusal, Distribution, Fanout, ForwardRefusal, Target, Via,
};
use bacnet_bip_core::{Arrival, LocalSubnet};

#[cfg(test)]
use super::address;
use super::{BbmdState, Tables};

/// Foreign devices one forwarded message goes to without a policy.
const DEFAULT_FDT_FANOUT: usize = 32;

/// What one forwarding decision sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Forward {
    /// The Forwarded-NPDU's originating B/IP address.
    pub(crate) origin: SocketAddrV4,
    /// The local subnet's broadcast address, when the message goes there.
    pub(crate) local_broadcast: Option<SocketAddrV4>,
    /// The peer subnets and foreign devices, in that order.
    pub(crate) remote: Vec<SocketAddrV4>,
    /// Destinations left out as repeats. The FDT counters have them already;
    /// the fan-out counters take them from here.
    pub(crate) deduplicated: u64,
}

impl BbmdState {
    /// The core's context for a decision at `now`.
    fn decision_context(&self, now: Instant) -> Context {
        let fdt_fanout = self
            .foreign_device_policy
            .as_ref()
            .map_or(DEFAULT_FDT_FANOUT, |policy| policy.max_fdt_fanout);
        Context::new(self.clock.to_core(now), fdt_fanout)
    }

    /// Count what the core reports about one list of destinations.
    fn count(&mut self, fanout: &Fanout<'_>) {
        debug_assert_eq!(fanout.dropped, 0, "storage of MAX_TARGETS drops nothing");
        self.counters.destinations_deduplicated += fanout.deduplicated as u64;
        if fanout.fanout_capped {
            self.counters.fanout_budget_reached += 1;
        }
    }

    /// This BBMD's subnet, with `broadcast` as its broadcast address.
    fn local_subnet(&self, broadcast: SocketAddrV4) -> LocalSubnet {
        let mut subnet = LocalSubnet::new(broadcast);
        subnet.mask = self.subnet_mask;
        subnet
    }

    /// Count `sent` and copy it out.
    fn forward(&mut self, sent: &Distribution<'_, '_>) -> Forward {
        self.forward_fanout(sent.origin, &sent.fanout)
    }

    /// Count `fanout`, a message from `origin` that goes on unchanged, and
    /// copy it out.
    fn forward_fanout(&mut self, origin: SocketAddrV4, fanout: &Fanout<'_>) -> Forward {
        self.count(fanout);
        let targets = fanout.targets;
        Forward {
            origin,
            local_broadcast: targets
                .iter()
                .find(|target| target.via == Via::LocalBroadcast)
                .map(|target| target.destination),
            remote: targets
                .iter()
                .filter(|target| target.via != Via::LocalBroadcast)
                .map(|target| target.destination)
                .collect(),
            deduplicated: fanout.deduplicated as u64,
        }
    }

    /// An Original-Broadcast-NPDU from `source`: see
    /// [`Tables::original_broadcast`](bacnet_bip_core::bbmd::Tables::original_broadcast).
    pub(crate) fn forward_original_broadcast(
        &mut self,
        source: SocketAddrV4,
        npdu: &[u8],
        now: Instant,
    ) -> Forward {
        self.purge_expired_at(now);
        let cx = self.decision_context(now);
        let mut out = [Target::UNUSED; Tables::MAX_TARGETS];
        let sent = self.tables.original_broadcast(&cx, source, npdu, &mut out);
        self.forward(&sent)
    }

    /// A Forwarded-NPDU `sender` sent: see
    /// [`Tables::forwarded_npdu`](bacnet_bip_core::bbmd::Tables::forwarded_npdu).
    /// Lapsed registrations are purged only when it is accepted.
    pub(crate) fn forward_forwarded_npdu(
        &mut self,
        sender: SocketAddrV4,
        arrival: Arrival,
        origin: SocketAddrV4,
        local_broadcast: SocketAddrV4,
        now: Instant,
    ) -> Result<Forward, ForwardRefusal> {
        let cx = self.decision_context(now);
        let subnet = self.local_subnet(local_broadcast);
        let mut out = [Target::UNUSED; Tables::MAX_TARGETS];
        let fanout = self
            .tables
            .forwarded_npdu(&cx, &subnet, sender, arrival, origin, &mut out)?;
        self.purge_expired_at(now);
        Ok(self.forward_fanout(origin, &fanout))
    }

    /// A Distribute-Broadcast-To-Network from `sender`, or why it is
    /// refused, whose result code is the NAK to answer with: see
    /// [`Tables::distribute_broadcast`](bacnet_bip_core::bbmd::Tables::distribute_broadcast).
    pub(crate) fn forward_distribute_broadcast(
        &mut self,
        sender: SocketAddrV4,
        npdu: &[u8],
        local_broadcast: SocketAddrV4,
        now: Instant,
    ) -> Result<Forward, DistributeRefusal> {
        self.purge_expired_at(now);
        let cx = self.decision_context(now);
        let subnet = self.local_subnet(local_broadcast);
        let mut out = [Target::UNUSED; Tables::MAX_TARGETS];
        let sent = self
            .tables
            .distribute_broadcast(&cx, &subnet, sender, npdu, &mut out)?;
        Ok(self.forward(&sent))
    }

    /// A broadcast this BBMD's own device sends: see
    /// [`Tables::own_broadcast`](bacnet_bip_core::bbmd::Tables::own_broadcast).
    pub(crate) fn forward_own_broadcast(&mut self, npdu: &[u8], now: Instant) -> Forward {
        self.purge_expired_at(now);
        let cx = self.decision_context(now);
        let mut out = [Target::UNUSED; Tables::MAX_TARGETS];
        let sent = self.tables.own_broadcast(&cx, npdu, &mut out);
        self.forward(&sent)
    }

    // -----------------------------------------------------------------------
    // Forwarding targets, for tests: the decisions themselves are
    // `bacnet_bip_core::bbmd`'s (#1669).
    // -----------------------------------------------------------------------

    /// All (ip, port) targets of a broadcast from `(exclude_ip,
    /// exclude_port)` now, as in [`Self::forwarding_targets_at`].
    #[cfg(test)]
    pub(crate) fn forwarding_targets(
        &mut self,
        exclude_ip: [u8; 4],
        exclude_port: u16,
    ) -> Vec<([u8; 4], u16)> {
        self.forwarding_targets_at(exclude_ip, exclude_port, Instant::now())
    }

    /// All (ip, port) targets of a broadcast from `(exclude_ip,
    /// exclude_port)` at `now`, leaving out that source and this BBMD: BDT
    /// rows at their directed broadcast address, then foreign devices, at
    /// most the policy's `max_fdt_fanout` of them. Lapsed registrations are
    /// purged first, and the fan-out counters count the call.
    #[cfg(test)]
    pub(crate) fn forwarding_targets_at(
        &mut self,
        exclude_ip: [u8; 4],
        exclude_port: u16,
        now: Instant,
    ) -> Vec<([u8; 4], u16)> {
        self.purge_expired_at(now);
        let cx = self.decision_context(now);
        let mut out = [Target::UNUSED; Tables::MAX_TARGETS];
        let fanout =
            self.tables
                .broadcast_targets(&cx, address(exclude_ip, exclude_port), &mut out);
        self.count(&fanout);
        pairs(fanout.targets)
    }

    /// The foreign devices among [`Self::forwarding_targets`], now.
    #[cfg(test)]
    pub(crate) fn fdt_forwarding_targets(
        &mut self,
        exclude_ip: [u8; 4],
        exclude_port: u16,
    ) -> Vec<([u8; 4], u16)> {
        self.fdt_forwarding_targets_at(exclude_ip, exclude_port, Instant::now())
    }

    /// The foreign devices among [`Self::forwarding_targets_at`] at `now`.
    #[cfg(test)]
    pub(crate) fn fdt_forwarding_targets_at(
        &mut self,
        exclude_ip: [u8; 4],
        exclude_port: u16,
        now: Instant,
    ) -> Vec<([u8; 4], u16)> {
        self.purge_expired_at(now);
        let cx = self.decision_context(now);
        let mut out = [Target::UNUSED; Tables::MAX_TARGETS];
        let fanout =
            self.tables
                .foreign_device_targets(&cx, address(exclude_ip, exclude_port), &mut out);
        self.count(&fanout);
        pairs(fanout.targets)
    }
}

/// `targets` as (ip, port) pairs.
#[cfg(test)]
fn pairs(targets: &[Target]) -> Vec<([u8; 4], u16)> {
    targets
        .iter()
        .map(|target| (target.destination.ip().octets(), target.destination.port()))
        .collect()
}
