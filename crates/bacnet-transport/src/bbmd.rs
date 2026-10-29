//! BBMD (BACnet/IP Broadcast Management Device) state and tables.
//!
//! The Broadcast Distribution Table (BDT), the Foreign Device Table (FDT)
//! and the forwarding rules of ASHRAE 135-2020 Annex J are
//! `bacnet-bip-core`'s ([`bacnet_bip_core::bbmd`]). [`BbmdState`] runs them
//! on the host: it maps `std::time::Instant` onto the core's clock, keeps
//! the host's own policy around them (the foreign-device access list and
//! rate limit, the Delete-FDT-Entry access list, the counters), and converts
//! between the core's codec rows and this module's [`BdtEntry`] and
//! [`FdtEntry`]. Pure state and logic: no async or I/O.

use std::net::{Ipv4Addr, SocketAddrV4};
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use bacnet_bip_codec::{self as codec, BdtEntries, BvlcResultCode, FdtEntries};
use bacnet_bip_core::Arrival;
use bacnet_bip_core::bbmd::{self as core_bbmd, BdtError, Request};
use bacnet_time::HostClock;
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bytes::BytesMut;

use crate::bvll::bvll_error;

mod activation;
mod forward;
mod nat;
mod persist;
mod policy;
mod serve;
mod tables;
pub(crate) use forward::Forward;
use nat::NatSetting;
pub use nat::{NatError, NatTraversal, PrivateRange};
pub(crate) use persist::{BdtFile, PersistedNat, decode_file, write_atomically};
use policy::ForeignDeviceRateTracker;
pub use policy::{FdtCounters, ForeignDevicePolicy};
pub(crate) use serve::Reply;
pub(crate) use tables::{GroupChange, LiveTables, lock};

/// Rows the BDT holds, this BBMD's own row included.
const MAX_BDT: usize = 128;
/// Registrations the FDT holds.
const MAX_FDT: usize = 128;

/// Both tables, sized for the host.
type Tables = core_bbmd::Tables<MAX_BDT, MAX_FDT>;

/// A Broadcast Distribution Table entry — one peer BBMD.
///
/// Its wire form is [`crate::bvll::BdtEntry`]; `From` converts both ways.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BdtEntry {
    /// IPv4 address of the peer BBMD.
    pub ip: [u8; 4],
    /// UDP port of the peer BBMD.
    pub port: u16,
    /// Mask applied to the peer's address to derive the directed-broadcast address used when
    /// forwarding to it (all ones sends unicast).
    pub broadcast_mask: [u8; 4],
}

/// A Foreign Device Table entry — one registered foreign device.
///
/// A copy of a registration, which [`BbmdState::fdt`] makes; changing it
/// changes nothing in the table. [`FdtEntry::wire_entry_at`] gives its wire
/// form, [`crate::bvll::FdtEntry`].
#[derive(Debug, Clone)]
pub struct FdtEntry {
    /// IPv4 address of the foreign device.
    pub ip: [u8; 4],
    /// UDP port of the foreign device.
    pub port: u16,
    /// Time-to-live in seconds requested at registration; expiry adds a 30-second grace period.
    pub ttl: u16,
    /// When the registration was last created or renewed.
    pub registered_at: Instant,
}

impl FdtEntry {
    /// This entry as the core's registration, on a clock whose midpoint is
    /// the registration time, and that clock.
    fn registration(&self) -> (core_bbmd::Registration, HostClock) {
        let clock = HostClock::centred(self.registered_at);
        let registration = core_bbmd::Registration::new(
            address(self.ip, self.port),
            self.ttl,
            clock.instant(self.registered_at),
        );
        (registration, clock)
    }

    /// Whether this entry has expired (TTL + grace period) relative to `now`.
    pub fn is_expired_at(&self, now: Instant) -> bool {
        let (registration, clock) = self.registration();
        registration.is_expired_at(clock.instant(now))
    }

    /// Whether this entry has expired (TTL + grace period).
    pub fn is_expired(&self) -> bool {
        self.is_expired_at(Instant::now())
    }

    /// Seconds remaining including the 30-second grace period relative to `now`.
    pub fn seconds_remaining_at(&self, now: Instant) -> u16 {
        let (registration, clock) = self.registration();
        registration.seconds_remaining_at(clock.instant(now))
    }

    /// Seconds remaining including the 30-second grace period.
    pub fn seconds_remaining(&self) -> u16 {
        self.seconds_remaining_at(Instant::now())
    }

    /// This entry as a Read-FDT-Ack reports it at `now` (Annex J.2.8.1).
    pub fn wire_entry_at(&self, now: Instant) -> codec::FdtEntry {
        let (registration, clock) = self.registration();
        registration.wire_entry_at(clock.instant(now))
    }
}

impl From<&BdtEntry> for codec::BdtEntry {
    fn from(entry: &BdtEntry) -> Self {
        Self::new(address(entry.ip, entry.port), entry.broadcast_mask.into())
    }
}

impl From<codec::BdtEntry> for BdtEntry {
    fn from(entry: codec::BdtEntry) -> Self {
        Self {
            ip: entry.address.ip().octets(),
            port: entry.address.port(),
            broadcast_mask: entry.mask.octets(),
        }
    }
}

/// `ip:port` as a socket address.
fn address(ip: [u8; 4], port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::from(ip), port)
}

/// The host's error for a BDT the core refused, worded as before the core
/// existed.
fn bdt_error(error: BdtError) -> Error {
    Error::Encoding(match error {
        BdtError::Invalid { entry, reason } => {
            // A mask under /8 can direct a broadcast to 255.255.255.255,
            // which this error has always named, and the tests check.
            let reason = match reason {
                core_bbmd::EntryError::MaskTooWide => {
                    "mask prefix too wide (< /8) or limited-broadcast target"
                }
                other => other.reason(),
            };
            format!("BDT {reason}: {:?}", BdtEntry::from(entry))
        }
        BdtError::ConflictingMasks { entry } => {
            format!("BDT conflicting masks: {:?}", BdtEntry::from(entry))
        }
        BdtError::Capacity { needed, available } => {
            format!("BDT size {needed} exceeds maximum of {available} after self-entry insertion")
        }
        other => format!("BDT refused: {other}"),
    })
}

/// Validate and canonicalize a BDT candidate (issue #529) without adding the
/// local BBMD's own row.
///
/// Byte-identical duplicates collapse in stable first-seen order; the same
/// `(ip, port)` with different masks rejects the whole candidate.
pub(crate) fn canonical_bdt(entries: Vec<BdtEntry>) -> Result<Vec<BdtEntry>, Error> {
    let rows: Vec<codec::BdtEntry> = entries.iter().map(codec::BdtEntry::from).collect();
    let mut canonical = rows.clone();
    let len = core_bbmd::canonicalize(BdtEntries::from_entries(&rows), &mut canonical)
        .map_err(bdt_error)?;
    Ok(canonical[..len]
        .iter()
        .copied()
        .map(BdtEntry::from)
        .collect())
}

/// An FDT entry decoded from the wire (Read-FDT-ACK payload).
///
/// Unlike [`FdtEntry`], this does not contain an `Instant` field — it carries
/// only the wire-format TTL and seconds-remaining values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdtEntryWire {
    /// IPv4 address of the foreign device.
    pub ip: [u8; 4],
    /// UDP port of the foreign device.
    pub port: u16,
    /// Registration time-to-live in seconds.
    pub ttl: u16,
    /// Seconds until the entry expires, as reported by the BBMD (includes the grace period).
    pub seconds_remaining: u16,
}

impl From<codec::FdtEntry> for FdtEntryWire {
    fn from(entry: codec::FdtEntry) -> Self {
        Self {
            ip: entry.address.ip().octets(),
            port: entry.address.port(),
            ttl: entry.ttl,
            seconds_remaining: entry.remaining,
        }
    }
}

/// Append the wire octets of `entries`, a BDT payload (Annex J.2.2.1), to
/// `buf`. A persisted BDT is stored in this form.
pub fn encode_bdt_entries(entries: &[BdtEntry], buf: &mut BytesMut) {
    buf.reserve(entries.len() * codec::BDT_ENTRY_LENGTH);
    for entry in entries {
        buf.extend_from_slice(&codec::BdtEntry::from(entry).to_octets());
    }
}

/// Decode FDT entries from a Read-FDT-ACK payload (Annex J.2.8.1).
pub fn decode_fdt(data: &[u8]) -> Result<Vec<FdtEntryWire>, Error> {
    fdt_from_wire(FdtEntries::from_wire(data).map_err(bvll_error)?)
}

/// The rows of a BDT a peer sent, refusing more than
/// [`BbmdState::MAX_BDT_ENTRIES`].
pub fn bdt_from_wire(table: BdtEntries<'_>) -> Result<Vec<BdtEntry>, Error> {
    let count = table.len();
    if count > BbmdState::MAX_BDT_ENTRIES {
        return Err(table_too_long(
            "BDT entry count {} exceeds maximum of {}",
            count,
            BbmdState::MAX_BDT_ENTRIES,
        ));
    }
    Ok(table.iter().map(BdtEntry::from).collect())
}

/// The rows of an FDT a BBMD returned, refusing more than
/// [`BbmdState::MAX_FDT_ENTRIES`].
pub fn fdt_from_wire(table: FdtEntries<'_>) -> Result<Vec<FdtEntryWire>, Error> {
    let count = table.len();
    if count > BbmdState::MAX_FDT_ENTRIES {
        return Err(table_too_long(
            "FDT entry count {} exceeds maximum of {}",
            count,
            BbmdState::MAX_FDT_ENTRIES,
        ));
    }
    Ok(table.iter().map(FdtEntryWire::from).collect())
}

/// The refusal of a table of `count` entries past its `max`.
fn table_too_long(label: &'static str, count: usize, max: usize) -> Error {
    DecodeError::new(DecodingKind::InvalidEncoding, 0, label)
        .with_values(count as u64, max as u64)
        .into()
}

/// BBMD state: the BDT and FDT, run by `bacnet-bip-core`, and the host's
/// policy around them.
#[derive(Debug)]
pub struct BbmdState {
    /// Maps the host clock onto the core's.
    clock: HostClock,
    /// Both tables. The BDT holds this BBMD's own B/IP address: the global
    /// one behind a NAT router, the local one otherwise.
    tables: Tables,
    /// The B/IP address of this BBMD's socket on its own subnet.
    local: SocketAddrV4,
    /// Whether it runs behind a NAT router, and how it is reached there.
    nat: NatSetting,
    /// The mask of the subnet this BBMD's own address is on, when the host
    /// reports it: a peer's Forwarded-NPDU sent from that subnet at this
    /// BBMD's own port isn't broadcast on it again (Annex J.4.5).
    subnet_mask: Option<Ipv4Addr>,
    /// Whether this BBMD is the only device of its B/IP network on its
    /// subnet: it then skips the local rebroadcast of forwarded and
    /// distributed broadcasts (Annex J.4.5, J.7.5).
    alone_on_subnet: bool,
    /// Allowed source IPs for Delete-Foreign-Device-Table-Entry.
    /// Empty means deny all (fail closed).
    management_acl: Vec<[u8; 4]>,
    /// Explicit policy for foreign device registrations.
    /// None means fail closed (deny all registrations).
    foreign_device_policy: Option<ForeignDevicePolicy>,
    /// The policy kept while BBMD_Accept_FD_Registrations is FALSE, for
    /// when it is TRUE again.
    parked_policy: Option<ForeignDevicePolicy>,
    rate_tracker: ForeignDeviceRateTracker,
    counters: FdtCounters,
}

impl BbmdState {
    /// Create a new BBMD with the given local address.
    pub fn new(local_ip: [u8; 4], local_port: u16) -> Self {
        Self {
            clock: HostClock::centred(Instant::now()),
            tables: Tables::new(address(local_ip, local_port)),
            local: address(local_ip, local_port),
            nat: NatSetting::default(),
            subnet_mask: None,
            alone_on_subnet: false,
            management_acl: Vec::new(),
            foreign_device_policy: None,
            parked_policy: None,
            rate_tracker: ForeignDeviceRateTracker::new(),
            counters: FdtCounters::default(),
        }
    }

    /// Enable foreign device registration with the specified policy.
    pub fn enable_foreign_device_registration(&mut self, policy: ForeignDevicePolicy) {
        self.foreign_device_policy = Some(policy);
    }

    /// Set or clear the foreign device registration policy.
    pub fn set_foreign_device_policy(&mut self, policy: Option<ForeignDevicePolicy>) {
        self.foreign_device_policy = policy;
    }

    /// Current foreign device registration policy, if enabled.
    pub fn foreign_device_policy(&self) -> Option<&ForeignDevicePolicy> {
        self.foreign_device_policy.as_ref()
    }

    /// Operational counters for Foreign Device Table management.
    pub fn fdt_counters(&self) -> FdtCounters {
        self.counters
    }

    /// This BBMD's B/IP address (IP and UDP port) on its own subnet: the
    /// one given to [`Self::new`], or the one a restart of a `0.0.0.0`-bound
    /// transport chose. Behind a NAT router the BDT knows it by its global
    /// address instead ([`Self::own_address`]).
    pub fn local_address(&self) -> SocketAddrV4 {
        self.local
    }

    /// [`Self::local_address`] as the octets and port the tests compare.
    #[cfg(test)]
    pub(crate) fn local_pair(&self) -> ([u8; 4], u16) {
        (self.local.ip().octets(), self.local.port())
    }

    /// Change this BBMD's own B/IP address, keeping the BDT and FDT. Only the
    /// B/IP transport's restart of a wildcard-bound BBMD calls this.
    ///
    /// A self row that [`Self::set_bdt`] appended for the old address moves to
    /// the new one. A row the BDT lists explicitly stays, and becomes an
    /// ordinary peer row once it no longer matches. Returns `Error::Encoding`,
    /// changing nothing, when the new self row would overflow the BDT.
    /// Behind a NAT router the BDT's own row is the global address, which
    /// stays.
    pub(crate) fn set_local_address(&mut self, ip: [u8; 4], port: u16) -> Result<(), Error> {
        let local = address(ip, port);
        if !self.nat.enabled {
            self.tables.bdt.set_own_address(local).map_err(bdt_error)?;
        }
        self.local = local;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // BDT management
    // -----------------------------------------------------------------------

    /// Maximum number of entries in the Broadcast Distribution Table.
    pub const MAX_BDT_ENTRIES: usize = MAX_BDT;

    /// Replace the entire BDT.
    ///
    /// Central commit choke point for issue #529: validates and canonicalizes
    /// the complete candidate before changing the table. Byte-identical
    /// duplicates collapse in stable first-seen order; the same `(ip, port)`
    /// with different masks rejects the whole candidate. The local BBMD is
    /// auto-inserted as `/32` when absent. Any invalid, conflicting, or
    /// over-capacity candidate returns `Error::Encoding` and preserves the
    /// previously committed table exactly.
    pub fn set_bdt(&mut self, entries: Vec<BdtEntry>) -> Result<(), Error> {
        let rows: Vec<codec::BdtEntry> = entries.iter().map(codec::BdtEntry::from).collect();
        self.tables
            .bdt
            .set(BdtEntries::from_entries(&rows))
            .map_err(bdt_error)
    }

    /// Say whether this BBMD serves a B/IP-M group (Annex J.8.3, #941):
    /// its own BDT row must then have the all-ones mask, and a BDT whose own
    /// row has another is refused from now on. Turning it on fails, changing
    /// nothing, while the own row has another mask.
    pub(crate) fn set_serves_group(&mut self, serves: bool) -> Result<(), Error> {
        self.tables.bdt.set_serves_group(serves).map_err(bdt_error)
    }

    /// Whether this BBMD serves a B/IP-M group (Annex J.8.3): see
    /// [`BipTransport::set_multicast`](crate::bip::BipTransport::set_multicast).
    pub fn serves_group(&self) -> bool {
        self.tables.bdt.serves_group()
    }

    /// A copy of the current BDT, this BBMD's own row included.
    pub fn bdt(&self) -> Vec<BdtEntry> {
        rows_of(self.tables.bdt.rows())
    }

    /// The BDT without the self row [`Self::set_bdt`] appended, if it did.
    pub(crate) fn configured_bdt(&self) -> Vec<BdtEntry> {
        rows_of(self.tables.bdt.configured_rows())
    }

    /// The BDT as wire entries, for a Read-BDT-ACK (Annex J.2.4.1).
    pub fn bdt_wire_entries(&self) -> Vec<codec::BdtEntry> {
        self.tables.bdt.rows().to_vec()
    }

    /// Decode a BDT from wire bytes, as a persisted BDT holds it (a Write-BDT
    /// or Read-BDT-ACK payload), refusing more than
    /// [`Self::MAX_BDT_ENTRIES`] rows.
    pub fn decode_bdt(data: &[u8]) -> Result<Vec<BdtEntry>, Error> {
        bdt_from_wire(BdtEntries::from_wire(data).map_err(bvll_error)?)
    }

    // -----------------------------------------------------------------------
    // FDT management
    // -----------------------------------------------------------------------

    /// Maximum number of entries in the Foreign Device Table.
    pub const MAX_FDT_ENTRIES: usize = MAX_FDT;

    /// Register or re-register a foreign device at the current wall-clock instant.
    pub fn register_foreign_device(&mut self, ip: [u8; 4], port: u16, ttl: u16) -> BvlcResultCode {
        self.register_foreign_device_at(ip, port, ttl, Instant::now())
    }

    /// Register or re-register a foreign device at the given `now` instant,
    /// as a Register-Foreign-Device from `ip:port` is answered.
    ///
    /// Without a policy every registration is refused. With one, the core
    /// purges lapsed entries and checks the time-to-live; then this
    /// policy's source list and rate limit apply, then the core's
    /// per-source quota and reserved capacity.
    pub fn register_foreign_device_at(
        &mut self,
        ip: [u8; 4],
        port: u16,
        ttl: u16,
        now: Instant,
    ) -> BvlcResultCode {
        let sender = address(ip, port);
        match self.serve(Request::Register { sender, ttl }, now) {
            Reply::Result(code) => code,
            other => unreachable!("a registration is answered with a result, not {other:?}"),
        }
    }

    /// Delete a foreign device entry, lapsed or not, on the host's own
    /// authority: no access list applies.
    pub fn delete_foreign_device(&mut self, ip: [u8; 4], port: u16) -> BvlcResultCode {
        if self.tables.fdt.delete(address(ip, port)) {
            BvlcResultCode::SUCCESSFUL_COMPLETION
        } else {
            BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
        }
    }

    /// Purge expired FDT entries at the given instant and return the number removed.
    pub fn purge_expired_at(&mut self, now: Instant) -> usize {
        let removed = self.tables.fdt.purge(self.clock.instant(now));
        self.counters.registrations_expired += removed as u64;
        removed
    }

    /// Purge expired FDT entries and return the number removed.
    pub fn purge_expired(&mut self) -> usize {
        self.purge_expired_at(Instant::now())
    }

    /// A copy of the current FDT (purges expired entries first).
    pub fn fdt(&mut self) -> Vec<FdtEntry> {
        self.purge_expired();
        self.tables
            .fdt
            .registrations()
            .iter()
            .map(|registration| FdtEntry {
                ip: registration.address.ip().octets(),
                port: registration.address.port(),
                ttl: registration.ttl,
                registered_at: self
                    .clock
                    .to_std(registration.registered_at)
                    .unwrap_or(self.clock.origin()),
            })
            .collect()
    }

    /// The FDT as wire entries for a Read-FDT-ACK (Annex J.2.8.1), after
    /// purging expired entries.
    pub fn fdt_wire_entries(&mut self) -> Vec<codec::FdtEntry> {
        self.fdt_wire_entries_at(Instant::now())
    }

    /// [`Self::fdt_wire_entries`] at `now`.
    pub(crate) fn fdt_wire_entries_at(&mut self, now: Instant) -> Vec<codec::FdtEntry> {
        self.purge_expired_at(now);
        let now = self.clock.instant(now);
        self.tables
            .fdt
            .registrations()
            .iter()
            .map(|registration| registration.wire_entry_at(now))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Source validation helpers
    // -----------------------------------------------------------------------

    /// Check if a sender is a BDT peer.
    pub fn is_bdt_peer(&self, ip: [u8; 4], port: u16) -> bool {
        self.tables.bdt.contains(address(ip, port))
    }

    /// Whether a Forwarded-NPDU that this BDT peer sent by unicast is also
    /// broadcast on this BBMD's subnet (Annex J.4.5): this BBMD's own BDT
    /// row has the two-hop mask, so peers send to it alone, and the peer
    /// isn't on this BBMD's subnet at its own port (once the transport has
    /// the subnet's mask). The sender's own row doesn't matter. One
    /// that arrives by broadcast never is; see
    /// [`Tables::rebroadcasts_forwarded`](core_bbmd::Tables::rebroadcasts_forwarded).
    /// A BBMD that serves a B/IP-M group ([`Self::serves_group`]) passes it
    /// to the group whatever the peer's subnet (Annex J.8.3).
    pub fn forwarded_npdu_needs_local_broadcast(&self, ip: [u8; 4], port: u16) -> bool {
        let sender = address(ip, port);
        let mask = self.subnet_mask.filter(|_| !self.serves_group());
        sender != self.tables.bdt.own_address()
            && self.tables.bdt.contains(sender)
            && self
                .tables
                .rebroadcasts_forwarded(mask, sender, Arrival::Unicast)
    }

    /// The mask of the subnet this BBMD's own address is on, as the host
    /// reported it at start; `None` when it didn't.
    pub(crate) fn subnet_mask(&self) -> Option<Ipv4Addr> {
        self.subnet_mask
    }

    /// Set the mask of the subnet this BBMD's own address is on.
    pub(crate) fn set_subnet_mask(&mut self, mask: Option<Ipv4Addr>) {
        self.subnet_mask = mask;
    }

    /// Whether this BBMD is set up as the only device of its B/IP network on
    /// its subnet ([`Self::set_alone_on_subnet`]).
    pub fn alone_on_subnet(&self) -> bool {
        self.alone_on_subnet
    }

    /// Say whether this BBMD is the only device of its B/IP network on its
    /// subnet, as a BBMD behind a NAT router often is (Annex J.7.5). When it
    /// is, a peer's Forwarded-NPDU and a foreign device's
    /// Distribute-Broadcast-To-Network go only to the peers and foreign
    /// devices, not to the subnet's broadcast address, where nothing would
    /// receive them. Annex J.4.5 permits this for the Forwarded-NPDU; the
    /// Distribute-Broadcast-To-Network case extends it. Off by default: with another device of
    /// the network on the subnet, that device stops receiving remote
    /// broadcasts.
    pub fn set_alone_on_subnet(&mut self, alone: bool) {
        self.alone_on_subnet = alone;
    }

    /// Check if a sender is a registered (non-expired) foreign device, for
    /// tests (#1669).
    #[cfg(test)]
    pub(crate) fn is_registered_foreign_device(&mut self, ip: [u8; 4], port: u16) -> bool {
        let now = Instant::now();
        self.purge_expired_at(now);
        let now = self.clock.instant(now);
        self.tables.fdt.is_registered_at(address(ip, port), now)
    }

    // -----------------------------------------------------------------------
    // Management ACL
    // -----------------------------------------------------------------------

    /// Check whether a source IP is allowed to perform Delete-FDT-Entry.
    /// An empty ACL denies all sources (fail closed).
    pub fn is_management_allowed(&self, source_ip: &[u8; 4]) -> bool {
        self.management_acl.contains(source_ip)
    }

    /// Set the Delete-FDT-Entry management ACL. An empty list denies all sources.
    pub fn set_management_acl(&mut self, acl: Vec<[u8; 4]>) {
        self.management_acl = acl;
    }

    #[cfg(test)]
    pub(crate) fn backdate_foreign_device_for_test(
        &mut self,
        ip: [u8; 4],
        port: u16,
        elapsed: Duration,
    ) {
        let mut registration = *self
            .tables
            .fdt
            .get(address(ip, port))
            .expect("foreign device entry exists");
        registration.registered_at = self
            .clock
            .instant(Instant::now())
            .saturating_sub(bacnet_bip_core::Duration::saturating_from_core(elapsed));
        self.tables
            .fdt
            .restore(registration)
            .expect("replaces the entry");
    }

    /// Store `entry` as given, with no policy check, as the tests that
    /// once pushed onto the old table's `Vec` did.
    #[cfg(test)]
    pub(crate) fn insert_fdt_entry_for_test(&mut self, entry: FdtEntry) {
        let registration = core_bbmd::Registration::new(
            address(entry.ip, entry.port),
            entry.ttl,
            self.clock.instant(entry.registered_at),
        );
        self.tables.fdt.restore(registration).expect("FDT has room");
    }

    #[cfg(test)]
    pub(crate) fn fdt_len_for_test(&self) -> usize {
        self.tables.fdt.len()
    }
}

/// `rows` as this module's entries.
fn rows_of(rows: &[codec::BdtEntry]) -> Vec<BdtEntry> {
    rows.iter().copied().map(BdtEntry::from).collect()
}

#[cfg(test)]
mod activation_tests;

#[cfg(test)]
mod foreign_device_tests;

#[cfg(test)]
mod nat_tests;

#[cfg(test)]
mod run_tests;

#[cfg(test)]
mod serve_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod validation_tests;
