//! Where a BBMD sends a broadcast it forwards (Annex J.4.5): pure decisions
//! over the two tables, written into the caller's storage.

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BvlcResultCode, Bvll};
use bacnet_time::Instant;

use core::fmt;

use super::bdt::{distribution_address, Bdt, BdtError, TWO_HOP_MASK};
use super::fdt::Fdt;
use super::nat::Nat;
use crate::{Arrival, LocalSubnet};

/// What kind of destination a [`Target`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Via {
    /// The B/IP broadcast address of this BBMD's own subnet.
    LocalBroadcast,
    /// A peer BBMD's subnet, through its BDT row: the subnet's directed
    /// broadcast address for one-hop distribution, the peer itself for
    /// two-hop.
    Peer,
    /// A registered foreign device.
    ForeignDevice,
}

/// One destination of a forwarded message. The decisions write it; a
/// caller fills its storage with [`Target::UNUSED`] first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Target {
    /// Where to send it.
    pub destination: SocketAddrV4,
    /// What kind of destination that is.
    pub via: Via,
}

impl Target {
    /// A placeholder to fill caller storage with before a call writes it.
    /// It is all zero bits, so a static filled with it can go in `.bss`.
    pub const UNUSED: Self = Self {
        destination: SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
        via: Via::LocalBroadcast,
    };
}

/// What every forwarding decision needs besides the tables and the message.
///
/// It is `#[non_exhaustive]`, so a new input isn't a breaking change:
/// outside this crate, build it with [`Context::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Context {
    /// The caller's current time; registrations lapsed by then are skipped.
    pub now: Instant,
    /// Most foreign devices one message goes to; the rest are skipped and
    /// [`Fanout::fanout_capped`] is set. A local budget, not Annex J's.
    pub fdt_fanout: usize,
}

impl Context {
    /// The decisions at `now`, sending to at most `fdt_fanout` foreign
    /// devices.
    #[must_use]
    pub const fn new(now: Instant, fdt_fanout: usize) -> Self {
        Self { now, fdt_fanout }
    }
}

/// Destinations written to the caller's storage, and how the list was cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Fanout<'o> {
    /// The destinations, in order: the local broadcast if any, then BDT
    /// peers in table order, then foreign devices in table order.
    pub targets: &'o [Target],
    /// Destinations left out because an earlier one had the same address,
    /// as when two BDT rows share a directed broadcast address.
    pub deduplicated: usize,
    /// Whether [`Context::fdt_fanout`] cut the foreign devices short.
    pub fanout_capped: bool,
    /// Destinations lost because the storage was full. Storage of
    /// [`Tables::MAX_TARGETS`] never loses any.
    pub dropped: usize,
}

/// A Forwarded-NPDU to send, and where (Annex J.2.5, J.4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Distribution<'n, 'o> {
    /// The message's originating B/IP address.
    pub origin: SocketAddrV4,
    /// The NPDU it carries.
    pub npdu: &'n [u8],
    /// Its destinations.
    pub fanout: Fanout<'o>,
}

impl<'n, 'o> Distribution<'n, 'o> {
    /// The Forwarded-NPDU every destination gets.
    #[must_use]
    pub const fn message(&self) -> Bvll<'n> {
        Bvll::ForwardedNpdu {
            origin: self.origin,
            npdu: self.npdu,
        }
    }

    /// Each destination with the message to send it.
    pub fn transmissions(
        &self,
    ) -> impl ExactSizeIterator<Item = (SocketAddrV4, Bvll<'n>)> + use<'n, 'o> {
        let message = self.message();
        self.fanout
            .targets
            .iter()
            .map(move |target| (target.destination, message))
    }
}

/// Why a BBMD neither delivers nor forwards a Forwarded-NPDU (Annex J.4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ForwardRefusal {
    /// The sender has no BDT row: only peers' are acted on.
    NotFromPeer,
    /// The sender is this BBMD's own address: the echo of its own
    /// rebroadcast, which its own row would otherwise pass as a peer's.
    OwnEcho,
}

impl ForwardRefusal {
    /// The BVLC-Result to answer the sender with: none, since Annex J has
    /// no answer to a Forwarded-NPDU.
    #[must_use]
    pub const fn result_code(self) -> Option<BvlcResultCode> {
        match self {
            Self::NotFromPeer | Self::OwnEcho => None,
        }
    }
}

impl fmt::Display for ForwardRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFromPeer => "Forwarded-NPDU from a sender that is no BDT peer",
            Self::OwnEcho => "Forwarded-NPDU from this BBMD's own address",
        })
    }
}

impl core::error::Error for ForwardRefusal {}

/// Why a BBMD neither delivers nor distributes a
/// Distribute-Broadcast-To-Network (Annex J.4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DistributeRefusal {
    /// The sender holds no registration live at [`Context::now`].
    NotRegistered,
}

impl DistributeRefusal {
    /// The BVLC-Result to answer the sender with: the
    /// Distribute-Broadcast-To-Network NAK.
    #[must_use]
    pub const fn result_code(self) -> BvlcResultCode {
        match self {
            Self::NotRegistered => BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
        }
    }
}

impl fmt::Display for DistributeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotRegistered => {
                "Distribute-Broadcast-To-Network from a sender with no live registration"
            }
        })
    }
}

impl core::error::Error for DistributeRefusal {}

/// A BBMD's two tables, and its forwarding decisions over them.
///
/// The tables are public fields so a caller can update one while reading
/// the other.
#[derive(Debug, Clone)]
pub struct Tables<const B: usize, const F: usize> {
    /// The broadcast distribution table, which holds this BBMD's own address.
    pub bdt: Bdt<B>,
    /// The foreign device table.
    pub fdt: Fdt<F>,
    /// How the BBMD is reached through a NAT router, if it is.
    nat: Option<Nat>,
}

impl<const B: usize, const F: usize> Tables<B, F> {
    /// The most destinations one decision writes: every BDT row and foreign
    /// device, and the local broadcast. Storage this long never drops one.
    pub const MAX_TARGETS: usize = B + F + 1;

    /// Empty tables for the BBMD at `own` (see [`Bdt::new`]).
    #[must_use]
    pub const fn new(own: SocketAddrV4) -> Self {
        Self {
            bdt: Bdt::new(own),
            fdt: Fdt::new(),
            nat: None,
        }
    }

    /// How the BBMD is reached through a NAT router, if it is.
    #[must_use]
    pub const fn nat(&self) -> Option<&Nat> {
        self.nat.as_ref()
    }

    /// Run behind a NAT router as `nat` says, or, with `None`, not, under
    /// `local`, the BBMD's own address on its subnet (Annex J.7.5).
    ///
    /// Behind NAT the BBMD's own address becomes [`Nat::global`], and the
    /// BDT's NAT rules apply ([`Bdt::set`]): the configured rows are checked
    /// against them now, so a table that has rows but lacks the global
    /// address, or has a one-hop row, refuses the change. Without NAT the
    /// own address is `local`, and an own row is added as usual. Turning NAT
    /// off drops the row of the old global address, which no longer names
    /// this BBMD. On error nothing changes. The global address must pass
    /// [`validate_entry`](super::validate_entry) as a two-hop row.
    pub fn set_nat(&mut self, nat: Option<Nat>, local: SocketAddrV4) -> Result<(), BdtError> {
        let own = Self::own_address_for(nat.as_ref(), local)?;
        let drop = self.leaving_nat(nat.as_ref());
        self.bdt.reconfigure(own, nat.is_some(), drop)?;
        self.nat = nat;
        Ok(())
    }

    /// The old global address when `nat` turns NAT off.
    fn leaving_nat(&self, nat: Option<&Nat>) -> Option<SocketAddrV4> {
        match (&self.nat, nat) {
            (Some(old), None) => Some(old.global),
            _ => None,
        }
    }

    /// [`set_nat`](Self::set_nat) and [`Bdt::set`] in one step: run behind
    /// a NAT router as `nat` says, or not, with the BDT `table`, checked
    /// under the rules that setting brings. A change of global address and
    /// the table that lists it go in together this way. Turning NAT off
    /// drops the old global address's row from `table`, as
    /// [`set_nat`](Self::set_nat) does. On error nothing changes.
    pub fn set_nat_and_bdt(
        &mut self,
        nat: Option<Nat>,
        local: SocketAddrV4,
        table: bacnet_bip_codec::BdtEntries<'_>,
    ) -> Result<(), BdtError> {
        let own = Self::own_address_for(nat.as_ref(), local)?;
        let mut bdt = self.bdt.with_nat(own, nat.is_some());
        bdt.set_without(table, self.leaving_nat(nat.as_ref()))?;
        self.bdt = bdt;
        self.nat = nat;
        Ok(())
    }

    /// The own address behind `nat`, checked, or `local` without it.
    fn own_address_for(nat: Option<&Nat>, local: SocketAddrV4) -> Result<SocketAddrV4, BdtError> {
        match nat {
            Some(nat) => {
                let entry = bacnet_bip_codec::BdtEntry::new(nat.global, TWO_HOP_MASK);
                super::validate_entry(entry)
                    .map_err(|reason| BdtError::Invalid { entry, reason })?;
                Ok(nat.global)
            }
            None => Ok(local),
        }
    }

    /// The originating address of a Forwarded-NPDU this BBMD builds for a
    /// broadcast from `source`: behind a NAT router, its global address
    /// when `private` says other sites can't reach `source` (Annex J.7.5).
    fn origin_for(&self, source: SocketAddrV4, private: impl FnOnce(&Nat) -> bool) -> SocketAddrV4 {
        match &self.nat {
            Some(nat) if private(nat) => nat.global,
            _ => source,
        }
    }

    /// Where a broadcast that starts on this BBMD's subnet goes, other than
    /// that subnet: each BDT row's [`distribution_address`] and each live
    /// foreign device, up to [`Context::fdt_fanout`] of those, skipping this
    /// BBMD's own row and `exclude` (the broadcast's source) in both tables.
    /// An address already listed is left out.
    pub fn broadcast_targets<'o>(
        &self,
        cx: &Context,
        exclude: SocketAddrV4,
        out: &'o mut [Target],
    ) -> Fanout<'o> {
        let mut writer = Writer::new(out);
        self.peer_targets(&mut writer, exclude);
        self.foreign_device_targets_into(&mut writer, cx, exclude);
        writer.finish()
    }

    /// The live foreign devices but `exclude`, up to [`Context::fdt_fanout`]
    /// of them.
    pub fn foreign_device_targets<'o>(
        &self,
        cx: &Context,
        exclude: SocketAddrV4,
        out: &'o mut [Target],
    ) -> Fanout<'o> {
        let mut writer = Writer::new(out);
        self.foreign_device_targets_into(&mut writer, cx, exclude);
        writer.finish()
    }

    /// An Original-Broadcast-NPDU from `source` on this subnet (Annex
    /// J.4.5): a Forwarded-NPDU with `source` as its origin to the
    /// [`broadcast_targets`](Self::broadcast_targets) excluding `source`.
    /// The subnet already has it, so the local broadcast address isn't one.
    /// Behind a NAT router the origin is the global address instead, the
    /// one address of this site that the others can answer (Annex J.7.5).
    pub fn original_broadcast<'n, 'o>(
        &self,
        cx: &Context,
        source: SocketAddrV4,
        npdu: &'n [u8],
        out: &'o mut [Target],
    ) -> Distribution<'n, 'o> {
        Distribution {
            origin: self.origin_for(source, |_| true),
            npdu,
            fanout: self.broadcast_targets(cx, source, out),
        }
    }

    /// A broadcast this BBMD's own device sends on its subnet: what
    /// [`original_broadcast`](Self::original_broadcast) does for one from
    /// this BBMD's own address.
    pub fn own_broadcast<'n, 'o>(
        &self,
        cx: &Context,
        npdu: &'n [u8],
        out: &'o mut [Target],
    ) -> Distribution<'n, 'o> {
        self.original_broadcast(cx, self.bdt.own_address(), npdu, out)
    }

    /// A Forwarded-NPDU that `sender` sent this BBMD, whose originating
    /// address is `origin` (Annex J.4.5).
    ///
    /// Only a BDT peer's is accepted; the caller delivers an accepted one
    /// to its own network layer and sends it on unchanged, origin and all,
    /// to the destinations returned: the live foreign devices but `origin`
    /// and, when [`rebroadcasts_forwarded`](Self::rebroadcasts_forwarded)
    /// says so and the BBMD isn't [`alone`](LocalSubnet::alone) on its
    /// subnet, the subnet's broadcast address first. The peers don't get
    /// it again. Behind a NAT router too the origin stays as it came
    /// (Annex J.7.5).
    ///
    /// For a BBMD that serves a B/IP-M group ([`LocalSubnet::is_group`])
    /// the subnet's broadcast address is the group, and the subnet's mask
    /// isn't used, so every peer's forward goes to the group (Annex J.8.3).
    /// A plain peer BBMD on the group BBMD's own IP subnet is outside the
    /// standard, since B/IP-M and plain devices don't share a subnet (Annex
    /// J.8.1), and is tolerated in one direction only: its forwards still
    /// reach the group, but it takes this BBMD's forwards as already heard
    /// on its subnet (J.4.5) and doesn't rebroadcast them there. The hosted
    /// transport warns about such a peer at start.
    pub fn forwarded_npdu<'o>(
        &self,
        cx: &Context,
        subnet: &LocalSubnet,
        sender: SocketAddrV4,
        arrival: Arrival,
        origin: SocketAddrV4,
        out: &'o mut [Target],
    ) -> Result<Fanout<'o>, ForwardRefusal> {
        if sender == self.bdt.own_address() {
            return Err(ForwardRefusal::OwnEcho);
        }
        if !self.bdt.contains(sender) {
            return Err(ForwardRefusal::NotFromPeer);
        }
        let mut writer = Writer::new(out);
        let mask = subnet.mask.filter(|_| !subnet.is_group());
        if !subnet.alone && self.rebroadcasts_forwarded(mask, sender, arrival) {
            writer.push_local(subnet.broadcast);
        }
        self.foreign_device_targets_into(&mut writer, cx, origin);
        Ok(writer.finish())
    }

    /// Whether a peer's Forwarded-NPDU from `sender` still has to be
    /// broadcast on this BBMD's subnet (Annex J.4.5): only when it came to
    /// this BBMD alone.
    ///
    /// Peers address this subnet through their copy of this BBMD's own BDT
    /// row, which every BDT gives the same mask (J.4.3.2). With
    /// [`TWO_HOP_MASK`] they send to this BBMD by unicast; with the subnet's
    /// mask, to the subnet's directed broadcast address, so every device
    /// there has it already. A datagram that `arrival`ed by broadcast
    /// reached the subnet too, and so did one whose sender is on this BBMD's
    /// subnet (by `subnet_mask`, when known) at this BBMD's own UDP port:
    /// that peer's own broadcasts reach this network's devices. A peer on
    /// the subnet at another port serves another B/IP network there (Annex
    /// J.1.2), so its forward is still broadcast. Behind a NAT router the
    /// own address is the global one, on no local subnet, so `subnet_mask`
    /// isn't used. This doesn't check that `sender` is a peer;
    /// [`forwarded_npdu`](Self::forwarded_npdu) does.
    #[must_use]
    pub fn rebroadcasts_forwarded(
        &self,
        subnet_mask: Option<Ipv4Addr>,
        sender: SocketAddrV4,
        arrival: Arrival,
    ) -> bool {
        let own = self.bdt.own_address();
        let peers_send_here = self.bdt.row(own).is_none_or(|row| row.mask == TWO_HOP_MASK);
        let sender_on_subnet = self.nat.is_none()
            && sender.port() == own.port()
            && subnet_mask.is_some_and(|mask| {
                (sender.ip().to_bits() ^ own.ip().to_bits()) & mask.to_bits() == 0
            });
        peers_send_here && arrival == Arrival::Unicast && !sender_on_subnet
    }

    /// A Distribute-Broadcast-To-Network from `sender` (Annex J.4.5).
    ///
    /// Only a foreign device registered and live at [`Context::now`] may ask;
    /// anyone else is refused with [`DistributeRefusal::NotRegistered`],
    /// whose [`result_code`](DistributeRefusal::result_code) is the NAK to
    /// send. The caller delivers
    /// an accepted one to its own network layer. It goes out as a
    /// Forwarded-NPDU with `sender` as its origin: first to the subnet's
    /// broadcast address, unless the BBMD is [`alone`](LocalSubnet::alone)
    /// there, then to the
    /// [`broadcast_targets`](Self::broadcast_targets) excluding `sender`.
    ///
    /// Behind a NAT router the origin is the global address when `sender`
    /// registered from a private address ([`Nat::is_private`]), which other
    /// sites couldn't answer, and `sender` otherwise. Read literally, Annex
    /// J.7.5 would put the global address on every Forwarded-NPDU the BBMD
    /// builds, this one included; keeping a public device's own address is
    /// a deliberate departure (maintainer decision, 2026-10-20, #940), so
    /// replies to a device that registered from the public side reach it
    /// directly.
    pub fn distribute_broadcast<'n, 'o>(
        &self,
        cx: &Context,
        subnet: &LocalSubnet,
        sender: SocketAddrV4,
        npdu: &'n [u8],
        out: &'o mut [Target],
    ) -> Result<Distribution<'n, 'o>, DistributeRefusal> {
        if !self.fdt.is_registered_at(sender, cx.now) {
            return Err(DistributeRefusal::NotRegistered);
        }
        let mut writer = Writer::new(out);
        if !subnet.alone {
            writer.push_local(subnet.broadcast);
        }
        self.peer_targets(&mut writer, sender);
        self.foreign_device_targets_into(&mut writer, cx, sender);
        Ok(Distribution {
            origin: self.origin_for(sender, |nat| nat.is_private(*sender.ip())),
            npdu,
            fanout: writer.finish(),
        })
    }

    /// The BDT rows but this BBMD's own and `exclude`.
    fn peer_targets(&self, writer: &mut Writer<'_>, exclude: SocketAddrV4) {
        let own = self.bdt.own_address();
        for row in self.bdt.rows() {
            if row.address == own || row.address == exclude {
                continue;
            }
            writer.push_unique(distribution_address(*row), Via::Peer);
        }
        writer.peers_end = writer.len;
    }

    /// The live foreign devices but `exclude`, after any peers.
    fn foreign_device_targets_into(
        &self,
        writer: &mut Writer<'_>,
        cx: &Context,
        exclude: SocketAddrV4,
    ) {
        let mut sent = 0;
        for entry in self.fdt.live_at(cx.now) {
            if entry.address == exclude {
                continue;
            }
            // The table holds one registration per address, so only a peer
            // target can repeat one.
            if writer.is_peer_target(entry.address) {
                writer.deduplicated += 1;
                continue;
            }
            if sent >= cx.fdt_fanout {
                writer.fanout_capped = true;
                break;
            }
            writer.push(entry.address, Via::ForeignDevice);
            sent += 1;
        }
    }
}

/// Fills the caller's storage for one decision.
struct Writer<'o> {
    out: &'o mut [Target],
    len: usize,
    /// Where the peer targets start: after the local broadcast, which is
    /// never compared with them.
    peers_start: usize,
    /// Where they end.
    peers_end: usize,
    deduplicated: usize,
    fanout_capped: bool,
    dropped: usize,
}

impl<'o> Writer<'o> {
    fn new(out: &'o mut [Target]) -> Self {
        Self {
            out,
            len: 0,
            peers_start: 0,
            peers_end: 0,
            deduplicated: 0,
            fanout_capped: false,
            dropped: 0,
        }
    }

    fn push(&mut self, destination: SocketAddrV4, via: Via) {
        match self.out.get_mut(self.len) {
            Some(slot) => {
                *slot = Target { destination, via };
                self.len += 1;
            }
            None => self.dropped += 1,
        }
    }

    /// The local broadcast, first.
    fn push_local(&mut self, destination: SocketAddrV4) {
        self.push(destination, Via::LocalBroadcast);
        self.peers_start = self.len;
        self.peers_end = self.len;
    }

    /// A peer target, unless an earlier one has its address.
    fn push_unique(&mut self, destination: SocketAddrV4, via: Via) {
        if self.out[self.peers_start..self.len]
            .iter()
            .any(|target| target.destination == destination)
        {
            self.deduplicated += 1;
        } else {
            self.push(destination, via);
        }
    }

    fn is_peer_target(&self, destination: SocketAddrV4) -> bool {
        self.out[self.peers_start..self.peers_end]
            .iter()
            .any(|target| target.destination == destination)
    }

    fn finish(self) -> Fanout<'o> {
        let Self {
            out,
            len,
            deduplicated,
            fanout_capped,
            dropped,
            ..
        } = self;
        Fanout {
            targets: &out[..len],
            deduplicated,
            fanout_capped,
            dropped,
        }
    }
}
